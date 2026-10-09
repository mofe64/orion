"""A failed home attempt must never displace a connection NM is still activating."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from orion_wifi_fallback import BUSY, recover


class Network:
    def __init__(self, states, home_result=1, hotspot_result=0):
        self.states = iter(states)
        self.home_result = home_result
        self.hotspot_result = hotspot_result
        self.activations = []

    def run(self, *arguments):
        if arguments[0] == '-g':
            state = next(self.states)
            return subprocess.CompletedProcess(arguments, 0, f'{state} (state)\n')
        self.activations.append(arguments[5])
        result = self.home_result if arguments[5] == 'home' else self.hotspot_result
        if isinstance(result, Exception):
            raise result
        return subprocess.CompletedProcess(arguments, result, '')

    def recover(self):
        return recover('wlan0', 'home', 'hotspot', self.run, lambda _: None)


class FallbackTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.attempt_file = Path(temporary.name) / 'home-attempt'

    def test_delayed_home_failure_reaches_hotspot_on_next_idle_run(self):
        first = Network([30, 50], subprocess.TimeoutExpired('nmcli', 21))
        recover('wlan0', 'home', 'hotspot', first.run, lambda _: None, self.attempt_file)
        self.assertEqual(first.activations, ['home'])
        self.assertTrue(self.attempt_file.exists())
        busy = Network([50])
        recover('wlan0', 'home', 'hotspot', busy.run, lambda _: None, self.attempt_file)
        self.assertEqual(busy.activations, [])
        failed = Network([30, 30])
        recover('wlan0', 'home', 'hotspot', failed.run, lambda _: None, self.attempt_file)
        self.assertEqual(failed.activations, ['hotspot'])
        self.assertFalse(self.attempt_file.exists())

    def test_background_connection_success_clears_pending_attempt(self):
        self.attempt_file.write_text('wlan0:home\n')
        network = Network([100])
        recover('wlan0', 'home', 'hotspot', network.run, lambda _: None, self.attempt_file)
        self.assertFalse(self.attempt_file.exists())
        self.assertEqual(network.activations, [])

    def test_changed_home_profile_does_not_reuse_old_pending_attempt(self):
        self.attempt_file.write_text('wlan0:previous-home\n')
        network = Network([30], home_result=0)
        recover('wlan0', 'home', 'hotspot', network.run, lambda _: None, self.attempt_file)
        self.assertEqual(network.activations, ['home'])
        self.assertFalse(self.attempt_file.exists())

    def test_hotspot_failure_retries_hotspot_without_restarting_home(self):
        self.attempt_file.write_text('wlan0:home\n')
        network = Network([30, 30], hotspot_result=1)
        with self.assertRaisesRegex(RuntimeError, 'Could not activate'):
            recover('wlan0', 'home', 'hotspot', network.run, lambda _: None, self.attempt_file)
        self.assertEqual(network.activations, ['hotspot'])
        self.assertTrue(self.attempt_file.exists())

    def test_connected_home_or_hotspot_is_never_replaced(self):
        network = Network([100])
        self.assertEqual(network.recover(), 0)
        self.assertEqual(network.activations, [])

    def test_all_busy_states_are_left_alone(self):
        for state in BUSY:
            with self.subTest(state=state):
                network = Network([state])
                self.assertEqual(network.recover(), 0)
                self.assertEqual(network.activations, [])

    def test_home_success_does_not_start_hotspot(self):
        network = Network([30], home_result=0)
        self.assertEqual(network.recover(), 0)
        self.assertEqual(network.activations, ['home'])

    def test_idle_or_failed_after_home_failure_starts_hotspot(self):
        for initial in (30, 120):
            for after in (30, 120):
                with self.subTest(initial=initial, after=after):
                    network = Network([initial, after])
                    self.assertEqual(network.recover(), 0)
                    self.assertEqual(network.activations, ['home', 'hotspot'])

    def test_connection_started_during_home_attempt_is_not_replaced(self):
        for state in BUSY | {100}:
            with self.subTest(state=state):
                network = Network([30, state])
                self.assertEqual(network.recover(), 0)
                self.assertEqual(network.activations, ['home'])

    def test_timed_out_client_does_not_interrupt_background_activation(self):
        network = Network([30, 70], subprocess.TimeoutExpired('nmcli', 21))
        self.assertEqual(network.recover(), 0)
        self.assertEqual(network.activations, ['home'])

    def test_unavailable_unmanaged_and_unknown_states_do_not_activate(self):
        for state in (0, 10, 20, 999):
            with self.subTest(state=state):
                network = Network([state])
                with self.assertRaisesRegex(RuntimeError, 'leaving it alone'):
                    network.recover()
                self.assertEqual(network.activations, [])

    def test_unknown_state_after_home_attempt_does_not_start_hotspot(self):
        network = Network([30, 20])
        with self.assertRaisesRegex(RuntimeError, 'leaving it alone'):
            network.recover()
        self.assertEqual(network.activations, ['home'])

    def test_state_query_failure_never_switches_connections(self):
        def run(*arguments):
            self.assertEqual(arguments[0], '-g')
            return subprocess.CompletedProcess(arguments, 1, '')
        with self.assertRaisesRegex(RuntimeError, 'Cannot read'):
            recover('wlan0', 'home', 'hotspot', run)

    def test_hotspot_failure_is_reported(self):
        network = Network([30, 30], hotspot_result=1)
        with self.assertRaisesRegex(RuntimeError, 'Could not activate'):
            network.recover()


if __name__ == '__main__':
    unittest.main()
