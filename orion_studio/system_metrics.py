"""Read Linux system counters and retain a bounded, memory-only history."""
from collections import deque
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import threading
import time

UNITS = ('oriond', 'orion-listener', 'orion-voice-stack', 'orion-studio-gateway', 'orion-vision')
FLAGS = ('under_voltage', 'frequency_capped', 'throttled', 'soft_temperature_limit')


def read(path):
    try:
        return Path(path).read_text()
    except (OSError, UnicodeError):
        return ''


def numbers(text):
    result = {}
    for line in text.splitlines():
        fields = line.replace(':', '').split()
        if len(fields) >= 2:
            try:
                result[fields[0]] = int(fields[1])
            except ValueError:
                pass
    return result


def cpu_counters(text):
    result = {}
    for line in text.splitlines():
        fields = line.split()
        if fields and (fields[0] == 'cpu' or fields[0][3:].isdigit()):
            values = [int(value) for value in fields[1:9]]
            if len(values) >= 4:
                result[fields[0]] = (sum(values), values[3] + (values[4] if len(values) > 4 else 0))
    return result


def cpu_percent(previous, current):
    if previous is None or current[0] <= previous[0]:
        return None
    total = current[0] - previous[0]
    return round(max(0, min(100, 100 * (total - current[1] + previous[1]) / total)), 2)


def throttling(value):
    try:
        bits = int(value.strip().split('=')[-1], 16)
    except ValueError:
        return None
    return {'raw': bits, 'now': {name: bool(bits & (1 << i)) for i, name in enumerate(FLAGS)},
            'since_boot': {name: bool(bits & (1 << (16 + i))) for i, name in enumerate(FLAGS)}}


def process_stat(text, page_size):
    # The command can contain spaces and parentheses; fields start after its final parenthesis.
    begin, end = text.index('('), text.rindex(')')
    fields = text[end + 2:].split()
    return {'pid': int(text[:begin]), 'name': text[begin + 1:end], 'ticks': int(fields[11]) + int(fields[12]),
            'start': int(fields[19]), 'rss_bytes': max(0, int(fields[21])) * page_size}


class SystemdProperties:
    """Read unit properties through libsystemd without starting a command."""
    def __init__(self):
        self.lib = None
        self.bus = ctypes.c_void_p()
        try:
            self.lib = ctypes.CDLL(ctypes.util.find_library('systemd') or 'libsystemd.so.0')
            self.lib.sd_bus_open_system.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
            self.lib.sd_bus_get_property_string.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p,
                ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
            self.lib.sd_bus_get_property_trivial.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p,
                ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p, ctypes.c_char, ctypes.c_void_p]
            self.lib.sd_bus_unref.argtypes = [ctypes.c_void_p]
            self.lib.sd_bus_set_method_call_timeout.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
            self.libc = ctypes.CDLL(None)
            self.libc.free.argtypes = [ctypes.c_void_p]
        except (OSError, AttributeError):
            self.lib = None

    def close(self):
        if self.lib and self.bus.value:
            self.lib.sd_bus_unref(self.bus)
            self.bus = ctypes.c_void_p()

    def __call__(self, unit):
        if not self.lib:
            return {}
        if not self.bus.value and self.lib.sd_bus_open_system(ctypes.byref(self.bus)) < 0:
            return {}
        self.lib.sd_bus_set_method_call_timeout(self.bus, 100_000)
        escaped = ''.join(char if char.isascii() and char.isalnum() else f'_{ord(char):02x}' for char in unit)
        path = ('/org/freedesktop/systemd1/unit/' + escaped).encode()
        result = {}
        for key, interface, kind in (('ActiveState', 'Unit', 's'), ('ControlGroup', 'Service', 's'), ('NRestarts', 'Service', 'u')):
            args = (self.bus, b'org.freedesktop.systemd1', path,
                    ('org.freedesktop.systemd1.' + interface).encode(), key.encode(), None)
            if kind == 's':
                value = ctypes.c_void_p()
                status = self.lib.sd_bus_get_property_string(*args, ctypes.byref(value))
                if status >= 0:
                    result[key] = ctypes.string_at(value).decode()
                    self.libc.free(value)
            else:
                value = ctypes.c_uint32()
                status = self.lib.sd_bus_get_property_trivial(*args, b'u', ctypes.byref(value))
                if status >= 0:
                    result[key] = value.value
        return result


class SystemMetrics:
    def __init__(self, proc=Path('/proc'), sys=Path('/sys'), data=None, unit_properties=None, clock=time.monotonic):
        self.proc, self.sys = Path(proc), Path(sys)
        self.data = data or Path.home() / '.local/share/orion'
        self.properties = unit_properties or SystemdProperties()
        self.clock = clock
        self.previous_cpu, self.previous_processes, self.previous_units = {}, {}, {}
        self.previous_time = None
        self.throttled, self.throttle_time = None, float('-inf')
        self.history = {name: deque(maxlen=451) for name in ('cpu', 'temperature', 'ram', 'disk_root', 'disk_data')}
        self.latest = None
        self.lock = threading.Lock()
        self.stopped = threading.Event()
        self.thread = None
        self.hz, self.page_size = os.sysconf('SC_CLK_TCK'), os.sysconf('SC_PAGE_SIZE')

    def start(self):
        if self.thread is None:
            self.thread = threading.Thread(target=self._run, name='system-metrics', daemon=True)
            self.thread.start()

    def close(self):
        self.stopped.set()
        if self.thread:
            self.thread.join(timeout=3)

    def _run(self):
        try:
            while not self.stopped.is_set():
                started = self.clock()
                try:
                    self.sample()
                except (OSError, ValueError):
                    pass
                self.stopped.wait(max(0.1, 2 - (self.clock() - started)))
        finally:
            if hasattr(self.properties, 'close'):
                self.properties.close()

    def snapshot(self):
        with self.lock:
            return {'api_version': 2, 'latest': self.latest,
                    'history': {name: list(points) for name, points in self.history.items()}}

    @staticmethod
    def disk(path):
        try:
            # Resolve a not-yet-created data directory to its containing filesystem.
            existing = Path(path)
            while not existing.exists() and existing != existing.parent:
                existing = existing.parent
            total, used, free = shutil.disk_usage(existing)
            return {'path': str(path), 'total_bytes': total, 'used_bytes': used, 'available_bytes': free,
                    'percent': round(100 * used / total, 2) if total else None}
        except OSError:
            return {'path': str(path), 'total_bytes': None, 'used_bytes': None, 'available_bytes': None, 'percent': None}

    def sample(self):
        now, stamp = self.clock(), time.time()
        elapsed = now - self.previous_time if self.previous_time is not None else 0
        counters = cpu_counters(read(self.proc / 'stat'))
        cpu = {key: cpu_percent(self.previous_cpu.get(key), value) for key, value in counters.items()}
        memory = numbers(read(self.proc / 'meminfo'))
        def mem(key):
            value = memory.get(key)
            return value * 1024 if value is not None else None
        total, available = mem('MemTotal'), mem('MemAvailable')
        used = total - available if total is not None and available is not None else None
        ram = {'total_bytes': total, 'available_bytes': available, 'used_bytes': used,
               'percent': round(used * 100 / total, 2) if used is not None and total else None,
               'swap_total_bytes': mem('SwapTotal'), 'swap_free_bytes': mem('SwapFree')}
        ram['swap_used_bytes'] = ram['swap_total_bytes'] - ram['swap_free_bytes'] if ram['swap_total_bytes'] is not None and ram['swap_free_bytes'] is not None else None
        try:
            temperature = int(read(self.sys / 'class/thermal/thermal_zone0/temp')) / 1000
        except ValueError:
            temperature = None
        frequencies = []
        for path in (self.sys / 'devices/system/cpu/cpufreq').glob('policy*/scaling_cur_freq'):
            try:
                frequencies.append(int(read(path)) / 1000)
            except ValueError:
                pass
        if now - self.throttle_time >= 10:
            self.throttle_time = now
            try:
                result = subprocess.run(['vcgencmd', 'get_throttled'], capture_output=True, text=True, timeout=0.5, check=True)
                self.throttled = throttling(result.stdout)
            except (OSError, subprocess.SubprocessError):
                self.throttled = None
        try:
            load = [float(value) for value in read(self.proc / 'loadavg').split()[:3]] or None
        except ValueError:
            load = None
        services, unit_usage = [], {}
        for name in UNITS:
            props = self.properties(name + '.service')
            group = props.get('ControlGroup')
            path = self.sys / 'fs/cgroup' / group.lstrip('/') if group else None
            services.append(self._service(name, props.get('ActiveState'), props.get('NRestarts'), path, elapsed, unit_usage))
        metadata = self.data / 'home-assistant-install.json'
        try:
            container = json.loads(read(metadata))
            identifier = container['container_id']
            if len(identifier) == 64 and all(char in '0123456789abcdef' for char in identifier):
                paths = [self.sys / f'fs/cgroup/system.slice/docker-{identifier}.scope', self.sys / f'fs/cgroup/docker/{identifier}']
                path = next((candidate for candidate in paths if candidate.exists()), paths[0])
                services.append(self._service('orion-home-assistant', 'active' if path.exists() else 'inactive', None, path, elapsed, unit_usage))
        except (ValueError, KeyError, TypeError):
            pass
        processes, process_usage = [], {}
        for path in self.proc.glob('[0-9]*/stat'):
            try:
                process = process_stat(read(path), self.page_size)
                key = (process['pid'], process.pop('start'))
                ticks = process.pop('ticks')
                previous = self.previous_processes.get(key)
                process['cpu_percent'] = round(max(0, (ticks - previous) / self.hz / elapsed * 100), 2) if previous is not None and elapsed > 0 else None
                process_usage[key] = ticks
                processes.append(process)
            except (ValueError, IndexError, OSError):
                continue
        disks = {'root': self.disk('/'), 'data': self.disk(self.data)}
        latest = {'timestamp': stamp, 'cpu': {'percent': cpu.get('cpu'), 'cores': {key: value for key, value in cpu.items() if key != 'cpu'},
            'load_average': load, 'frequency_mhz': sum(frequencies) / len(frequencies) if frequencies else None,
            'temperature_c': temperature, 'throttling': self.throttled}, 'ram': ram, 'disk': disks,
            'services': services, 'processes': {'cpu': sorted(processes, key=lambda p: p['cpu_percent'] or 0, reverse=True)[:10],
            'rss': sorted(processes, key=lambda p: p['rss_bytes'], reverse=True)[:10]}, 'hailo_temperature_c': None}
        values = {'cpu': cpu.get('cpu'), 'temperature': temperature, 'ram': ram['percent'],
                  'disk_root': disks['root']['percent'], 'disk_data': disks['data']['percent']}
        with self.lock:
            for name, points in self.history.items():
                points.append([stamp, values[name]])
                while points and stamp - points[0][0] > 900:
                    points.popleft()
            self.latest = latest
        self.previous_cpu, self.previous_processes, self.previous_units = counters, process_usage, unit_usage
        self.previous_time = now
        return latest

    def _service(self, name, state, restarts, path, elapsed, usage):
        stats = numbers(read(path / 'cpu.stat')) if path else {}
        current, previous = stats.get('usage_usec'), self.previous_units.get(name)
        usage[name] = current
        percent = round(max(0, (current - previous) / elapsed / 10000), 2) if current is not None and previous is not None and current >= previous and elapsed > 0 else None
        try:
            memory = int(read(path / 'memory.current')) if path else None
        except ValueError:
            memory = None
        return {'name': name, 'active_state': state, 'restarts': restarts, 'cpu_percent': percent,
                'nr_throttled': stats.get('nr_throttled'), 'memory_bytes': memory}
