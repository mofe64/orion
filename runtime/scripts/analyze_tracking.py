#!/usr/bin/env python3
"""Analyze opt-in oriond JSONL capture. Standard library only; angles are radians."""
import argparse
import bisect
from collections import defaultdict
import json
import math
from pathlib import Path
import statistics
import sys


def interpolate(times, values, t):
    i = bisect.bisect_right(times, t)
    if i == 0:
        return values[0]
    if i == len(times):
        return values[-1]
    weight = (t - times[i - 1]) / (times[i] - times[i - 1])
    return values[i - 1] + weight * (values[i] - values[i - 1])


def approximate_lag(times, commanded, measured, max_lag=0.5):
    """Positive lag means measured(t) follows commanded(t-lag).

    Correlation removes bias/gain so shoulder droop is not mistaken for lag.
    Use the same interior window for every shift, with a 10 ms search grid.
    """
    if len(times) < 12 or times[-1] - times[0] <= 2 * max_lag + 0.2:
        return None, None
    if max(commanded) - min(commanded) < 0.003 or max(measured) - min(measured) < 0.003:
        return None, None
    dt = 0.01
    grid = [times[0] + max_lag + i * dt
            for i in range(int((times[-1] - times[0] - 2 * max_lag) / dt) + 1)]
    measured_grid = [interpolate(times, measured, t) for t in grid]
    mean = statistics.fmean(measured_grid)
    y = [v - mean for v in measured_grid]
    yy = sum(v * v for v in y)
    candidates = []
    for step in range(-round(max_lag / dt), round(max_lag / dt) + 1):
        lag = step * dt
        x = [interpolate(times, commanded, t - lag) for t in grid]
        xm = statistics.fmean(x)
        x = [v - xm for v in x]
        xx = sum(v * v for v in x)
        if xx > 1e-12 and yy > 1e-12:
            score = sum(a * b for a, b in zip(x, y)) / math.sqrt(xx * yy)
            candidates.append((score, -abs(lag), lag))
    if not candidates:
        return None, None
    score, _, lag = max(candidates)
    return lag * 1000, score


def weighted_square(times, values):
    duration = times[-1] - times[0]
    if duration <= 0:
        return values[0] ** 2, 1.0
    area = sum((b - a) * (x * x + y * y) / 2
               for a, b, x, y in zip(times, times[1:], values, values[1:]))
    return area, duration


def category(name):
    if name.startswith('idle_'):
        return 'idle'
    if name in {'speaking_performance', 'speak_settle'}:
        return 'speech'
    return 'other'


def analyze(path, min_excursion=2 * math.tau / 4096, trial_run_ids=None):
    headers, summaries, ends = [], [], {}
    runs = defaultdict(list)
    for number, line in enumerate(Path(path).read_text().splitlines(), 1):
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f'Invalid JSON on line {number}: {error}') from error
        kind = row.get('kind')
        if kind == 'header':
            headers.append(row)
        elif kind == 'summary':
            summaries.append(row)
        elif kind == 'end':
            ends[row['run_id']] = row
        elif kind == 'sample':
            runs[row['run_id']].append(row)
        else:
            raise ValueError(f'Unknown record kind on line {number}: {kind}')
    if len(headers) != 1 or headers[0].get('schema_version') != 1:
        raise ValueError('Expected one schema-version-1 header; analyze each capture separately.')
    if not runs:
        raise ValueError('Capture has no movement samples.')
    warnings = []
    if len(summaries) != 1:
        warnings.append('No clean shutdown summary: capture completeness is unverified.')
    dropped = sum(row.get('dropped_records', 0) for row in summaries)
    if dropped:
        warnings.append(f'{dropped} records dropped: measurements may miss peaks or terminal events.')
    results = []
    all_rows = []
    for run_id, rows in sorted(runs.items()):
        if trial_run_ids is not None and run_id not in trial_run_ids:
            continue
        times = [r['runtime_time_seconds'] for r in rows]
        if any(b <= a for a, b in zip(times, times[1:])):
            raise ValueError(f'Run {run_id} has non-increasing times (spline clock used as run clock?).')
        all_rows.extend(rows)
        groups = defaultdict(list)
        for r in rows:
            cat = category(r['motion_name'])
            # A takeover keeps its run ID. Isolate idle and speech portions.
            label = 'speech' if cat == 'speech' else r['motion_name']
            groups[(cat, label)].append(r)
        for (cat, label), samples in groups.items():
            final_group = samples[-1] is rows[-1]
            terminal = ends.get(run_id, {}).get('state', 'incomplete') if final_group else 'replaced'
            settling = [r for r in samples if r['phase'] == 'settling']
            complete = terminal == 'completed' and bool(settling)
            end_time = samples[-1]['runtime_time_seconds']
            # This is the terminal settling window, not a post-completion hold.
            tail = [r for r in settling if r['runtime_time_seconds'] >= end_time - 0.25] if complete else []
            full_start = samples[0] is rows[0]
            output = {
                'run_id': run_id, 'run_name': samples[0]['run_name'], 'motion_name': label,
                'motion_names': list(dict.fromkeys(r['motion_name'] for r in samples)),
                'category': cat, 'terminal_state': terminal, 'samples': len(samples),
                'trajectory_revisions': len(set(r['trajectory_revision'] for r in samples)),
                'start_reference': 'run_start' if full_start else 'first_sample_after_takeover',
                'duration_seconds': end_time - samples[0]['runtime_time_seconds'],
                'settling_seconds': end_time - settling[0]['runtime_time_seconds'] if settling else None,
                'joints': {},
            }
            t = [r['runtime_time_seconds'] for r in samples]
            active = [r for r in samples if r['phase'] == 'executing']
            for joint in samples[0]['joints']:
                c = [r['joints'][joint]['commanded_position_rad'] for r in samples]
                m = [r['joints'][joint]['measured_position_rad'] for r in samples]
                c0 = samples[0]['commanded_start'][joint] if full_start else c[0]
                m0 = samples[0]['measured_start'][joint] if full_start else m[0]
                cp = max(abs(v - c0) for v in c)
                mp = max(abs(v - m0) for v in m)
                error = [a - b for a, b in zip(c, m)]
                area, duration = weighted_square(t, error)
                ratio = mp / cp if cp >= min_excursion else None
                lag, correlation = approximate_lag(
                    [r['runtime_time_seconds'] for r in active],
                    [r['joints'][joint]['commanded_position_rad'] for r in active],
                    [r['joints'][joint]['measured_position_rad'] for r in active],
                ) if active else (None, None)
                steady = [r['joints'][joint]['commanded_position_rad'] -
                          r['joints'][joint]['measured_position_rad'] for r in tail]
                settled_positions = [r['joints'][joint]['measured_position_rad'] for r in tail]
                output['joints'][joint] = {
                    'commanded_peak_rad': cp, 'measured_peak_rad': mp, 'amplitude_ratio': ratio,
                    'loses_over_one_third': ratio is not None and ratio < 2 / 3,
                    'rms_error_rad': math.sqrt(area / duration), 'max_error_rad': max(map(abs, error)),
                    'lag_ms': lag, 'lag_correlation': correlation,
                    'lag_at_search_limit': lag is not None and abs(lag) >= 500,
                    'steady_state_error_rad': statistics.fmean(steady) if steady else None,
                    'settle_position_std_rad': statistics.pstdev(settled_positions) if settled_positions else None,
                    'outside_command_envelope_rad': max(0, max(m) - max(c), min(c) - min(m)),
                    'max_measured_velocity_rad_s': max(abs(r['joints'][joint]['measured_velocity_rad_s']) for r in samples),
                    'error_square_integral': area, 'error_duration_seconds': duration,
                }
            results.append(output)
    if not results:
        raise ValueError('No samples match the selected trial run IDs.')
    aggregate = {}
    for cat in ['idle', 'speech', 'other']:
        selected = [r for r in results if r['category'] == cat]
        if not selected:
            continue
        aggregate[cat] = {}
        for joint in selected[0]['joints']:
            moving = [r['joints'][joint] for r in selected if r['joints'][joint]['amplitude_ratio'] is not None]
            values = [r['joints'][joint] for r in selected]
            lags = [v['lag_ms'] for v in moving if v['lag_ms'] is not None]
            area = sum(v['error_square_integral'] for v in values)
            duration = sum(v['error_duration_seconds'] for v in values)
            aggregate[cat][joint] = {
                'runs': len(selected), 'moving_runs': len(moving),
                'commanded_peak_rad': statistics.fmean(v['commanded_peak_rad'] for v in moving) if moving else None,
                'measured_peak_rad': statistics.fmean(v['measured_peak_rad'] for v in moving) if moving else None,
                'amplitude_ratio': sum(v['measured_peak_rad'] for v in moving) / sum(v['commanded_peak_rad'] for v in moving) if moving else None,
                'rms_error_rad': math.sqrt(area / duration),
                'max_error_rad': max(v['max_error_rad'] for v in values),
                'lag_ms': statistics.median(lags) if lags else None,
                'loss_runs': sum(v['loses_over_one_third'] for v in moving),
            }
    ordered = sorted(all_rows, key=lambda r: r['runtime_time_seconds'])
    intervals = [b['runtime_time_seconds'] - a['runtime_time_seconds']
                 for a, b in zip(ordered, ordered[1:]) if a['run_id'] == b['run_id']]
    skipped = sum(max(0, b['sequence'] - a['sequence'] - 1)
                  for a, b in zip(ordered, ordered[1:]) if a['run_id'] == b['run_id'])
    if skipped:
        warnings.append(f'{skipped} control-cycle samples missing within runs.')
    diagnostics = {
        'dropped_records': dropped, 'missing_cycles_within_runs': skipped,
        'median_cycle_ms': statistics.median(intervals) * 1000 if intervals else None,
        'max_cycle_ms': max(intervals) * 1000 if intervals else None,
        'max_control_work_ms': max(r['control_work_seconds'] for r in ordered) * 1000,
        'settle_timeouts': [r['run_id'] for r in results if r['terminal_state'] == 'timed_out'],
    }
    return {'source': str(path), 'header': headers[0], 'warnings': warnings,
            'selected_trial_run_ids': sorted(trial_run_ids) if trial_run_ids is not None else None,
            'minimum_excursion_rad': min_excursion, 'diagnostics': diagnostics,
            'aggregate': aggregate, 'runs': results}


def number(value, digits=4):
    return 'n/a' if value is None else f'{value:.{digits}f}'


def markdown(report):
    lines = [f'Capture: `{report["source"]}`', '',
             'Peaks use separate commanded/measured start positions. Ratio is measured/commanded.',
             'Category peaks average runs with at least two encoder counts of commanded excursion;',
             'RMS includes all samples; lag is the median best correlation shift (positive = follows).',
             'Feedback precedes the new goal write; lag includes the normal control-cycle delay.', '']
    for cat, joints in report['aggregate'].items():
        lines += [f'### {cat.capitalize()}', '',
                  '| Joint | Moving runs | Command peak (rad) | Measured peak (rad) | Ratio | RMS error (rad) | Lag (ms) |',
                  '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
        for joint, v in joints.items():
            lines.append(f'| {joint} | {v["moving_runs"]} | {number(v["commanded_peak_rad"])} | {number(v["measured_peak_rad"])} | {number(v["amplitude_ratio"],3)} | {number(v["rms_error_rad"])} | {number(v["lag_ms"],0)} |')
        lines.append('')
    lines += ['### Per-run measurements', '',
              '| Run | Motion | Joint | Command peak | Measured peak | Ratio | RMS | Max error | Lag ms | Final error | End |',
              '| ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |']
    for r in report['runs']:
        for joint, v in r['joints'].items():
            lines.append(f'| {r["run_id"]} | {r["motion_name"]} | {joint} | {number(v["commanded_peak_rad"])} | {number(v["measured_peak_rad"])} | {number(v["amplitude_ratio"],3)} | {number(v["rms_error_rad"])} | {number(v["max_error_rad"])} | {number(v["lag_ms"],0)} | {number(v["steady_state_error_rad"])} | {r["terminal_state"]} |')
    lines += ['', '### Amplitude losses greater than one third', '']
    losses = [(r, j, v) for r in report['runs'] for j, v in r['joints'].items() if v['loses_over_one_third']]
    lines += [f'- Run {r["run_id"]}, {r["motion_name"]}, {j}: ratio {v["amplitude_ratio"]:.3f}.' for r,j,v in losses] or ['None at the minimum excursion threshold.']
    lines += ['', '### Capture diagnostics', '', '```json', json.dumps(report['diagnostics'], indent=2), '```']
    lines += [''] + [f'- {w}' for w in report['warnings']]
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path)
    parser.add_argument('--trials', type=Path, help='Restrict results to run IDs in the experiment trials.jsonl.')
    parser.add_argument('--json', type=Path, help='Write machine-readable per-run and aggregate results.')
    parser.add_argument('--markdown', type=Path, help='Write the full readable report.')
    args = parser.parse_args()
    try:
        ids = None
        if args.trials:
            trials = [json.loads(line) for line in args.trials.read_text().splitlines()]
            ids = {r['run_id'] for r in trials if r['kind'] == 'idle_trial'}
            ids.update(r['motion_terminal']['run_id'] for r in trials if r['kind'] == 'speech_trial' and r.get('motion_terminal'))
        report = analyze(args.capture, trial_run_ids=ids)
    except (ValueError, KeyError, OSError) as error:
        parser.error(str(error))
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + '\n')
    result = markdown(report)
    if args.markdown:
        args.markdown.write_text(result)
    else:
        print(result, end='')


if __name__ == '__main__':
    main()
