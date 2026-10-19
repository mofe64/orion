#!/usr/bin/env python3
"""Render recorded music through Orion's beat tracker, dance planner and V2 model."""
import argparse
import html
import json
from pathlib import Path
import subprocess
import wave

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import numpy as np

from movement_preview_runtime import build_adapter
from preview_movements import ROOT, contact_model, record


JOINTS = ['base_yaw_joint', 'shoulder_pitch_joint', 'elbow_pitch_joint',
          'head_roll_joint', 'head_pitch_joint']
LABELS = ['Base yaw', 'Shoulder pitch', 'Elbow pitch', 'Neck turn', 'Head pitch (positive = down)']


def prepare_recording(track, output):
    """Keep the recording's pre-roll for tempo acquisition; fade before the end."""
    start, duration = float(track['start']), float(track.get('duration', 40))
    source = Path(track['source']).resolve()
    # No synthesized percussion or supplied BPM enters the runtime beat tracker.
    raw = subprocess.check_output(['ffmpeg', '-v', 'error', '-i', str(source),
        '-t', str(start + duration), '-f', 'f32le', '-ar', '48000', '-ac', '2', '-'])
    stereo = np.frombuffer(raw, dtype='<f4').copy().reshape(-1, 2)
    count = round((start + duration) * 48000)
    if len(stereo) < count:
        raise ValueError(f'Recording is too short: {source}')
    stereo = stereo[:count]
    stereo *= float(track.get('gain', 1.))
    time = np.arange(count) / 48000 - start
    # Leave room to see the planner's one-beat return after the audio ends.
    stereo *= np.clip((duration - 1.2 - time) / 0.8, 0, 1)[:, None]
    pcm = output / f'{track["name"]}.f32'
    pcm.write_bytes(stereo.tobytes())
    excerpt = stereo[round(start * 48000):]
    audio_path = output / f'{track["name"]}.wav'
    with wave.open(str(audio_path), 'wb') as audio:
        audio.setnchannels(2)
        audio.setsampwidth(2)
        audio.setframerate(48000)
        audio.writeframes((np.clip(excerpt, -1, 1) * 32767).astype('<i2').tobytes())
    return {'name': track['name'], 'planner': 'music', 'pcm': str(pcm), 'start': start}


def joint_plot(item, output, title):
    samples = item['samples']
    time = np.array([s['time'] for s in samples])
    beats = np.array([s['beats'] for s in samples])
    # Anchor comes from the runtime adapter, rather than the first moving sample.
    anchor = item['expected_end']
    beat_times = time[1:][np.floor(beats[1:]) > np.floor(beats[:-1])]
    fig, axes = plt.subplots(5, 1, figsize=(14, 9), sharex=True, layout='constrained')
    colors = ['#305d8a', '#b65c31', '#286f60', '#78579b', '#ad4860']
    for ax, name, label, color in zip(axes, JOINTS, LABELS, colors):
        values = np.degrees([s['positions'][name] - anchor[name] for s in samples])
        ax.plot(time, values, color=color, linewidth=1.4)
        ax.axhline(0, color='#505861', linewidth=0.6)
        for at in beat_times:
            ax.axvline(at, color='#dce0e4', linewidth=0.6, zorder=0)
        ax.set_ylabel(f'{label}\n(deg from anchor)', fontsize=9)
        ax.spines[['top', 'right']].set_visible(False)
        ax.grid(axis='y', alpha=0.16)
        ax.set_xlim(0, 40)
    axes[-1].set_xlabel('Time in excerpt (s); pale lines mark the tracked beats')
    fig.suptitle(title, fontsize=16, weight='bold')
    paths = []
    for suffix in ['png', 'svg']:
        path = output / f'{item["name"]}-joints.{suffix}'
        fig.savefig(path, dpi=160)
        paths.append(path.name)
    plt.close(fig)
    return paths


def write_index(output, tracks, rows):
    cards = []
    for track, row in zip(tracks, rows):
        escape = lambda value: html.escape(str(value), quote=True)
        credit = (f'{track["title"]} by {track["artist"]}. '
                  f'Excerpt with a fade and a silent tail. {track["bpm"]} BPM in the source catalogue.')
        if track.get('gain', 1.) != 1.:
            credit += ' Playback level is reduced for the calm example.'
        note = f'<p>Tracked tempo: {row["tracked_bpm"]:.1f} BPM.</p>'
        cards.append(f'<article><h2>{escape(track["title"])} · {track["bpm"]} BPM</h2>'
            f'<video controls preload="metadata" src="{escape(row["video"])}"></video>'
            f'<img alt="Five joint angles over the excerpt" src="{escape(row["plots"][0])}">'
            f'<p>{escape(credit)} <a href="{escape(track["page"])}">Recording source</a>. '
            f'<a href="{escape(track["license"])}">CC BY 4.0</a>.</p>{note}</article>')
    (output / 'index.html').write_text('<!doctype html><html lang="en"><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width"><title>Orion dance previews</title>'
        '<style>body{font:16px system-ui;max-width:1100px;margin:32px auto;padding:0 20px;color:#17251b;'
        'background:#f3f3ef}article{margin:36px 0}video,img{width:100%;display:block;margin:14px 0}'
        'p{line-height:1.6}a{color:#305d8a}</style><h1>Orion dance previews</h1>'
        '<p>The left panel shows the side view. The right panel shows the front view. '
        'Each preview uses a real recording through the runtime beat tracker and dance planner.</p>'
        '<p>Owner approval is required before release. MuJoCo follows the commanded joints; '
        'these previews do not establish physical servo performance.</p>' + ''.join(cards) + '</html>')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--tracks', required=True, type=Path,
        help='JSON list: name, source, start, duration, bpm, title, artist, page and license')
    parser.add_argument('--samples-only', action='store_true', help='Compile samples and plots without video rendering')
    args = parser.parse_args()
    if args.output is None and args.directory is None:
        parser.error('Choose an output directory')
    output = (args.output or args.directory).resolve()
    output.mkdir(parents=True, exist_ok=True)
    tracks = json.loads(args.tracks.read_text())
    requests = [prepare_recording(track, output) for track in tracks]
    request_file = output / 'requests.json'
    request_file.write_text(json.dumps(requests))
    adapter = build_adapter(ROOT)
    items = json.loads(subprocess.check_output([str(adapter), str(ROOT), str(ROOT), str(request_file)], cwd=ROOT))
    (output / 'samples.json').write_text(json.dumps(items))
    for item, track in zip(items, tracks):
        if 'error' in item:
            raise RuntimeError(f'{track["name"]}: {item["error"]}')
        tempos = np.array([s['bpm'] for s in item['samples'] if s['clip'] == 'music_performance'])
        if not len(tempos) or np.max(np.abs(tempos / track['bpm'] - 1)) > 0.03:
            raise RuntimeError(f'{track["name"]}: tracked tempo does not match the recording')
        item['tempo_summary'] = {'source_bpm': track['bpm'], 'median_bpm': float(np.median(tempos)),
                                 'min_bpm': float(tempos.min()), 'max_bpm': float(tempos.max())}
        item['plots'] = joint_plot(item, output, f'Orion · {track["title"]} · {track["bpm"]} BPM recording')
    (output / 'tempo-summary.json').write_text(json.dumps({i['name']: i['tempo_summary'] for i in items}, indent=2))
    if args.samples_only:
        return
    # The catalogue exporter owns the calibrated conversion to radians.
    subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'runtime/Cargo.toml'),
                    '--bin', 'oriond'], check=True, cwd=ROOT)
    catalog = output / 'catalog.json'
    subprocess.run([str(ROOT / 'runtime/target/debug/oriond'), '--validate-catalog', '--catalog', str(ROOT),
                    '--hardware', 'v2', '--output', str(catalog)], check=True, cwd=ROOT)
    document = json.loads(catalog.read_text())
    model = contact_model()
    rows = []
    for item, track in zip(items, tracks):
        row = record(item, document, model, output, 25)
        source = output / row['video']
        movie = output / f'{item["name"]}-bpm.mp4'
        credit = f'{track["title"]} by {track["artist"]}; CC BY 4.0 {track["license"]}; {track["page"]}; {track["changes"]}'
        subprocess.run(['ffmpeg', '-loglevel', 'error', '-y', '-i', str(source), '-i',
            str(output / f'{item["name"]}.wav'), '-c:v', 'copy', '-c:a', 'aac', '-shortest',
            '-metadata', f'comment={credit}', '-movflags', '+faststart', str(movie)], check=True)
        row.update(video=movie.name, plots=item['plots'], tracked_bpm=item['tempo_summary']['median_bpm'],
                   tempo_summary=item['tempo_summary'])
        rows.append(row)
    (output / 'checks.json').write_text(json.dumps(rows, indent=2))
    (output / 'recordings.json').write_text(json.dumps(tracks, indent=2))
    write_index(output, tracks, rows)
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    main()
