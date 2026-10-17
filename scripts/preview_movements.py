#!/usr/bin/env python3
"""Compile the V2 catalog, inspect every 50 Hz sample, and render private review videos."""
import argparse
from collections import Counter
import html
import json
from pathlib import Path
import re
import shutil
import subprocess

import yaml

from movement_preview_runtime import build_adapter

import mujoco
import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
INTENTS = ROOT / 'motion/config/v2/movement-intents.yaml'
CONTACT_SCOPE = ('MuJoCo mesh convex hulls; joined parent links and welded parts are excluded. '
                 'Other lamp-body contacts are conservative collision candidates. '
                 'V2 follows joint targets kinematically; this check does not model servo loads or base tipping.')


def intent_table():
    return yaml.safe_load(INTENTS.read_text())


def motion_requests(table):
    return [{'name': name, **context, 'planner': spec.get('planner', 'authored'), 'source': spec['source']}
            for name, spec in table['motions'].items() for context in spec['contexts']]



def compile_samples(output, compiler):
    if compiler is None:
        subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'runtime/Cargo.toml'), '--bin', 'oriond'], check=True, cwd=ROOT)
        compiler = ROOT / 'runtime/target/debug/oriond'
    target = output / 'samples.json'
    result = subprocess.run([str(compiler), '--validate-catalog', '--catalog', str(ROOT), '--hardware', 'v2', '--output', str(target)], cwd=ROOT)
    if not target.is_file(): raise RuntimeError(f'Catalog compiler failed ({result.returncode})')
    document = json.loads(target.read_text())
    table = intent_table()
    adapter = build_adapter(ROOT)
    requests = output / 'requests.json'
    requests.write_text(json.dumps(motion_requests(table)))
    compiled = subprocess.check_output([str(adapter), str(ROOT), str(ROOT), str(requests)], cwd=ROOT)
    motions = json.loads(compiled)
    # Pose round trips, startup and scenes retain the daemon catalog exporter.
    # Motions use the command-time planner and the start/anchor pair in the intent table.
    document['items'] = [i for i in document['items'] if i['kind'] != 'motion'] + motions
    for item in document['items']:
        item.setdefault('start_pose', item['anchor'])
        if item['kind'] == 'startup': item['anchor'] = 'home'
    target.write_text(json.dumps(document))
    return document



def contact_model():
    spec = mujoco.MjSpec.from_file(str(ROOT / 'simulation/mujoco/v2/scene.xml'))
    # Compile after changing masks: MuJoCo also derives body collision masks.
    # Editing only model.geom_contype would leave those body masks disabled.
    for geom in spec.geoms:
        if geom.parent.name != 'world':
            geom.contype = 1
            geom.conaffinity = 1
    # The standard parent filter excludes only joined links.
    return spec.compile()


def fk(model, data, positions):
    for name, value in positions.items(): data.qpos[model.joint(name).qposadr[0]] = value
    mujoco.mj_forward(model, data)
    axis = data.site('light_frame').xmat.reshape(3, 3)[:, 2]
    return np.array([data.site('v21_face').xpos[2] * 1000,
                     np.degrees(np.arctan2(axis[2], np.linalg.norm(axis[:2])))])


def heading(model, data, positions):
    fk(model, data, positions)
    axis = data.site('light_frame').xmat.reshape(3, 3)[:, 2]
    return float(np.degrees(np.arctan2(-axis[0], -axis[1])))


def intent_problems(spec, height, gaze, movement, gaze_series, apex_time, samples):
    problems = []
    direction = spec['direction']
    if direction == 'up' and not (height > 1.0 or gaze > 1.0):
        problems.append(f'Intent requires a head lift or raised gaze; got {height:+.1f} mm, {gaze:+.1f} deg')
    if direction == 'down' and gaze > -1.0:
        problems.append(f'Intent requires a lowered gaze; got {gaze:+.1f} deg')
    if direction == 'nod':
        after = [v for v, sample in zip(gaze_series, samples) if sample['time'] > apex_time]
        if gaze > -8 or not after or max(after) < gaze + 8:
            problems.append('Intent requires a clear down-then-up nod')
    minimum, maximum = spec.get('movement_degrees', [0, None])
    if movement < minimum - 1e-6 or (maximum is not None and movement > maximum + 1e-6):
        problems.append(f'Main movement {movement:.1f} deg is outside {minimum}..{maximum or "unbounded"} deg')
    return problems


def lamp_contacts(model, data):
    for contact in data.contact:
        if contact.dist >= -1e-6: continue
        a, b = sorted(int(model.geom_bodyid[g]) for g in (contact.geom1, contact.geom2))
        if a == 0 or a == b or model.body_parentid[b] == a: continue
        yield contact, (a, b)


def inspect(item, document, model):
    if 'error' in item: return {'problems': [item['error']], 'contacts': [], 'clamped_samples': [], 'peak_speed': {}}
    table = intent_table()
    spec = table['motions'].get(item['name']) if item['kind'] == 'motion' else None
    data = mujoco.MjData(model)
    names = [joint['name'] for joint in document['joint_limits']]
    peaks = dict.fromkeys(names, 0.0)
    contacts = {}; outside = set(); boundary = Counter(); previous = None
    start = item['samples'][0]['positions']
    start_fk = fk(model, data, start)
    gaze_series = []
    bounds = np.array([[np.inf] * 3, [-np.inf] * 3])
    lamp = np.where(model.geom_bodyid > 0)[0]
    minimum_margin = np.inf
    for sample in item['samples']:
        positions = sample['positions']
        for limit in document['joint_limits']:
            name = limit['name']; value = positions[name]
            margin = min(value - limit['lower'], limit['upper'] - value)
            minimum_margin = min(minimum_margin, np.degrees(margin))
            if margin < -1e-9: outside.add(name)
            if margin < 1e-9: boundary[name] += 1
            if previous:
                dt = sample['time'] - previous['time']
                peaks[name] = max(peaks[name], abs(value - previous['positions'][name]) / dt)
            peaks[name] = max(peaks[name], abs(sample.get('velocities', {}).get(name, 0)))
        data.time = sample['time']
        gaze_series.append(float(fk(model, data, positions)[1] - start_fk[1]))
        radii = model.geom_rbound[lamp, None]
        bounds[0] = np.minimum(bounds[0], (data.geom_xpos[lamp] - radii).min(axis=0))
        bounds[1] = np.maximum(bounds[1], (data.geom_xpos[lamp] + radii).max(axis=0))
        for contact, bodies in lamp_contacts(model, data):
            pair = tuple(mujoco.mj_id2name(model, mujoco.mjtObj.mjOBJ_BODY, b) for b in bodies)
            entry = contacts.setdefault(pair, {'bodies': pair, 'first_time': sample['time'], 'depth_m': 0.0, 'samples': 0})
            entry['depth_m'] = max(entry['depth_m'], -float(contact.dist)); entry['samples'] += 1
        previous = sample
    for observation in item.get('hand_points', []):
        if observation['point'] is not None:
            bounds[0] = np.minimum(bounds[0], np.array(observation['point']) - 0.01)
            bounds[1] = np.maximum(bounds[1], np.array(observation['point']) + 0.01)
    final = item['samples'][-1]['positions']
    end_error = max(abs(final[name] - item['expected_end'][name]) for name in names)
    speed_limit = document['maximum_velocity_rad_s']
    problems = []
    if contacts: problems.append(f'{len(contacts)} non-adjacent lamp-body contact pairs')
    clamps = item['clamped_samples']
    if clamps: problems.append('Runtime clamped joints: ' + ', '.join(sorted(set(clamps))))
    if outside: problems.append('Samples outside limits: ' + ', '.join(sorted(outside)))
    if any(peak > speed_limit + 1e-6 for peak in peaks.values()): problems.append('Servo speed exceeded')
    if end_error > 1e-6: problems.append(f'End anchor error: {end_error:.6f} rad')
    # Mechanical rest is an intentional hard-stop pose. Its entry and startup
    # paths must pass through the margin; all operating drawings retain 3 degrees.
    rest_transit = table['rest_margin_exempt'] and (item['kind'] == 'startup' or (item['kind'] == 'pose' and item['name'] == 'rest'))
    if not rest_transit and minimum_margin < table['margin_degrees'] - 1e-6:
        problems.append(f'Joint-limit margin is only {minimum_margin:.2f} deg')
    limits = spec.get('speed_degrees_s') if spec else table['scene_speed_degrees_s'].get(item['name']) if item['kind'] == 'scene' else None
    if limits:
        for name, peak in peaks.items():
            ceiling = limits.get(name, limits['other'])
            if np.degrees(peak) > ceiling + 1e-6: problems.append(f'{name} speed {np.degrees(peak):.1f} exceeds {ceiling} deg/s')
    if spec:
        apex = item['keyframes'][spec['main_keyframe']]
        main = apex['positions']; apex_time = apex['time']
    elif item['kind'] == 'tracking':
        apex = max(item['samples'], key=lambda s: max(abs(s['positions'][n] - start[n]) for n in names))
        main, apex_time = apex['positions'], apex['time']
    else:
        # Pose items are round trips. Measure the held target, not their final home.
        poses = yaml.safe_load((ROOT / 'motion/config/v2/poses.yaml').read_text())['poses']
        main = poses[item['name']]['positions'] if item['kind'] == 'pose' else final
        apex_time = 0
    height, gaze = (float(v) for v in fk(model, data, main) - start_fk)
    movement = max(float(np.degrees(abs(main[name] - start[name]))) for name in names)
    if item['kind'] == 'pose':
        direction = table['poses'][item['name']]
        if direction == 'up' and not (height > 1 or gaze > 1): problems.append('Pose intent requires a head lift or raised gaze')
        if direction == 'down' and gaze > -1: problems.append('Pose intent requires a lowered gaze')
    is_turn = item['name'].startswith(('look_left', 'look_right', 'attention_left', 'attention_right', 'look_at_')) or item['name'] in table['scene_speed_degrees_s']
    if is_turn:
        largest_turn = max(np.degrees(abs(s['positions']['base_yaw_joint']) + abs(s['positions']['head_roll_joint'])) for s in item['samples'])
        if largest_turn > 90 + 1e-6: problems.append(f'Combined yaw and swivel reach {largest_turn:.1f} deg')
    main_joint_movement = float(np.degrees(abs(main[spec['main_joint']] - start[spec['main_joint']]))) if spec and 'main_joint' in spec else movement
    heading_change = (heading(model, data, main) - heading(model, data, start) + 180) % 360 - 180
    if spec:
        problems.extend(intent_problems(spec, height, gaze, main_joint_movement, gaze_series, apex_time, item['samples']))
        if spec['direction'] == 'lateral' and abs(heading_change) < 1:
            problems.append('Intent requires a sideways head turn')
    no_op = max(abs(sample['positions'][name] - start[name]) for sample in item['samples'] for name in names) < 1e-9
    if no_op and item['start_pose'] == 'rest': problems = []
    arm_movement = max(np.degrees(abs(s['positions'][n] - start[n])) for s in item['samples'] for n in ('shoulder_pitch_joint', 'elbow_pitch_joint'))
    if item['kind'] == 'tracking' and arm_movement > item['maximum_arm_degrees'] + 1e-6:
        problems.append(f'Arm fallback exceeds {item["maximum_arm_degrees"]} degrees')
    return {'largest_arm_movement_deg': float(arm_movement), 'problems': problems, 'contacts': list(contacts.values()), 'clamped_samples': clamps,
            'limit_boundary_samples': dict(boundary), 'minimum_margin_degrees': float(minimum_margin),
            'outside_limits': sorted(outside), 'peak_speed': peaks, 'peak_speed_degrees_s': {n: float(np.degrees(v)) for n, v in peaks.items()},
            'servo_speed_limit': speed_limit, 'end_anchor_error_rad': end_error,
            'main_joint_movement_deg': main_joint_movement, 'head_heading_change_deg': heading_change,
            'head_height_change_mm': height, 'gaze_pitch_change_deg': gaze, 'largest_joint_movement_deg': movement,
            'intent': spec['intent'] if spec else table['poses'].get(item['name'], item['kind']),
            'no_op': no_op, 'view_bounds': bounds.tolist()}


def video_name(item):
    return re.sub(r'[^a-zA-Z0-9_-]', '_', f'{item["kind"]}-{item["name"]}-{item.get("start_pose",item["anchor"])}-to-{item["anchor"]}') + '.mp4'


def camera_framing(bounds, fovy):
    bounds = np.array(bounds)
    centre = bounds.mean(axis=0)
    radius = np.linalg.norm(bounds[1] - bounds[0]) / 2
    return centre, radius / np.sin(np.radians(fovy / 2)) * 1.12


def record(item, document, model, output, fps):
    video = output / video_name(item)
    width, height = 960, 540
    font = ImageFont.load_default(size=18)
    checks = inspect(item, document, model)
    # Fit the entire lamp over all samples, with space around its enclosing sphere.
    centre, distance = camera_framing(checks['view_bounds'], model.vis.global_.fovy)
    cameras = []
    for azimuth, elevation in ((0, -7), (90, -16)):
        camera = mujoco.MjvCamera(); camera.lookat[:] = centre
        camera.distance = distance; camera.azimuth = azimuth; camera.elevation = elevation
        cameras.append(camera)
    encoder = subprocess.Popen(['ffmpeg', '-loglevel', 'error', '-y', '-f', 'rawvideo', '-pix_fmt', 'rgb24',
        '-s', f'{width}x{height}', '-r', str(fps), '-i', '-', '-an', '-c:v', 'libx264', '-preset', 'fast',
        '-crf', '23', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(video)], stdin=subprocess.PIPE)
    try:
        data = mujoco.MjData(model)
        with mujoco.Renderer(model, height=480, width=480) as renderer:
            for index, sample in enumerate(item['samples']):
                if index % (50 // fps): continue
                fk(model, data, sample['positions'])
                frame = Image.new('RGB', (width, height), '#eef1ed')
                for column, camera in enumerate(cameras):
                    renderer.update_scene(data, camera=camera)
                    # A green sphere marks the scripted hand point on the desk.
                    observations = item.get('hand_points', [])
                    observation = next((o for o in reversed(observations) if o['time'] <= sample['time']), None)
                    if observation and observation['point'] is not None:
                        scene = renderer.scene
                        mujoco.mjv_initGeom(scene.geoms[scene.ngeom], mujoco.mjtGeom.mjGEOM_SPHERE,
                            np.array([0.009] * 3), np.array(observation['point']), np.eye(3).ravel(),
                            np.array([0.15, 0.9, 0.3, 1.]))
                        scene.ngeom += 1
                    frame.paste(Image.fromarray(renderer.render()), (column * 480, 60))
                pen = ImageDraw.Draw(frame)
                pen.text((12, 5), f'{item["kind"]}: {item["name"]} · start {item["start_pose"]} · anchor {item["anchor"]}', font=font, fill='#17251b')
                pen.text((12, 31), f'Side view                  {sample["time"]:.2f} s', font=font, fill='#17251b')
                pen.text((492, 31), 'Front view', font=font, fill='#17251b')
                encoder.stdin.write(frame.tobytes())
    finally:
        encoder.stdin.close()
        if encoder.wait() != 0: raise RuntimeError(f'Video encoding failed: {video}')
    return result_row(item, checks)


def result_row(item, checks):
    return {'kind': item['kind'], 'name': item['name'], 'start_pose': item['start_pose'], 'anchor': item['anchor'],
            'anchor_source': item.get('anchor_source', 'Runtime catalogue: pose round trip, startup, or scene'),
            'planner': item.get('planner', 'catalog'), 'video': video_name(item), **checks}


def write_comparison(output, baseline, document, model):
    before = json.loads(baseline.read_text())
    if isinstance(before, dict): before = before['items']
    prior = {(i['name'], i.get('start_pose', i['anchor'])): i for i in before if i['kind'] == 'motion'}
    current = {(i['name'], i['start_pose']): i for i in document['items'] if i['kind'] == 'motion'}
    table = intent_table(); data = mujoco.MjData(model)
    rows = []
    for name, spec in table['motions'].items():
        start = spec['contexts'][0]['start']
        if (name, start) not in prior: continue
        values = []
        for item in (prior[(name, start)], current[(name, start)]):
            main = item['keyframes'][spec['main_keyframe']]['positions']
            values.append(fk(model, data, main) - fk(model, data, item['samples'][0]['positions']))
        old, new = values
        rows.append({'motion': name, 'intent': spec['intent'], 'start': start,
                     'before_head_mm': float(old[0]), 'after_head_mm': float(new[0]),
                     'before_gaze_deg': float(old[1]), 'after_gaze_deg': float(new[1])})
    (output / 'comparison.json').write_text(json.dumps(rows, indent=2) + '\n')
    markdown = '| Motion | Intent | Head mm, before → after | Gaze deg, before → after |\n| --- | --- | ---: | ---: |\n'
    html_rows = []
    for row in rows:
        head = f'{row["before_head_mm"]:+.1f} → {row["after_head_mm"]:+.1f}'
        gaze = f'{row["before_gaze_deg"]:+.1f} → {row["after_gaze_deg"]:+.1f}'
        markdown += f'| `{row["motion"]}` | {row["intent"]} | {head} | {gaze} |\n'
        html_rows.append('<tr>' + ''.join('<td>' + html.escape(value) + '</td>' for value in (row['motion'], row['intent'], head, gaze)) + '</tr>')
    (output / 'comparison.md').write_text(markdown)
    (output / 'comparison.html').write_text('<details><summary>Before and after: every motion</summary><p>Each change uses the main compiled keyframe relative to its own starting anchor. Positive gaze means looking up.</p><table><thead><tr><th>Motion</th><th>Intent</th><th>Head mm: before → after</th><th>Gaze deg: before → after</th></tr></thead><tbody>' + ''.join(html_rows) + '</tbody></table></details>')


def write_index(output, results):
    results.sort(key=lambda r: (not bool(r['problems']), r['kind'], r['name'], r['start_pose']))
    comparison = (output / 'comparison.html').read_text() if (output / 'comparison.html').is_file() else ''
    cards = []
    for result in results:
        problems = result['problems']
        peak = max(result['peak_speed'].values(), default=0)
        detail = json.dumps({key: value for key, value in result.items() if key not in ('video', 'problems')}, indent=2)
        cards.append(f'<article class="{"problem" if problems else "pass"}"><h2>{html.escape(result["kind"] + ": " + result["name"])}</h2>'
                     f'<p>Start: {html.escape(result["start_pose"])} · anchor: {html.escape(result["anchor"])}</p><p>{html.escape(result["intent"])}</p><video controls preload="none" src="{result["video"]}"></video>'
                     f'<p>{html.escape("; ".join(problems) if problems else "All checks pass")}</p>'
                     f'<p>Head: {result["head_height_change_mm"]:+.1f} mm · gaze: {result["gaze_pitch_change_deg"]:+.1f}° · largest joint: {result["largest_joint_movement_deg"]:.1f}°</p>'
                     f'<p>Peak speed: {np.degrees(peak):.1f} deg/s. Clamps: {len(result["clamped_samples"])}. '
                     f'End error: {result.get("end_anchor_error_rad", float("nan")):.6f} rad.</p>'
                     f'<details><summary>Sample checks</summary><pre>{html.escape(detail)}</pre></details></article>')
    (output / 'index.html').write_text('<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width">'
        '<title>Orion movement review</title><style>body{font:16px system-ui;margin:24px;background:#f3f3ef;color:#17251b}'
        'main{display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:20px}article{background:white;padding:16px;border-top:5px solid #587c67}'
        'table{border-collapse:collapse;margin:20px 0}td,th{padding:8px;border-bottom:1px solid #ccd3cc;text-align:left}.problem{border-color:#b45b35}video{width:100%}h2{font-size:20px}pre{white-space:pre-wrap;font-size:12px}</style>'
        f'<h1>Orion V2 movement review</h1><p>{len(results)} items; {sum(bool(r["problems"]) for r in results)} with problems. Problems appear first.</p>'
        f'<p>{html.escape(CONTACT_SCOPE)}</p><p>Compiler commands use 50 Hz. Videos use 25 or 50 frames per second. '
        'Head height uses v21_face; gaze uses the light_frame forward axis. Speed checks use compiler velocities and consecutive 50 Hz command differences. Positive gaze change means looking up. A pose moves from home to its target and back. '
        'Rest and startup retain their intentional hard-stop transit. Other drawings require 3° joint-limit margin. Joined links and rest-to-rest no-op clips do not add problems.</p>' + comparison + '<main>' + ''.join(cards) + '</main></html>')
    (output / 'checks.json').write_text(json.dumps(results, indent=2) + '\n')
    problems = [f'{r["kind"]} {r["name"]} (from {r["start_pose"]}, anchor {r["anchor"]}): ' + '; '.join(r['problems'])
                for r in results if r['problems']]
    (output / 'problems.txt').write_text('\n'.join(problems) + '\n' if problems else 'No problems.\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT.parent / 'scratch/motion-previews')
    parser.add_argument('--compiler', type=Path, help='Use an existing oriond instead of building it')
    parser.add_argument('--baseline', type=Path, help='Previous motion sample JSON for a before/after table')
    parser.add_argument('--validate-only', action='store_true', help='Compile and check without recording videos; fail on a problem')
    parser.add_argument('--checks-only', action='store_true', help='Recheck existing videos without rendering')
    parser.add_argument('--fps', type=int, choices=(25, 50), default=25)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.is_relative_to(ROOT): parser.error('Keep review material outside the repository')
    if shutil.which('ffmpeg') is None: parser.error('Install ffmpeg first')
    output.mkdir(parents=True, exist_ok=True)
    document = compile_samples(output, args.compiler.resolve() if args.compiler else None)
    model = contact_model(); results = []
    for item in document['items']:
        if args.validate_only:
            result = result_row(item, inspect(item, document, model))
        elif args.checks_only:
            if not (output / video_name(item)).is_file(): raise RuntimeError(f'Missing video: {video_name(item)}')
            result = result_row(item, inspect(item, document, model))
        else:
            result = record(item, document, model, output, args.fps)
        results.append(result)
        print(f'{len(results)}/{len(document["items"])} {item["kind"]} {item["name"]}: {"; ".join(result["problems"]) or "pass"}', flush=True)
    if args.baseline: write_comparison(output, args.baseline, document, model)
    write_index(output, results)
    print(output / 'index.html')
    if args.validate_only and any(r['problems'] for r in results): raise SystemExit(1)


if __name__ == '__main__': main()
