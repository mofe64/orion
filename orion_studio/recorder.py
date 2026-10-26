"""Orion's flight recorder: readings over time, incidents and reply speed.

Every second the recorder reads oriond's status and the system readings. Every
10 seconds it keeps one row with the worst values of those seconds, so a short
current spike or voltage drop is not lost. Rows, incidents and reply times stay
for 7 days in ~/.local/share/orion/recorder. Files are written once a minute to
spare the SD card; incidents are written at once.
"""
from collections import deque
import datetime as dt
import json
import os
from pathlib import Path
import threading
import time

KEEP_DAYS = 7
ROW_SECONDS = 10
FLUSH_SECONDS = 60
# STS3215 status register bits.
SERVO_FAULTS = ((1, 'voltage'), (2, 'position sensor'), (4, 'temperature'), (8, 'current'), (16, 'angle'), (32, 'overload'))
SERVO_HOT_C, SERVO_COOL_C = 65.0, 60.0
SERVO_LOW_V, SERVO_OK_V = 5.5, 5.8
CPU_HOT_C, CPU_COOL_C = 80.0, 75.0
LATE_TICKS = 25
LATE_REPEAT_SECONDS = 300
UNREACHABLE_SAMPLES = 3
PARTS = {'oriond': 'Movement and lights', 'orion-listener': 'Listening', 'orion-voice-stack': 'Speech and replies',
         'orion-studio-gateway': 'Studio connection', 'orion-vision': 'Camera'}


def day_of(stamp):
    return dt.datetime.fromtimestamp(stamp, dt.timezone.utc).strftime('%Y-%m-%d')


def servo_faults(code):
    names = [name for bit, name in SERVO_FAULTS if code & bit]
    return ', '.join(names) or f'code {code}'


def history_directory():
    configured = os.environ.get('ORION_VOICE_HISTORY_DIR')
    return Path(configured) if configured else Path.home() / '.local/share/orion/voice-stack/history'


def turn_latency(turn):
    """Reply speed of one voice turn from its saved events, or None."""
    heard = thought = voice = None
    for entry in turn.get('events', []):
        event, at = entry.get('event') or {}, entry.get('at')
        kind = event.get('type')
        if kind == 'transcript.final' and heard is None:
            heard = (at, event.get('durationMs'))
        elif kind == 'agent.response' and thought is None:
            thought = event.get('durationMs')
        elif kind == 'speech.started' and voice is None:
            voice = at
    if not heard or not voice:
        return None
    try:
        start = dt.datetime.fromisoformat(heard[0].replace('Z', '+00:00'))
        first = (dt.datetime.fromisoformat(voice.replace('Z', '+00:00')) - start).total_seconds() * 1000
    except (AttributeError, ValueError):
        return None
    listen = heard[1] if isinstance(heard[1], (int, float)) else None
    return {'t': round(start.timestamp(), 3), 'session': turn.get('sessionId'),
            'listen_ms': round(listen) if listen is not None else None,
            'think_ms': round(thought) if isinstance(thought, (int, float)) else None,
            'first_voice_ms': round(first),
            'total_ms': round(first + (listen or 0))}


def merge_rows(rows):
    """One row with the worst values of several rows."""
    def worst(values, pick=max):
        values = [value for value in values if isinstance(value, (int, float))]
        return pick(values) if values else None
    joints = {}
    for name in sorted({name for row in rows for name in row.get('joints', {})}):
        items = [row['joints'][name] for row in rows if name in row.get('joints', {})]
        status = 0
        for item in items:
            status |= int(item.get('status') or 0)
        joints[name] = {'temp': worst([item.get('temp') for item in items]),
                        'current': worst([item.get('current') for item in items]),
                        'volt_min': worst([item.get('volt_min') for item in items], min), 'status': status}
    loops = [row.get('loop') or {} for row in rows]
    return {'t': rows[-1]['t'], 'reachable': all(row.get('reachable') for row in rows), 'joints': joints,
            'loop': {key: worst([loop.get(key) for loop in loops]) for key in ('work_ms', 'delay_ms', 'late')},
            **{key: worst([row.get(key) for row in rows]) for key in ('bus_errors', 'cpu_temp', 'cpu', 'ram')}}


class Recorder:
    def __init__(self, request, metrics, data=None, history=None, clock=time.time, monotonic=time.monotonic):
        self.request = request
        self.metrics = metrics
        self.folder = Path(data or Path.home() / '.local/share/orion') / 'recorder'
        self.history = Path(history) if history else history_directory()
        self.clock, self.monotonic = clock, monotonic
        self.window = []
        self.window_start = None
        self.rows = []
        self.latencies = []
        self.last_flush = None
        self.last_scan = 0.0
        self.seen_sessions = set()
        self.incidents = deque(maxlen=500)
        self.flags = {}
        self.counters = {}
        self.unreachable = 0
        self.last_late = float('-inf')
        self.lock = threading.Lock()
        self.stopped = threading.Event()
        self.thread = None
        self._load()

    def _load(self):
        """Recent incidents and reply times survive a gateway restart."""
        for name, target in (('incidents', self.incidents), ('latency', None)):
            for path in sorted(self.folder.glob(f'{name}-*.jsonl'))[-KEEP_DAYS:]:
                for line in self._lines(path):
                    if target is not None:
                        target.append(line)
                    elif line.get('session'):
                        self.seen_sessions.add(line['session'])

    @staticmethod
    def _lines(path):
        try:
            text = path.read_text()
        except (OSError, UnicodeError):
            return []
        values = []
        for line in text.splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if isinstance(value, dict):
                values.append(value)
        return values

    def start(self):
        if self.thread is None:
            self.thread = threading.Thread(target=self._run, name='recorder', daemon=True)
            self.thread.start()

    def close(self):
        self.stopped.set()
        if self.thread:
            self.thread.join(timeout=3)
        self.flush()

    def _run(self):
        while not self.stopped.is_set():
            started = self.monotonic()
            try:
                self.tick()
            except (OSError, ValueError, TypeError, KeyError) as error:
                print(json.dumps({'event': 'gateway.recorder_failed', 'reason': str(error)}), flush=True)
            self.stopped.wait(max(0.1, 1 - (self.monotonic() - started)))

    # Sampling -----------------------------------------------------------

    def sample(self):
        now = self.clock()
        system = getattr(self.metrics, 'latest', None) or {}
        cpu = system.get('cpu') or {}
        value = {'t': now, 'cpu_temp': cpu.get('temperature_c'), 'cpu': cpu.get('percent'),
                 'ram': (system.get('ram') or {}).get('percent'),
                 'disk': ((system.get('disk') or {}).get('data') or {}).get('percent'),
                 'throttling': (cpu.get('throttling') or {}).get('now') or {},
                 'services': {item['name']: item for item in system.get('services') or [] if isinstance(item, dict)}}
        try:
            status = self.request('status')
            character = self.request('character status')
        except Exception as error:  # Any failure means oriond did not answer.
            return {**value, 'reachable': False, 'error': str(error)[:200]}
        value.update({'reachable': True, 'joints': {joint['name']: joint for joint in status.get('joints') or []
                                                    if isinstance(joint, dict) and joint.get('name')},
                      'loop': status.get('loop') or {}, 'bus_fault': status.get('bus_fault'),
                      'bus_errors': status.get('bus_error_count'), 'mode': status.get('mode'),
                      'rest': (character.get('rest') or {}).get('state'),
                      'rest_error': (character.get('rest') or {}).get('error'),
                      'character': (character.get('character') or {}).get('state')})
        return value

    def tick(self):
        sample = self.sample()
        now = sample['t']
        self.check(sample)
        with self.lock:
            if self.window_start is None:
                self.window_start = now
            self.window.append(sample)
            if now - self.window_start >= ROW_SECONDS:
                self.rows.append(self.summarise(self.window))
                self.window, self.window_start = [], None
        tick = self.monotonic()
        if self.last_flush is None:
            self.last_flush = tick
        if tick - self.last_flush >= FLUSH_SECONDS:
            self.last_flush = tick
            self.scan_latency()
            self.flush()
            self.prune(now)

    @staticmethod
    def summarise(samples):
        def worst(values, pick=max):
            values = [value for value in values if isinstance(value, (int, float))]
            return round(pick(values), 2) if values else None
        reached = [sample for sample in samples if sample.get('reachable')]
        joints = {}
        for name in sorted({name for sample in reached for name in sample['joints']}):
            readings = [sample['joints'][name] for sample in reached if name in sample['joints']]
            status = 0
            for reading in readings:
                status |= int(reading.get('status') or 0)
            joints[name] = {'temp': worst([r.get('temperature_c') for r in readings]),
                            'current': worst([abs(r['current_ma']) for r in readings if isinstance(r.get('current_ma'), (int, float))]),
                            'volt_min': worst([r.get('voltage_v') for r in readings if (r.get('voltage_v') or 0) > 0], min),
                            'status': status}
        loops = [sample['loop'] for sample in reached]
        return {'t': round(samples[-1]['t'], 3), 'reachable': len(reached) == len(samples),
                'joints': joints,
                'loop': {'work_ms': worst([loop.get('worst_work_ms') for loop in loops]),
                         'delay_ms': worst([loop.get('worst_delay_ms') for loop in loops]),
                         'late': worst([loop.get('late_ticks') for loop in loops])},
                'bus_errors': worst([sample.get('bus_errors') for sample in reached]),
                'cpu_temp': worst([sample.get('cpu_temp') for sample in samples]),
                'cpu': worst([sample.get('cpu') for sample in samples]),
                'ram': worst([sample.get('ram') for sample in samples])}

    # Incidents ----------------------------------------------------------

    def incident(self, kind, severity, message, now, detail=None):
        value = {'t': round(now, 3), 'kind': kind, 'severity': severity, 'message': message}
        if detail:
            value['detail'] = str(detail)[:300]
        with self.lock:
            self.incidents.append(value)
        self._append(f'incidents-{day_of(now)}.jsonl', [value])

    def _edge(self, key, active, now, kind, start, end=None, severity='problem', detail=None):
        """Records an incident when a condition starts and, optionally, when it ends."""
        was = self.flags.get(key, False)
        if active and not was:
            self.incident(kind, severity, start, now, detail)
        elif was and not active and end:
            self.incident(kind, 'recovered', end, now)
        self.flags[key] = active

    def _grew(self, key, value):
        previous = self.counters.get(key)
        self.counters[key] = value
        return isinstance(value, (int, float)) and isinstance(previous, (int, float)) and value > previous

    def check(self, sample):
        now = sample['t']
        self.unreachable = 0 if sample['reachable'] else self.unreachable + 1
        self._edge('unreachable', self.unreachable >= UNREACHABLE_SAMPLES, now, 'runtime',
                   'Orion’s movement service stopped answering.', 'Orion’s movement service answers again.',
                   detail=sample.get('error'))
        throttling = sample['throttling']
        self._edge('under_voltage', bool(throttling.get('under_voltage')), now, 'power',
                   'The Pi is not getting enough power.', 'The Pi’s power is back to normal.')
        self._edge('throttled', any(throttling.get(name) for name in ('throttled', 'frequency_capped', 'soft_temperature_limit')),
                   now, 'heat', 'The Pi slowed down to protect itself.', 'The Pi runs at full speed again.', 'warning')
        cpu_temp = sample.get('cpu_temp')
        if isinstance(cpu_temp, (int, float)):
            hot = cpu_temp >= CPU_HOT_C or (self.flags.get('cpu_hot') and cpu_temp >= CPU_COOL_C)
            self._edge('cpu_hot', hot, now, 'heat', f'The Pi is hot: {cpu_temp:.0f} °C.', 'The Pi cooled down.', 'warning')
        disk = sample.get('disk')
        if isinstance(disk, (int, float)):
            self._edge('disk', disk >= 90, now, 'storage', f'Orion’s storage is {disk:.0f}% full.', 'Orion’s storage has room again.', 'warning')
        for name, service in sample['services'].items():
            part = PARTS.get(name, name)
            if self._grew(f'restarts:{name}', service.get('restarts')):
                self.incident('service', 'warning', f'{part} restarted.', now)
            self._edge(f'failed:{name}', service.get('active_state') == 'failed', now, 'service',
                       f'{part} stopped with a problem.', f'{part} runs again.')
        if not sample['reachable']:
            return
        self._edge('bus_fault', bool(sample.get('bus_fault')), now, 'servos', 'Orion lost contact with its servos.',
                   'Orion has contact with its servos again.', detail=sample.get('bus_fault'))
        loop = sample['loop']
        if self._grew('bus_reconnects', loop.get('bus_reconnects')):
            self.incident('servos', 'warning', 'The servo board disconnected and came back.', now)
        late = loop.get('late_ticks') or 0
        if late >= LATE_TICKS and now - self.last_late >= LATE_REPEAT_SECONDS:
            self.last_late = now
            self.incident('timing', 'warning', f'Movement timing slipped: {late} late steps in 10 s.', now,
                          f'worst delay {loop.get("worst_delay_ms", 0):.0f} ms, worst step {loop.get("worst_work_ms", 0):.0f} ms')
        for name, joint in sample['joints'].items():
            code = int(joint.get('status') or 0)
            previous = self.counters.get(f'status:{name}', 0)
            self.counters[f'status:{name}'] = code
            if code and code != previous:
                self.incident('servos', 'problem', f'Servo {name} reported a fault: {servo_faults(code)}.', now)
            temp = joint.get('temperature_c')
            if isinstance(temp, (int, float)):
                hot = temp >= SERVO_HOT_C or (self.flags.get(f'hot:{name}') and temp >= SERVO_COOL_C)
                self._edge(f'hot:{name}', hot, now, 'servos', f'Servo {name} is hot: {temp:.0f} °C.', f'Servo {name} cooled down.', 'warning')
            volts = joint.get('voltage_v')
            if isinstance(volts, (int, float)) and volts > 0:
                low = volts < SERVO_LOW_V or (self.flags.get(f'volts:{name}') and volts < SERVO_OK_V)
                self._edge(f'volts:{name}', low, now, 'power', f'Servo power dropped to {volts:.1f} V at {name}.',
                           f'Servo power is back at {name}.', 'warning')
        self._edge('rest_fault', sample.get('rest') == 'fault', now, 'rest', 'Orion could not finish going to sleep or waking up.',
                   'Orion recovered from its sleep problem.', detail=sample.get('rest_error'))

    # Reply speed --------------------------------------------------------

    def scan_latency(self):
        root = self.history / 'conversations'
        since, newest = self.last_scan, self.last_scan
        found = []
        for path in root.glob('*/turns/*.json'):
            try:
                changed = path.stat().st_mtime
            except OSError:
                continue
            if changed <= since or path.stem in self.seen_sessions:
                continue
            newest = max(newest, changed)
            try:
                turn = json.loads(path.read_text())
            except (OSError, ValueError, UnicodeError):
                continue
            latency = turn_latency(turn) if isinstance(turn, dict) else None
            if latency:
                self.seen_sessions.add(path.stem)
                found.append(latency)
        self.last_scan = newest
        if found:
            found.sort(key=lambda item: item['t'])
            with self.lock:
                self.latencies.extend(found)

    # Files --------------------------------------------------------------

    def _append(self, name, values):
        try:
            self.folder.mkdir(parents=True, exist_ok=True, mode=0o700)
            with open(self.folder / name, 'a') as file:
                for value in values:
                    file.write(json.dumps(value, separators=(',', ':')) + '\n')
        except OSError as error:
            print(json.dumps({'event': 'gateway.recorder_write_failed', 'reason': str(error)}), flush=True)

    def flush(self):
        with self.lock:
            rows, self.rows = self.rows, []
            latencies, self.latencies = self.latencies, []
        for prefix, values in (('readings', rows), ('latency', latencies)):
            by_day = {}
            for value in values:
                by_day.setdefault(day_of(value['t']), []).append(value)
            for day, items in by_day.items():
                self._append(f'{prefix}-{day}.jsonl', items)

    def prune(self, now):
        oldest = day_of(now - KEEP_DAYS * 86400)
        for path in self.folder.glob('*-*.jsonl'):
            day = path.stem.split('-', 1)[-1]
            if day < oldest:
                try:
                    path.unlink()
                except OSError:
                    pass

    # Reading back -------------------------------------------------------

    def readings(self, hours, now=None):
        now = now or self.clock()
        start = now - hours * 3600
        rows = []
        days = sorted({day_of(start + offset * 86400) for offset in range(int(hours / 24) + 2)} | {day_of(now)})
        for day in days:
            rows.extend(row for row in self._lines(self.folder / f'readings-{day}.jsonl') if row.get('t', 0) >= start)
        with self.lock:
            rows.extend(row for row in self.rows if row['t'] >= start)
        rows.sort(key=lambda row: row['t'])
        # At most about 720 points: long ranges keep the worst of each slot.
        step = -(-len(rows) // 720)
        return [merge_rows(rows[index:index + step]) for index in range(0, len(rows), step)] if step > 1 else rows

    def snapshot(self, hours=1):
        hours = min(max(float(hours), 0.25), KEEP_DAYS * 24)
        now = self.clock()
        start = now - hours * 3600
        latency = []
        for path in sorted(self.folder.glob('latency-*.jsonl'))[-KEEP_DAYS:]:
            latency.extend(item for item in self._lines(path) if item.get('t', 0) >= start)
        with self.lock:
            latency.extend(item for item in self.latencies if item['t'] >= start)
            incidents = [item for item in self.incidents if item['t'] >= start]
        return {'api_version': 2, 'hours': hours, 'row_seconds': ROW_SECONDS, 'readings': self.readings(hours, now),
                'incidents': incidents[::-1][:200], 'latency': sorted(latency, key=lambda item: item['t'])[-200:]}
