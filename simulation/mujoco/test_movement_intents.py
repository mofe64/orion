"""Authored V2 movement intent, operating margins, turn bounds and preview checks."""
import copy
import json
from pathlib import Path
import sys

import mujoco
import numpy as np
import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import preview_movements as preview


@pytest.fixture(scope='module')
def catalogue(tmp_path_factory):
    output = tmp_path_factory.mktemp('movement-checks')
    return preview.compile_samples(output, ROOT / 'runtime/target/debug/oriond')


@pytest.fixture(scope='module')
def model():
    return preview.contact_model()


def test_intent_table_covers_exactly_the_live_v2_catalogue():
    table = preview.intent_table()
    motions = {yaml.safe_load(p.read_text())['motion']['name'] for p in (ROOT / 'motion/motions/v2').glob('*.yaml')}
    poses = yaml.safe_load((ROOT / 'motion/config/v2/poses.yaml').read_text())['poses']
    assert set(table['motions']) == motions
    assert set(table['poses']) == set(poses)
    for name, definition in poses.items():
        if name.startswith(('look_', 'attention_')):
            assert np.degrees(abs(definition['positions']['base_yaw_joint']) + abs(definition['positions']['head_roll_joint'])) <= 90
            other = poses[name.replace('_left', '_right') if '_left' in name else name.replace('_right', '_left')]['positions']
            for joint, value in definition['positions'].items():
                assert value == pytest.approx(-other[joint] if joint in ('base_yaw_joint', 'head_roll_joint') else other[joint])


def test_every_compiled_item_matches_its_fk_intent_size_speed_and_clearance(catalogue, model):
    failures = []
    for item in catalogue['items']:
        problems = preview.inspect(item, catalogue, model)['problems']
        if problems: failures.append((item['kind'], item['name'], item.get('start_pose'), problems))
    assert not failures, failures


def test_runtime_transitions_use_their_real_start_and_destination_anchors(catalogue):
    item = next(i for i in catalogue['items'] if i['name'] == 'hands_return' and i['kind'] == 'motion')
    poses = yaml.safe_load((ROOT / 'motion/config/v2/poses.yaml').read_text())['poses']
    assert item['planner'] == 'hands'
    assert item['samples'][0]['positions'] == poses['desk_work']['positions']
    assert item['expected_end'] == poses[item['anchor']]['positions']
    assert item['anchor_source'].startswith('runtime/src/expression/hands.rs')
    startup = next(i for i in catalogue['items'] if i['kind'] == 'startup')
    assert startup['start_pose'] == 'rest' and startup['anchor'] == 'home'
    assert startup['samples'][0]['positions'] == poses['rest']['positions']
    assert startup['expected_end'] == poses['home']['positions']


def test_checks_reject_reversed_intent_oversized_turn_and_excess_speed(catalogue, model):
    table = preview.intent_table()
    happy = table['motions']['speak_react_happy']
    assert preview.intent_problems(happy, -20, -12, 12, [-12, 0], 0, [{'time': 0}, {'time': 1}])
    nod = table['motions']['speak_react_agree']
    assert preview.intent_problems(nod, 20, 12, 12, [12, 0], 0, [{'time': 0}, {'time': 1}])
    item = copy.deepcopy(next(i for i in catalogue['items'] if i['name'] == 'look_at_left_expressive'))
    ceilings = table['motions'][item['name']]['speed_degrees_s']
    item['samples'][2]['velocities']['base_yaw_joint'] = np.radians(ceilings['base_yaw_joint'] + 1)
    item['samples'][2]['velocities']['head_pitch_joint'] = np.radians(ceilings['other'] + 1)
    problems = preview.inspect(item, catalogue, model)['problems']
    assert any('base_yaw_joint speed' in p for p in problems)
    assert any('head_pitch_joint speed' in p for p in problems)
    item['samples'][2]['positions'].update(base_yaw_joint=-1.05, head_roll_joint=-0.65)
    assert any('Combined yaw' in p for p in preview.inspect(item, catalogue, model)['problems'])


def test_joined_parts_are_ignored_but_nonadjacent_collisions_are_visible(model):
    data = mujoco.MjData(model)
    home = yaml.safe_load((ROOT / 'motion/config/v2/poses.yaml').read_text())['poses']['home']['positions']
    preview.fk(model, data, home)
    assert not list(preview.lamp_contacts(model, data))
    # An intentionally folded test configuration intersects separate lamp links.
    bad = {'base_yaw_joint': -0.3865516700572879, 'shoulder_pitch_joint': 1.7538627994127753,
           'elbow_pitch_joint': -0.00022332713283629773, 'head_roll_joint': -1.16168270798248,
           'head_pitch_joint': 0.33692335026857134}
    preview.fk(model, data, bad)
    contacts = list(preview.lamp_contacts(model, data))
    assert contacts
    for contact, (a, b) in contacts:
        assert a != b
        assert model.body_parentid[a] != b and model.body_parentid[b] != a


def test_camera_encloses_the_entire_movement_in_each_view(catalogue, model):
    for item in catalogue['items']:
        checks = preview.inspect(item, catalogue, model)
        bounds = np.array(checks['view_bounds'])
        centre, distance = preview.camera_framing(bounds, model.vis.global_.fovy)
        # An enclosing sphere fits both square views regardless of their rotation.
        radius = np.linalg.norm(bounds[1] - centre)
        assert distance * np.sin(np.radians(model.vis.global_.fovy / 2)) > radius
