#!/usr/bin/env python3
"""Compare compiled speech strokes and measured pitch maxima to their audio peaks.

Uses the runtime's software audio clock, not microphone/ALSA loopback timing.
Only nod strokes have a reliably identifiable positive pitch apex; reflective
tilts still appear in compiled statistics. Replaced, unperformed futures and
maxima touching the search boundary are excluded from measured statistics.
"""
import argparse
import json
import statistics
from pathlib import Path


def rows(path):
    for line in Path(path).read_text().splitlines():
        try:
            yield json.loads(line)
        except json.JSONDecodeError:
            continue  # Daemon stderr/journal also contains ordinary text.


def summarize(values):
    return dict(count=len(values), minimum_ms=min(values)*1000,
                median_ms=statistics.median(values)*1000,
                maximum_ms=max(values)*1000) if values else dict(count=0)


def analyze(samples, events):
    events = sorted((e for e in events if e.get('event') == 'speech.motion_compiled'),
                    key=lambda e: e['trajectory_start_runtime_seconds'])
    samples = [s for s in samples if s.get('kind') == 'sample']
    results = []
    for i, event in enumerate(events):
        start = event['trajectory_start_runtime_seconds']
        run = event['motion_run_id']
        next_start = next((e['trajectory_start_runtime_seconds'] for e in events[i+1:]
                           if e['motion_run_id'] == run), float('inf'))
        trace = [s for s in samples if s['run_id'] == run
                 and start <= s['runtime_time_seconds'] < next_start
                 and s['motion_name'] == 'speaking_performance']
        if not trace:
            continue
        last = trace[-1]['runtime_time_seconds']
        for apex in event['apices']:
            commanded = start + apex['commanded_apex_seconds']
            peak = start + apex['peak_seconds']
            row = dict(run_id=run, marker=apex['marker'],
                       commanded_apex_runtime_seconds=commanded,
                       audio_peak_runtime_seconds=peak,
                       compiled_apex_minus_peak_seconds=commanded-peak)
            if commanded >= min(next_start, last):
                row['measurement'] = 'unperformed_future'
            elif 'speak_emphasis_nod' not in apex['marker']:
                row['measurement'] = 'not_a_nod_pitch_apex'
            else:
                window = [s for s in trace if commanded-.10 <= s['runtime_time_seconds'] <= commanded+.35]
                if len(window) < 6:
                    row['measurement'] = 'insufficient_samples'
                else:
                    maximum = max(s['joints']['head_pitch_joint']['measured_position_rad'] for s in window)
                    plateau = [s['runtime_time_seconds'] for s in window
                               if abs(s['joints']['head_pitch_joint']['measured_position_rad']-maximum) < 1e-9]
                    if plateau[0] <= window[0]['runtime_time_seconds'] or plateau[-1] >= window[-1]['runtime_time_seconds']:
                        row['measurement'] = 'maximum_on_window_boundary'
                    else:
                        measured = (plateau[0]+plateau[-1])/2
                        row.update(measurement='measured', measured_apex_runtime_seconds=measured,
                                   measured_apex_minus_peak_seconds=measured-peak,
                                   measured_apex_plateau_ms=(plateau[-1]-plateau[0])*1000)
            results.append(row)
    compiled = [r['compiled_apex_minus_peak_seconds'] for r in results if r['measurement'] != 'unperformed_future']
    measured = [r['measured_apex_minus_peak_seconds'] for r in results if r['measurement'] == 'measured']
    return dict(compiled=summarize(compiled), measured_nod_pitch=summarize(measured), strokes=results)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('tracking', type=Path)
    parser.add_argument('daemon_log', type=Path)
    parser.add_argument('--json', type=Path)
    args = parser.parse_args()
    result = analyze(list(rows(args.tracking)), list(rows(args.daemon_log)))
    output = json.dumps(result, indent=2) + '\n'
    if args.json:
        args.json.write_text(output)
    print(json.dumps({k:v for k,v in result.items() if k != 'strokes'}, indent=2))


if __name__ == '__main__':
    main()
