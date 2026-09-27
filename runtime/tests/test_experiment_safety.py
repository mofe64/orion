"""Offline regression tests: no sockets or robot actions."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'runtime/scripts'))
from experiment_safety import TemperatureGuard, latch_hold, measured_rest_complete, before_shutdown, LATCH


def snapshot(seconds,temperature):
    return dict(sampled_at_unix_ns=round(seconds*1e9),joints=[dict(name='elbow_pitch_joint',temperature_c=temperature,velocity_rad_s=0,status=0)])

class GuardTests(unittest.TestCase):
    def test_recorded_fall_sequence_does_not_trip(self):
        rows=[json.loads(l)['status'] for l in (ROOT/'artifacts/gain-experiment-2026-09-27/shoulder_p24_i0/status-samples.jsonl').read_text().splitlines()]
        index=next(i for i,s in enumerate(rows) if next(j for j in s['joints'] if j['name']=='elbow_pitch_joint')['temperature_c']==65)
        recovered=json.loads((ROOT/'artifacts/gain-experiment-2026-09-27/shoulder_p24_i0/restored-rest-before-readback.json').read_text())
        temperatures=[next(j['temperature_c'] for j in s['joints'] if j['name']=='elbow_pitch_joint') for s in [rows[index-1],rows[index],recovered]]
        self.assertEqual(temperatures,[35,65,35])
        guard=TemperatureGuard();events=[]
        for s in [rows[index-1],rows[index],recovered]:self.assertEqual(guard.observe(s,events.append),[])
        self.assertTrue(any(e['kind']=='temperature_glitch' and e['temperature_c']==65 for e in events))

    def test_fast_spike_and_recovery(self):
        guard=TemperatureGuard();events=[]
        for t,value in [(0,35),(.12,65),(.24,35)]:self.assertEqual(guard.observe(snapshot(t,value),events.append),[])
        self.assertEqual(len(events),2)

    def test_three_plausible_high_readings_stop_and_hold(self):
        guard=TemperatureGuard();events=[];commands=[]
        with tempfile.TemporaryDirectory() as directory:
            for t,value in [(0,54),(.12,56),(.24,57),(.36,58)]:
                reasons=guard.observe(snapshot(t,value),events.append)
                if t<.36:self.assertEqual(reasons,[])
            self.assertEqual(len(reasons),1)
            latch_hold(lambda command:commands.append(command),directory,reasons[0],events.append)
            self.assertTrue((Path(directory)/LATCH).exists())
        self.assertEqual(commands,['stop','speech stop','stop'])
        self.assertNotIn('disable',commands)
        self.assertNotIn('character stop',commands)

    def test_persistent_high_plateau_is_not_hidden_forever(self):
        guard=TemperatureGuard();events=[]
        guard.observe(snapshot(0,35),events.append)
        for t in [.1,.2,.8]:self.assertEqual(guard.observe(snapshot(t,65),events.append),[])
        for t in [1.1,1.2]:self.assertEqual(guard.observe(snapshot(t,65),events.append),[])
        self.assertEqual(len(guard.observe(snapshot(1.3,65),events.append)),1)

    def test_duplicate_snapshot_does_not_confirm_heat(self):
        guard=TemperatureGuard();events=[]
        for _ in range(10):self.assertEqual(guard.observe(snapshot(0,60),events.append),[])
        self.assertEqual(guard.observe(snapshot(.1,60),events.append),[])
        self.assertEqual(len(guard.observe(snapshot(.2,60),events.append)),1)

    def test_recovery_resets_confirmation(self):
        guard=TemperatureGuard();events=[]
        for t,value in [(0,56),(.1,57),(.2,55),(.3,56),(.4,57)]:self.assertEqual(guard.observe(snapshot(t,value),events.append),[])
        self.assertEqual(len(guard.observe(snapshot(.5,58),events.append)),1)

    def test_rest_gate_rejects_active_cancelled_or_torque_on(self):
        state=snapshot(0,35);state.update(motion=None,torque_enabled=False,last_motion=dict(name='rest',state='completed'))
        self.assertTrue(measured_rest_complete(state))
        for change in [dict(motion=dict(run_id=1)),dict(torque_enabled=True),dict(last_motion=dict(name='rest',state='cancelled')),dict(last_motion=dict(name='goto',state='completed'))]:
            self.assertFalse(measured_rest_complete(dict(state,**change)))

    def test_service_stop_waits_for_completed_rest_without_disabling(self):
        spec=importlib.util.spec_from_file_location('candidate_stop',ROOT/'artifacts/gain-experiment-2026-09-27/candidate_torque_off.py')
        sys.path.insert(0,str(ROOT/'artifacts/gain-experiment-2026-09-27'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        moving=snapshot(0,35);moving.update(motion=dict(run_id=1),torque_enabled=True,last_motion=None)
        complete=dict(moving,motion=None,torque_enabled=False,last_motion=dict(name='rest',state='completed'))
        states=iter([moving,complete]);commands=[]
        def request(command):
            commands.append(command)
            return next(states) if command=='status' else dict(ok=True)
        with patch.object(module,'request',side_effect=request),patch.object(module.time,'sleep'):
            module.main()
        self.assertEqual(commands,['status','stop','speech stop','stop','status'])
        self.assertNotIn('disable',commands)

    def test_failed_trial_blocks_restoration_and_freezes_without_disable(self):
        commands=[]
        with tempfile.TemporaryDirectory() as directory,patch('experiment_safety.socket_request',side_effect=lambda c:commands.append(c)),patch('experiment_safety.freeze_daemon') as freeze:
            with self.assertRaises(RuntimeError):before_shutdown(directory,1)
            freeze.assert_called_once_with()
            self.assertTrue((Path(directory)/LATCH).exists())
        self.assertEqual(commands,['stop','speech stop','stop'])

if __name__=='__main__':unittest.main()
