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

import mujoco
import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
CONTACT_SCOPE = ('MuJoCo mesh convex hulls; contacts include parent links. Welded parts in the same body are excluded. '
                 'These are conservative collision candidates, including designed joint interfaces. '
                 'V2 follows joint targets kinematically; this check does not model servo loads or base tipping.')


def compile_samples(output, compiler):
    if compiler is None:
        subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'runtime/Cargo.toml'), '--bin', 'oriond'], check=True, cwd=ROOT)
        compiler = ROOT / 'runtime/target/debug/oriond'
    target = output / 'samples.json'
    result = subprocess.run([str(compiler), '--validate-catalog', '--catalog', str(ROOT), '--hardware', 'v2', '--output', str(target)], cwd=ROOT)
    if not target.is_file(): raise RuntimeError(f'Catalog compiler failed ({result.returncode})')
    return json.loads(target.read_text())


def contact_model():
    spec = mujoco.MjSpec.from_file(str(ROOT / 'simulation/mujoco/v2/scene.xml'))
    # Compile after changing masks: MuJoCo also derives body collision masks.
    # Editing only model.geom_contype would leave those body masks disabled.
    for geom in spec.geoms:
        if geom.parent.name != 'world':
            geom.contype = 1
            geom.conaffinity = 1
    spec.option.disableflags |= mujoco.mjtDisableBit.mjDSBL_FILTERPARENT
    return spec.compile()


def inspect(item, document, model, render_sample=None):
    if 'error' in item: return {'problems': [item['error']], 'contacts': [], 'clamped_samples': [], 'peak_speed': {}}
    data = mujoco.MjData(model)
    names = [joint['name'] for joint in document['joint_limits']]
    indices = {name: model.jnt_qposadr[mujoco.mj_name2id(model, mujoco.mjtObj.mjOBJ_JOINT, name)] for name in names}
    peaks = dict.fromkeys(names, 0.0)
    contacts = {}
    outside = set()
    boundary = Counter()
    previous = None
    for index, sample in enumerate(item['samples']):
        positions = sample['positions']
        for limit in document['joint_limits']:
            name = limit['name']; value = positions[name]
            if value < limit['lower'] - 1e-9 or value > limit['upper'] + 1e-9: outside.add(name)
            if min(abs(value - limit['lower']), abs(value - limit['upper'])) < 1e-9: boundary[name] += 1
            data.qpos[indices[name]] = value
            if previous:
                dt = sample['time'] - previous['time']
                peaks[name] = max(peaks[name], abs(value - previous['positions'][name]) / dt)
        data.time = sample['time']
        mujoco.mj_forward(model, data)
        for contact in data.contact:
            if contact.dist >= -1e-6: continue
            bodies = sorted(int(model.geom_bodyid[g]) for g in (contact.geom1, contact.geom2))
            if bodies[0] == 0 or bodies[0] == bodies[1]: continue
            pair = tuple(mujoco.mj_id2name(model, mujoco.mjtObj.mjOBJ_BODY, b) for b in bodies)
            entry = contacts.setdefault(pair, {'bodies': pair, 'first_time': sample['time'], 'depth_m': 0.0, 'samples': 0, 'adjacent_links': bool(model.body_parentid[bodies[1]] == bodies[0])})
            entry['depth_m'] = max(entry['depth_m'], -float(contact.dist)); entry['samples'] += 1
        if render_sample is not None: render_sample(index, sample, data)
        previous = sample
    final = item['samples'][-1]['positions']
    end_error = max(abs(final[name] - item['expected_end'][name]) for name in names)
    speed_limit = document['maximum_velocity_rad_s']
    problems = []
    if contacts: problems.append(f'{len(contacts)} lamp-body contact pairs (convex hull candidates)')
    clamps = item['clamped_samples']
    if clamps: problems.append('Runtime clamped joints: ' + ', '.join(sorted(set(clamps))))
    if outside: problems.append('Samples outside limits: ' + ', '.join(sorted(outside)))
    fast = [name for name, peak in peaks.items() if peak > speed_limit + 1e-6]
    if fast: problems.append('Servo speed exceeded: ' + ', '.join(fast))
    if end_error > 1e-6: problems.append(f'End anchor error: {end_error:.6f} rad')
    return {'problems': problems, 'contacts': list(contacts.values()), 'clamped_samples': clamps,
            'limit_boundary_samples': dict(boundary), 'outside_limits': sorted(outside), 'peak_speed': peaks,
            'servo_speed_limit': speed_limit, 'end_anchor_error_rad': end_error}


def record(item, document, model, output, fps):
    slug = re.sub(r'[^a-zA-Z0-9_-]', '_', f'{item["kind"]}-{item["name"]}-{item["anchor"]}')
    video = output / f'{slug}.mp4'
    width, height = 960, 540
    font = ImageFont.load_default(size=19)
    cameras = []
    for azimuth, elevation in ((0, -7), (90, -16)):
        camera = mujoco.MjvCamera(); camera.lookat[:] = [0, 0, .17]
        camera.distance = .7; camera.azimuth = azimuth; camera.elevation = elevation
        cameras.append(camera)
    encoder = subprocess.Popen(['ffmpeg', '-loglevel', 'error', '-y', '-f', 'rawvideo', '-pix_fmt', 'rgb24',
        '-s', f'{width}x{height}', '-r', str(fps), '-i', '-', '-an', '-c:v', 'libx264', '-preset', 'fast',
        '-crf', '23', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(video)], stdin=subprocess.PIPE)
    try:
        with mujoco.Renderer(model, height=480, width=480) as renderer:
            def draw(index, sample, data):
                if index % (50 // fps): return
                frame = Image.new('RGB', (width, height), '#eef1ed')
                for column, camera in enumerate(cameras):
                    renderer.update_scene(data, camera=camera)
                    frame.paste(Image.fromarray(renderer.render()), (column * 480, 60))
                pen = ImageDraw.Draw(frame)
                pen.text((12, 5), f'{item["kind"]}: {item["name"]} · from {item["anchor"]}', font=font, fill='#17251b')
                pen.text((12, 31), f'Side view                         {sample["time"]:.2f} s', font=font, fill='#17251b')
                pen.text((492, 31), 'Front view', font=font, fill='#17251b')
                encoder.stdin.write(frame.tobytes())
            checks = inspect(item, document, model, draw)
            if 'error' in item:
                frame = Image.new('RGB', (width, height), '#eee5e1')
                ImageDraw.Draw(frame).text((20, 30), f'{item["name"]}\nCompilation failed. See checks below.', font=font, fill='black')
                for _ in range(fps): encoder.stdin.write(frame.tobytes())
    finally:
        encoder.stdin.close()
        if encoder.wait() != 0: raise RuntimeError(f'Video encoding failed: {video}')
    return {'kind': item['kind'], 'name': item['name'], 'anchor': item['anchor'], 'video': video.name, **checks}


def write_index(output, results):
    results.sort(key=lambda r: (not bool(r['problems']), not any(not c['adjacent_links'] for c in r['contacts']), r['kind'], r['name'], r['anchor']))
    cards = []
    for result in results:
        problems = result['problems']
        peak = max(result['peak_speed'].values(), default=0)
        detail = json.dumps({key: value for key, value in result.items() if key not in ('video', 'problems')}, indent=2)
        cards.append(f'<article class="{"problem" if problems else "pass"}"><h2>{html.escape(result["kind"] + ": " + result["name"])}</h2>'
                     f'<p>From {html.escape(result["anchor"])}</p><video controls preload="none" src="{result["video"]}"></video>'
                     f'<p>{html.escape("; ".join(problems) if problems else "All checks pass")}</p>'
                     f'<p>Peak speed: {peak:.3f} rad/s. Clamps: {len(result["clamped_samples"])}. '
                     f'End error: {result.get("end_anchor_error_rad", float("nan")):.6f} rad.</p>'
                     f'<details><summary>Sample checks</summary><pre>{html.escape(detail)}</pre></details></article>')
    (output / 'index.html').write_text('<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width">'
        '<title>Orion movement review</title><style>body{font:16px system-ui;margin:24px;background:#f3f3ef;color:#17251b}'
        'main{display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:20px}article{background:white;padding:16px;border-top:5px solid #587c67}'
        '.problem{border-color:#b45b35}video{width:100%}h2{font-size:20px}pre{white-space:pre-wrap;font-size:12px}</style>'
        f'<h1>Orion V2 movement review</h1><p>{len(results)} items; {sum(bool(r["problems"]) for r in results)} with problems. Problems appear first.</p>'
        f'<p>{html.escape(CONTACT_SCOPE)}</p><p>Compiler commands use 50 Hz. Videos use 25 or 50 frames per second. '
        'Speed checks use consecutive 50 Hz command differences. A pose moves from home to its target and back. The end check uses home for these round trips. '
        'Boundary counts include authored hard-stop positions; a boundary sample alone is not a clamp.</p><main>' + ''.join(cards) + '</main></html>')
    (output / 'checks.json').write_text(json.dumps(results, indent=2) + '\n')
    (output / 'problems.txt').write_text('\n'.join(
        f'{r["kind"]} {r["name"]} (from {r["anchor"]}): ' + '; '.join(r['problems'])
        for r in results if r['problems']) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT.parent / 'scratch/motion-previews')
    parser.add_argument('--compiler', type=Path, help='Use an existing oriond instead of building it')
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
        if args.checks_only:
            slug = re.sub(r'[^a-zA-Z0-9_-]', '_', f'{item["kind"]}-{item["name"]}-{item["anchor"]}')
            if not (output / f'{slug}.mp4').is_file(): raise RuntimeError(f'Missing video: {slug}')
            result = {'kind': item['kind'], 'name': item['name'], 'anchor': item['anchor'],
                      'video': f'{slug}.mp4', **inspect(item, document, model)}
        else:
            result = record(item, document, model, output, args.fps)
        results.append(result)
        print(f'{len(results)}/{len(document["items"])} {item["kind"]} {item["name"]}: {"; ".join(result["problems"]) or "pass"}', flush=True)
    write_index(output, results)
    print(output / 'index.html')


if __name__ == '__main__': main()
