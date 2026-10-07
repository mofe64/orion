#!/usr/bin/env python3
"""Estimate link inertias from CAD geometry; printed and supplier masses remain unmeasured."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import xml.etree.ElementTree as ET

import numpy as np

PROJECT = Path(__file__).resolve().parents[1]


def vertices(path):
    if path.suffix == '.obj':
        return np.array([[float(x) for x in line.split()[1:4]]
                         for line in path.read_text().splitlines() if line.startswith('v ')])
    data = path.read_bytes()
    count = struct.unpack_from('<I', data, 80)[0]
    if len(data) != 84 + 50 * count:
        raise ValueError(f'Expected binary STL: {path}')
    dtype = np.dtype([('normal', '<f4', (3,)), ('vertices', '<f4', (3, 3)), ('attribute', '<u2')])
    return np.frombuffer(data, dtype=dtype, count=count, offset=84)['vertices'].astype(float)


def solid_properties(triangles):
    """Integrate oriented tetrahedra, returning volume, centre and unit-density inertia."""
    # Multiple touching closed solids can share edges four times. Require
    # balanced oriented edges rather than incorrectly rejecting that assembly.
    _, index = np.unique(np.round(triangles.reshape(-1, 3), 8), axis=0, return_inverse=True)
    faces = index.reshape(-1, 3)
    edges = np.concatenate([faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]]])
    _, edge_ids = np.unique(np.sort(edges, axis=1), axis=0, return_inverse=True)
    orientation = np.bincount(edge_ids, weights=np.where(edges[:, 0] < edges[:, 1], 1, -1))
    if np.any(orientation):
        raise ValueError('Printed mesh has open or inconsistently oriented edges')
    origin = (triangles.min(axis=(0, 1)) + triangles.max(axis=(0, 1))) / 2
    shifted = triangles - origin
    a, b, c = shifted[:, 0], shifted[:, 1], shifted[:, 2]
    volumes = np.einsum('ij,ij->i', a, np.cross(b, c)) / 6
    volume = volumes.sum()
    if abs(volume) < 1e-15:
        raise ValueError('Printed mesh has zero enclosed volume')
    if volume < 0:
        volumes = -volumes
        volume = -volume
    sums = a + b + c
    centre = np.einsum('i,ij->j', volumes, sums) / (4 * volume)
    second = (np.einsum('i,ijk,ijl->kl', volumes, shifted, shifted) +
              np.einsum('i,ij,ik->jk', volumes, sums, sums)) / 20
    inertia = np.trace(second) * np.eye(3) - second
    inertia -= volume * (centre @ centre * np.eye(3) - np.outer(centre, centre))
    if np.min(np.linalg.eigvalsh(inertia)) <= 0:
        raise ValueError('Printed mesh has invalid inertia')
    return float(volume), centre + origin, inertia


def combine(parts):
    mass = sum(row['mass_kg'] for row in parts)
    centre = sum(row['mass_kg'] * np.array(row['centre_m']) for row in parts) / mass
    inertia = np.zeros((3, 3))
    for row in parts:
        offset = np.array(row['centre_m']) - centre
        inertia += np.array(row['inertia_kg_m2']) + row['mass_kg'] * (
            offset @ offset * np.eye(3) - np.outer(offset, offset))
    return dict(mass_kg=mass, centre_m=centre.tolist(), inertia_kg_m2=inertia.tolist())


def supplier_mass_range(filename):
    # These deliberately reproduce the unmeasured ranges in the CAD assessment;
    # supplier shape does not establish a material density or a measured mass.
    if 'STS3215' in filename:
        return [50, 75]
    if 'horn' in filename:
        return [4, 15]
    if 'Speaker' in filename:
        return [40, 90]
    if 'Camera' in filename:
        return [3, 10]
    return [5, 20]


def write_xml(tree, path):
    ET.indent(tree, space='  ')
    ET.ElementTree(tree).write(path, encoding='unicode', xml_declaration=True)


def update_model(model_root, urdf, inputs=None):
    inputs = inputs or {}
    density = float(inputs.get('density_g_cm3', 1.24)) * 1000  # kg/m3
    overrides = inputs.get('part_masses_g', {})
    if not np.isfinite(density) or density <= 0:
        raise ValueError('PLA density must be positive and finite')
    tree = ET.parse(model_root / 'robot.xml').getroot()
    mesh_files = {mesh.get('name'): model_root / mesh.get('file') for mesh in tree.findall('./asset/mesh')}
    unknown = set(overrides) - {path.name for path in mesh_files.values()}
    if unknown:
        raise ValueError(f'Mass overrides do not match packaged meshes: {sorted(unknown)}')
    rows, links = [], {}
    for body in tree.findall('.//worldbody//body'):
        parts = []
        for geom in body.findall('geom'):
            if geom.get('type') != 'mesh':
                continue
            path = mesh_files[geom.get('mesh')]
            points = vertices(path)
            offset = np.fromstring(geom.get('pos', '0 0 0'), sep=' ')
            row = dict(file=str(path.relative_to(model_root)), body=body.get('name'),
                       mesh_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
            if path.name.startswith('v21_'):
                volume, centre, inertia = solid_properties(points)
                mass = float(overrides[path.name]) / 1000 if path.name in overrides else volume * density
                inertia *= mass / volume
                row.update(volume_m3=volume, solid_pla_equivalent_g=volume * density * 1000,
                           mass_source='operator_mass_override' if path.name in overrides else 'solid_PLA_CAD_equivalent')
            else:
                points = points.reshape(-1, 3)
                size = points.max(axis=0) - points.min(axis=0)
                centre = (points.max(axis=0) + points.min(axis=0)) / 2
                interval = supplier_mass_range(path.name)
                mass = float(overrides[path.name]) / 1000 if path.name in overrides else sum(interval) / 2000
                inertia = np.diag(mass / 12 * (size @ size - size * size))
                row.update(mass_source='operator_mass_override' if path.name in overrides else 'assumed_supplier_range_midpoint',
                           assumed_mass_range_g=interval, shape_assumption='uniform bounding box')
            if not np.isfinite(mass) or mass <= 0:
                raise ValueError(f'Part mass must be positive and finite: {path.name}')
            row.update(mass_kg=mass, centre_m=(centre + offset).tolist(), inertia_kg_m2=inertia.tolist())
            rows.append(row); parts.append(row)
        for extra in inputs.get('extra_components', []):
            if extra['body'] != body.get('name'):
                continue
            mass = float(extra['mass_g']) / 1000
            size, centre = np.array(extra['size_m'], dtype=float), np.array(extra['centre_m'], dtype=float)
            if (not np.isfinite(mass) or mass <= 0 or size.shape != (3,) or centre.shape != (3,) or
                    not np.all(np.isfinite(size)) or not np.all(size > 0) or not np.all(np.isfinite(centre))):
                raise ValueError('Extra components need a positive mass, box size and finite body-local centre')
            row = dict(name=extra['name'], body=body.get('name'), mass_source='operator_extra_component',
                       mass_kg=mass, centre_m=centre.tolist(), shape_assumption='uniform bounding box',
                       inertia_kg_m2=np.diag(mass / 12 * (size @ size - size * size)).tolist())
            rows.append(row); parts.append(row)
        if parts:
            links[body.get('name')] = combine(parts)
            previous = body.find('inertial')
            if previous is not None:
                body.remove(previous)
            combined = links[body.get('name')]
            matrix = np.array(combined['inertia_kg_m2'])
            ET.SubElement(body, 'inertial', mass=str(combined['mass_kg']),
                          pos=' '.join(map(str, combined['centre_m'])),
                          fullinertia=' '.join(map(str, [matrix[0, 0], matrix[1, 1], matrix[2, 2], matrix[0, 1], matrix[0, 2], matrix[1, 2]])))
    missing_bodies = {row['body'] for row in inputs.get('extra_components', [])} - set(links)
    if missing_bodies:
        raise ValueError(f'Unknown extra-component bodies: {sorted(missing_bodies)}')
    report = dict(scope='unmeasured_mass_estimate', density_g_cm3=density / 1000,
                  note='CAD solid PLA equivalents are not sliced print weights. Overrides scale CAD mass distribution uniformly. Supplier interiors use uniform boxes.',
                  omitted_unless_added=['base yaw servo', 'Raspberry Pi and cooler', 'XVF3800 board', 'servo controller', 'fasteners', 'cables', 'ballast'],
                  contact_validation=False, dynamic_validation=False, parts=rows, links=links,
                  accounted_mass_kg=sum(row['mass_kg'] for row in rows))
    write_xml(tree, model_root / 'robot.xml')
    robot = ET.parse(urdf).getroot()
    for link in robot.findall('link'):
        if link.get('name') not in links:
            continue
        previous = link.find('inertial')
        if previous is not None:
            link.remove(previous)
        combined = links[link.get('name')]; matrix = np.array(combined['inertia_kg_m2'])
        inertial = ET.SubElement(link, 'inertial')
        ET.SubElement(inertial, 'origin', xyz=' '.join(map(str, combined['centre_m'])), rpy='0 0 0')
        ET.SubElement(inertial, 'mass', value=str(combined['mass_kg']))
        ET.SubElement(inertial, 'inertia', **{key: str(matrix[i, j]) for key, i, j in (
            ('ixx', 0, 0), ('iyy', 1, 1), ('izz', 2, 2), ('ixy', 0, 1), ('ixz', 0, 2), ('iyz', 1, 2))})
    write_xml(robot, urdf)
    report_path = model_root / 'mass-properties.json'
    report_path.write_text(json.dumps(report, indent=2) + '\n')
    provenance_path = model_root / 'provenance.json'
    provenance = json.loads(provenance_path.read_text())
    provenance.update(inertias='CAD-based unmeasured estimates',
                      mass_properties_sha256=hashlib.sha256(report_path.read_bytes()).hexdigest())
    provenance_path.write_text(json.dumps(provenance, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, help='Optional part masses and missing-component boxes, in documented units')
    args = parser.parse_args()
    report = update_model(PROJECT / 'simulation/mujoco/v2', PROJECT / 'description/urdf/orion-v2.urdf',
                          json.loads(args.inputs.read_text()) if args.inputs else None)
    print(f"Accounted estimate: {report['accounted_mass_kg']:.3f} kg; see mass-properties.json for assumptions and omissions.")


if __name__ == '__main__':
    main()
