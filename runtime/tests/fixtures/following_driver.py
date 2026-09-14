#!/usr/bin/env python3
"""Deterministic bridge fixture for daemon protocol tests, without physics.

This executable substitutes for --python and receives the real bridge's script
path and arguments. It exercises oriond's actual event loop, Unix dispatch,
movement completion, and light/audio ownership without claiming MuJoCo fidelity.
"""
import argparse
import json
import os
from pathlib import Path
import sys

parser = argparse.ArgumentParser()
parser.add_argument('bridge_script')
parser.add_argument('--scene')
parser.add_argument('--start-json')
args = parser.parse_args()
positions = json.loads(args.start_json)
names = ('base_yaw_joint', 'shoulder_pitch_joint', 'elbow_pitch_joint',
         'head_roll_joint', 'head_pitch_joint')
active = False
control_path = Path(os.environ['ORION_TEST_DRIVER_CONTROL'])
events_path = Path(os.environ['ORION_TEST_DRIVER_EVENTS'])


def emit(value):
    print(json.dumps(value), flush=True)


def state():
    return {'ok': True, 'joints': [dict(name=name, position_rad=positions[name],
        velocity_rad_s=0, current_ma=0, voltage_v=7.4, temperature_c=25, status=0) for name in names]}


emit({'ok': True, 'joint_limits': {name: [-3, 3] for name in names}})
for line in sys.stdin:
    request = json.loads(line)
    command = request['command']
    control = json.loads(control_path.read_text()) if control_path.exists() else {}
    if command in ('activate', 'deactivate'):
        with events_path.open('a') as events:
            events.write(json.dumps({'command': command}) + '\n')
    if command == 'shutdown': break
    if command == 'activate':
        active = True
        emit(state())
    elif command == 'deactivate':
        if control.get('release_fails'):
            emit({'ok': False, 'error': 'Injected torque release failure'})
        else:
            active = False
            emit({'ok': True})
    elif command == 'write':
        if not active:
            emit({'ok': False, 'error': 'Write while torque is off'})
        else:
            if not control.get('stall'):
                positions = request['positions']
            emit({'ok': True})
    elif command == 'read': emit(state())
    else: emit({'ok': False, 'error': 'Unsupported test bridge command'})
