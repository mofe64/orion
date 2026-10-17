import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
from http.client import HTTPConnection
import threading
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from system_metrics import SystemMetrics, cpu_counters, cpu_percent, process_stat, throttling
from gateway import GatewayHTTPServer, OrionGateway, make_handler


class MetricsTests(unittest.TestCase):
    def test_parsers(self):
        self.assertEqual(cpu_counters('cpu 10 0 20 70 10 0 0 0 99 0\nintr 3')['cpu'], (110, 80))
        self.assertEqual(cpu_percent((100, 60), (200, 110)), 50)
        self.assertIsNone(cpu_percent((200, 110), (100, 60)))
        flags = throttling('throttled=0xf000f')
        self.assertTrue(all(flags['now'].values()))
        self.assertTrue(all(flags['since_boot'].values()))
        self.assertIsNone(throttling('unavailable'))
        text = '42 (a worker (x)) S ' + ' '.join(['0'] * 10 + ['100', '50'] + ['0'] * 6 + ['1234', '2048', '7'])
        process = process_stat(text, 4096)
        self.assertEqual((process['name'], process['ticks'], process['start'], process['rss_bytes']), ('a worker (x)', 150, 1234, 28672))

    def test_fixture_sampling_and_bounded_history(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            def put(path, value):
                target = root / path; target.parent.mkdir(parents=True, exist_ok=True); target.write_text(value)
            put('proc/stat', 'cpu 10 0 20 70\ncpu0 10 0 20 70')
            put('proc/meminfo', 'MemTotal: 1000 kB\nMemAvailable: 600 kB\nSwapTotal: 100 kB\nSwapFree: 80 kB')
            put('proc/loadavg', '0.1 0.2 0.3 1/50 42')
            put('sys/class/thermal/thermal_zone0/temp', '76000')
            put('sys/devices/system/cpu/cpufreq/policy0/scaling_cur_freq', '1800000')
            put('sys/fs/cgroup/system.slice/oriond.service/cpu.stat', 'usage_usec 1000000\nnr_throttled 2')
            put('sys/fs/cgroup/system.slice/oriond.service/memory.current', '12345')
            clock = Mock(return_value=0.)
            metrics = SystemMetrics(root / 'proc', root / 'sys', root / 'data',
                unit_properties=lambda name: {'ControlGroup': '/system.slice/' + name, 'ActiveState': 'active', 'NRestarts': 3}, clock=clock)
            with patch('system_metrics.subprocess.run', return_value=Mock(stdout='throttled=0x0')) as run:
                first = metrics.sample()
                self.assertIsNone(first['cpu']['percent'])
                clock.return_value = 2
                put('proc/stat', 'cpu 50 0 30 120\ncpu0 50 0 30 120')
                put('sys/fs/cgroup/system.slice/oriond.service/cpu.stat', 'usage_usec 2000000\nnr_throttled 4')
                result = metrics.sample()
                self.assertEqual(result['cpu']['percent'], 50)
                self.assertEqual(result['cpu']['temperature_c'], 76)
                self.assertEqual(result['ram']['used_bytes'], 400 * 1024)
                self.assertEqual(result['services'][0]['cpu_percent'], 50)
                self.assertEqual(result['services'][0]['nr_throttled'], 4)
                self.assertEqual(result['services'][0]['restarts'], 3)
                self.assertEqual(result['services'][0]['memory_bytes'], 12345)
                self.assertEqual(run.call_count, 1)
                for tick in range(460):
                    clock.return_value = tick * 2 + 4
                    metrics.sample()
            snapshot = metrics.snapshot()
            self.assertLessEqual(len(snapshot['history']['cpu']), 451)
            json.dumps(snapshot, allow_nan=False)
            self.assertEqual(sorted(root.iterdir()), [root / 'proc', root / 'sys'])

    def test_authenticated_endpoint(self):
        gateway = OrionGateway(Mock())
        gateway.system_metrics = Mock(snapshot=lambda: {'api_version': 2, 'latest': None, 'history': {'cpu': []}})
        server = GatewayHTTPServer(('127.0.0.1', 0), make_handler(gateway, 'test-token', []))
        worker = threading.Thread(target=server.serve_forever); worker.start()
        try:
            for headers, expected in (({}, 401), ({'Authorization': 'Bearer test-token'}, 200)):
                connection = HTTPConnection(*server.server_address)
                connection.request('GET', '/api/v2/system/metrics', headers=headers)
                response = connection.getresponse(); body = json.loads(response.read()); connection.close()
                self.assertEqual(response.status, expected)
                if expected == 200:
                    self.assertEqual(set(body), {'api_version', 'latest', 'history'})
        finally:
            server.shutdown(); server.server_close(); worker.join()
