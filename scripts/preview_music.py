#!/usr/bin/env python3
"""Render calm and upbeat music through Orion's beat tracker and character planner."""
import argparse
import json
from pathlib import Path
import subprocess
import wave

import numpy as np

from movement_preview_runtime import build_adapter
from preview_movements import ROOT, contact_model, record


def song(path, bpm, upbeat, seconds=48):
    rate = 48_000
    time = np.arange(rate * seconds) / rate
    beat = time * bpm / 60
    phase = beat % 1
    # Original short instrumental loops: a slow chord bed and melody, with
    # bass, kick and a brighter rhythm for the upbeat arrangement.
    notes = np.array([220., 261.6256, 329.6276, 293.6648])
    note = notes[(beat // 4).astype(int) % 4]
    level = .10 if upbeat else .018
    music = level * np.sin(2 * np.pi * note * time) * (.75 + .25 * np.sin(np.pi * beat / 4))
    music += level * .5 * np.sin(2 * np.pi * note * 1.5 * time)
    kick = np.sin(2 * np.pi * (55 * phase * 60 / bpm + 5 * (1 - np.exp(-phase * 30)))) * np.exp(-phase * 35)
    music += (.55 if upbeat else .055) * kick
    if upbeat:
        music += .05 * np.sin(2 * np.pi * 110 * time) * np.exp(-phase * 9)
        music += .025 * np.sin(2 * np.pi * 3300 * time) * np.exp(-(beat * 2 % 1) * 45)
    music[time >= seconds - 3] = 0
    stereo = np.repeat(music[:, None], 2, axis=1).astype('<f4')
    path.with_suffix('.f32').write_bytes(stereo.tobytes())
    with wave.open(str(path), 'wb') as audio:
        audio.setnchannels(2)
        audio.setsampwidth(2)
        audio.setframerate(rate)
        audio.writeframes((np.clip(stereo, -1, 1) * 32767).astype('<i2').tobytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.output is None and args.directory is None: parser.error('Choose an output directory')
    output = (args.output or args.directory).resolve()
    output.mkdir(parents=True, exist_ok=True)
    requests = []
    for name, bpm, upbeat in [('calm', 90, False), ('upbeat', 132, True)]:
        song(output / f'{name}.wav', bpm, upbeat)
        requests.append({'name': name, 'planner': 'music', 'pcm': str(output / f'{name}.f32')})
    request_file = output / 'requests.json'
    request_file.write_text(json.dumps(requests))
    adapter = build_adapter(ROOT)
    items = json.loads(subprocess.check_output([str(adapter), str(ROOT), str(ROOT), str(request_file)], cwd=ROOT))
    (output / 'samples.json').write_text(json.dumps(items))
    # The catalogue exporter owns the calibrated conversion to radians.
    catalog = output / 'catalog.json'
    subprocess.run([str(ROOT / 'runtime/target/debug/oriond'), '--validate-catalog', '--catalog', str(ROOT), '--hardware', 'v2', '--output', str(catalog)], check=True, cwd=ROOT)
    document = json.loads(catalog.read_text())
    model = contact_model()
    rows = []
    for item in items:
        if 'error' in item:
            raise RuntimeError(item['error'])
        row = record(item, document, model, output, 25)
        source = output / row['video']
        movie = output / f'{item["name"]}.mp4'
        subprocess.run(['ffmpeg', '-loglevel', 'error', '-y', '-i', str(source), '-i', str(output / f'{item["name"]}.wav'), '-c:v', 'copy', '-c:a', 'aac', '-shortest', str(movie)], check=True)
        row['video'] = movie.name
        rows.append(row)
    (output / 'checks.json').write_text(json.dumps(rows, indent=2))
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    main()
