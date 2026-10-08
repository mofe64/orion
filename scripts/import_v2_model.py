#!/usr/bin/env python3
"""Package V2.1 CAD review meshes, MJCF and matching URDF for Orion preview."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import xml.etree.ElementTree as ET

PROJECT = Path(__file__).resolve().parents[1]
JOINT_MAP = {
    'v21_base_yaw': 'base_yaw_joint',
    'v21_shoulder_pitch': 'shoulder_pitch_joint',
    'v21_elbow_pitch': 'elbow_pitch_joint',
    'v21_wrist_pitch': 'head_pitch_joint',
    'v21_neck_swivel': 'head_roll_joint',
}

REVERSED_JOINTS = {'base_yaw_joint', 'head_roll_joint'}


def save_xml(tree, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.indent(tree, space='  ')
    ET.ElementTree(tree).write(path, encoding='unicode', xml_declaration=True)


def import_model(source, destination=PROJECT / 'simulation/mujoco/v2', urdf=PROJECT / 'description/urdf/orion-v2.urdf'):
    tree = ET.parse(source).getroot()
    provenance = {'source': 'orionV21CAD/simulation/references/v21/robot.xml',
                  'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                  'mechanical_revision': 'v2.1-r4-r2', 'scope': 'kinematic_preview',
                  'contacts_enabled': False, 'inertias': 'provisional', 'meshes': []}
    tree.set('model', 'Orion v2 / V2.1 CAD kinematic preview')
    meshes = {}
    for mesh in tree.findall('./asset/mesh'):
        asset = Path(mesh.attrib['file'])
        if not asset.is_absolute():
            asset = source.parent / asset
        target = destination / 'meshes' / asset.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(asset, target)
        mesh.set('file', 'meshes/' + asset.name)
        meshes[mesh.attrib['name']] = target
        provenance['meshes'].append({'file': 'meshes/' + asset.name,
                                     'sha256': hashlib.sha256(target.read_bytes()).hexdigest()})
    present = {joint.attrib['name'] for joint in tree.findall('.//joint') if 'name' in joint.attrib}
    if present != set(JOINT_MAP):
        raise ValueError('CAD model must contain exactly the five V2.1 joints')
    for joint in tree.findall('.//joint'):
        if 'name' in joint.attrib:
            joint.set('name', JOINT_MAP[joint.attrib['name']])
            # On the fitted lamp a negative base yaw or neck swivel turns
            # toward the lamp's own left (checked 2026-10-08 with look_left).
            # The CAD's +Z right-hand axis turns the other way, so reverse it.
            if joint.attrib['name'] in REVERSED_JOINTS:
                axis = [-float(value) for value in joint.attrib['axis'].split()]
                joint.set('axis', ' '.join(f'{value:g}' for value in axis))
    custom = ET.SubElement(tree, 'custom')
    ET.SubElement(custom, 'text', name='orion_hardware', data='v2')
    ET.SubElement(custom, 'text', name='orion_simulation_scope', data='kinematic_preview')
    # Actuators supply a complete mapping to tooling. The runtime bridge uses
    # ideal kinematic tracking for this model, so these gains are not physical.
    actuators = ET.SubElement(tree, 'actuator')
    for name in JOINT_MAP.values():
        ET.SubElement(actuators, 'position', name=name, joint=name, kp='50', kv='2')
    save_xml(tree, destination / 'robot.xml')
    scene = ET.Element('mujoco', model='Orion v2 preview scene')
    ET.SubElement(scene, 'include', file='robot.xml')
    save_xml(scene, destination / 'scene.xml')
    (destination / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')

    robot = ET.Element('robot', name='orion_v2')
    def link(body, parent=None):
        name = body.attrib['name']
        item = ET.SubElement(robot, 'link', name=name)
        for geom in body.findall('geom'):
            if geom.get('type') != 'mesh':
                continue
            visual = ET.SubElement(item, 'visual', name=geom.attrib['name'])
            ET.SubElement(visual, 'origin', xyz=geom.get('pos', '0 0 0'), rpy='0 0 0')
            geometry = ET.SubElement(visual, 'geometry')
            asset_path = os.path.relpath(meshes[geom.attrib['mesh']], urdf.parent)
            # Portable repo-relative resource, resolved from description/urdf.
            ET.SubElement(geometry, 'mesh', filename=asset_path)
        if parent is not None:
            mj_joint = body.find('joint')
            joint = ET.SubElement(robot, 'joint', name=mj_joint.attrib['name'] if mj_joint is not None else name + '_fixed', type='revolute' if mj_joint is not None else 'fixed')
            ET.SubElement(joint, 'parent', link=parent)
            ET.SubElement(joint, 'child', link=name)
            ET.SubElement(joint, 'origin', xyz=body.get('pos', '0 0 0'), rpy='0 0 0')
            if mj_joint is not None:
                ET.SubElement(joint, 'axis', xyz=mj_joint.attrib['axis'])
                lower, upper = mj_joint.attrib['range'].split()
                ET.SubElement(joint, 'limit', lower=lower, upper=upper, effort='0', velocity='0')
        for child in body.findall('body'):
            link(child, name)
    for body in tree.findall('./worldbody/body'):
        link(body)
    save_xml(robot, urdf)
    # MuJoCo's Python environment supplies NumPy. Keep geometry import and the
    # mass calculation reproducible together, with unmeasured scope explicit.
    from estimate_v2_inertias import update_model
    update_model(destination, urdf)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=PROJECT.parent / 'orionV21CAD/simulation/references/v21/robot.xml')
    args = parser.parse_args()
    import_model(args.source)


if __name__ == '__main__':
    main()
