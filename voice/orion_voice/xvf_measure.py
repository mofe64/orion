"""Record and analyse XVF3800 echo, wake and direction trials on the lamp.

``record`` captures every USB channel, polls the board's beam and AEC state,
and optionally plays a WAV through the same card so the board's echo
canceller sees it as the far end. ``analyze`` turns one or more recordings into
echo levels, AEC convergence, Rustpotter hits per channel and direction
statistics. Recordings stay outside the repository.
"""
from __future__ import annotations

import argparse
from datetime import datetime
import json
import math
from pathlib import Path
import subprocess
import threading
import time
import wave

import numpy as np

from .xvf_control import CONFIGURATION, LIVE, XvfControl

SAMPLE_RATE = 16_000
FRAME_SAMPLES = 320  # 20 ms, the listener's frame size
DEFAULT_DEVICE = "plughw:CARD=Array,DEV=0"
DEFAULT_OUT = Path.home() / "orion-measurements"
DEFAULT_WAKE_MODEL = Path(__file__).resolve().parents[1] / "models" / "wake" / "hey_orion_reference.rpw"
DEFAULT_WAKE_THRESHOLD = 0.8
LEAD_IN_SECONDS = 1.0
TAIL_SECONDS = 1.5


# ---------------------------------------------------------------- record


def record(args) -> Path:
    if args.seconds is None and args.play is None:
        raise SystemExit("Give --seconds, --play or both")
    play_seconds = wav_seconds(args.play) if args.play else 0.0
    seconds = max(args.seconds or 0.0, LEAD_IN_SECONDS + play_seconds + TAIL_SECONDS if args.play else 0.0)

    stamp = datetime.now().strftime("%Y%m%dT%H%M%S")
    folder = Path(args.out).expanduser() / f"{stamp}-{args.label}"
    folder.mkdir(parents=True, exist_ok=False)

    control = XvfControl.open()
    configuration = control.snapshot(CONFIGURATION)

    command = ["arecord", "-q", "-D", args.device, "-t", "raw", "-f", "S16_LE",
               "-r", str(SAMPLE_RATE), "-c", str(args.channels)]
    capture = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    started = threading.Event()
    stop = threading.Event()
    origin = {}
    frames_bytes = FRAME_SAMPLES * 2 * args.channels

    def read_audio(sink):
        while not stop.is_set():
            chunk = capture.stdout.read(frames_bytes)
            if not chunk:
                break
            if not started.is_set():
                # Time zero is the end of the first frame the card delivered.
                origin["t"] = time.monotonic() - len(chunk) / (2 * args.channels) / SAMPLE_RATE
                started.set()
            sink.writeframes(chunk)

    def poll(log):
        period = 1.0 / args.poll_hz
        next_at = time.monotonic()
        while not stop.is_set():
            now = time.monotonic()
            row = {"t": round(now - origin["t"], 4)}
            try:
                row.update({name: list(control.read(name)) for name in LIVE})
            except Exception as error:  # Keep recording; the gap shows in the log.
                row["error"] = str(error)
            log.write(json.dumps(row) + "\n")
            next_at += period
            time.sleep(max(0.0, next_at - time.monotonic()))

    events = {}
    with wave.open(str(folder / "capture.wav"), "wb") as sink, open(folder / "control.jsonl", "w") as log:
        sink.setnchannels(args.channels)
        sink.setsampwidth(2)
        sink.setframerate(SAMPLE_RATE)
        reader = threading.Thread(target=read_audio, args=(sink,), daemon=True)
        reader.start()
        if not started.wait(3.0):
            capture.kill()
            error = capture.stderr.read().decode(errors="replace").strip()
            folder.joinpath("FAILED").write_text(error + "\n")
            raise SystemExit("Capture did not start. If the device is busy, stop the listener first:\n"
                             "  sudo systemctl stop orion-listener\n" + error)
        poller = threading.Thread(target=poll, args=(log,), daemon=True)
        poller.start()
        print(f"Recording {seconds:.1f} s into {folder}", flush=True)
        if args.play:
            time.sleep(LEAD_IN_SECONDS)
            events["play_start"] = round(time.monotonic() - origin["t"], 4)
            subprocess.run(["aplay", "-q", "-D", args.device, str(args.play)], check=True)
            events["play_end"] = round(time.monotonic() - origin["t"], 4)
        remaining = seconds - (time.monotonic() - origin["t"])
        if remaining > 0:
            time.sleep(remaining)
        stop.set()
        capture.terminate()
        capture.wait(timeout=2)
        reader.join(timeout=2)
        poller.join(timeout=2)

    meta = {
        "label": args.label,
        "recorded_at": stamp,
        "device": args.device,
        "channels": args.channels,
        "sample_rate": SAMPLE_RATE,
        "poll_hz": args.poll_hz,
        "play": str(args.play) if args.play else None,
        "expected_azimuth_deg": args.expected_azimuth,
        "note": args.note,
        "events": events,
        "configuration": configuration,
    }
    (folder / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(f"Saved {folder}")
    return folder


def wav_seconds(path) -> float:
    with wave.open(str(path), "rb") as source:
        return source.getnframes() / source.getframerate()


# ---------------------------------------------------------------- analyze


def load_session(folder: Path):
    meta = json.loads((folder / "meta.json").read_text())
    with wave.open(str(folder / "capture.wav"), "rb") as source:
        channels = source.getnchannels()
        audio = np.frombuffer(source.readframes(source.getnframes()), dtype="<i2").reshape(-1, channels)
    rows = [json.loads(line) for line in (folder / "control.jsonl").read_text().splitlines() if line.strip()]
    return meta, audio, rows


def level_dbfs(samples: np.ndarray) -> float | None:
    if samples.size == 0:
        return None
    rms = math.sqrt(float(np.mean(samples.astype(np.float64) ** 2)))
    return round(20 * math.log10(max(rms, 1e-3) / 32768), 1)


def circular_stats(degrees) -> dict:
    values = np.radians(np.asarray(list(degrees), dtype=float))
    if values.size == 0:
        return {"count": 0, "mean_deg": None, "spread_deg": None}
    mean = complex(np.mean(np.cos(values)), np.mean(np.sin(values)))
    length = min(abs(mean), 1.0)
    # Circular standard deviation; small for tight clusters, grows without bound as they spread.
    spread = math.degrees(math.sqrt(max(0.0, -2 * math.log(length)))) if length > 0 else 180.0
    return {"count": int(values.size), "mean_deg": round(math.degrees(math.atan2(mean.imag, mean.real)) % 360, 1),
            "spread_deg": round(spread, 1)}


def wrap180(degrees: float) -> float:
    return (degrees + 180.0) % 360.0 - 180.0


def windows(meta, duration):
    events = meta.get("events", {})
    if "play_start" in events:
        start, end = events["play_start"], events["play_end"]
        return {"before": (0.2, start), "during": (start, end), "after": (end, duration)}
    return {"all": (0.2, duration)}


def channel_levels(audio, spans):
    result = {}
    for channel in range(audio.shape[1]):
        result[f"ch{channel}"] = {
            name: level_dbfs(audio[int(a * SAMPLE_RATE):int(b * SAMPLE_RATE), channel])
            for name, (a, b) in spans.items()}
    return result


def echo_reduction(levels, channels):
    """Raw mic level minus processed level during playback (6-channel firmware only)."""
    if channels < 6 or "during" not in levels["ch0"]:
        return None
    raw = [levels[f"ch{c}"]["during"] for c in range(2, 6)]
    raw_db = 10 * math.log10(np.mean([10 ** (value / 10) for value in raw]))
    return {f"ch{c}": round(raw_db - levels[f"ch{c}"]["during"], 1) for c in (0, 1)}


def aec_state(rows, spans):
    if "during" not in spans:
        return None
    a, b = spans["during"]
    during = [row for row in rows if a <= row["t"] < b and "AEC_AECCONVERGED" in row]
    if not during:
        return {"samples": 0}
    converged = [row["AEC_AECCONVERGED"][0] == 1 for row in during]
    first = next((row["t"] - a for row, ok in zip(during, converged) if ok), None)
    return {"samples": len(during), "converged_fraction": round(sum(converged) / len(converged), 2),
            "first_converged_after_s": None if first is None else round(first, 2),
            "path_changes": sum(row["AEC_AECPATHCHANGE"][0] for row in during)}


def direction(rows, energy_threshold, spans):
    """Beam azimuths while the auto-select beam reports speech, per window."""
    result = {}
    for name, (a, b) in spans.items():
        speech = [row for row in rows if a <= row["t"] < b and "AEC_SPENERGY_VALUES" in row
                  and (row["AEC_SPENERGY_VALUES"][3] or 0) > energy_threshold]
        auto = [math.degrees(row["AEC_AZIMUTH_VALUES"][3]) for row in speech
                if row["AEC_AZIMUTH_VALUES"][3] is not None]
        processed = [math.degrees(row["AUDIO_MGR_SELECTED_AZIMUTHS"][0]) for row in speech
                     if row["AUDIO_MGR_SELECTED_AZIMUTHS"][0] is not None]
        doa = [row["DOA_VALUE"][0] for row in speech if row.get("DOA_VALUE")]
        result[name] = {"auto_select_beam": circular_stats(auto),
                        "processed_doa": circular_stats(processed),
                        "doa_value": circular_stats(doa)}
    return result


def wake_hits(audio, model, threshold, detector_factory=None):
    if detector_factory is None:
        from .rustpotter import RustpotterWakeDetector

        def detector_factory():
            return RustpotterWakeDetector(model, threshold)
    hits = {}
    for channel in range(min(audio.shape[1], 2)):
        detector = detector_factory()
        mono = np.ascontiguousarray(audio[:, channel])
        found = []
        for start in range(0, len(mono) - FRAME_SAMPLES + 1, FRAME_SAMPLES):
            detection = detector.process(mono[start:start + FRAME_SAMPLES].astype("<i2").tobytes())
            if detection is not None:
                found.append({"t": round((start + FRAME_SAMPLES) / SAMPLE_RATE, 2),
                              "score": round(detection.score, 3)})
        hits[f"ch{channel}"] = found
    return hits


def analyse_session(folder: Path, *, energy_threshold=0.0, wake_model=None, threshold=DEFAULT_WAKE_THRESHOLD,
                    detector_factory=None) -> dict:
    meta, audio, rows = load_session(folder)
    duration = len(audio) / SAMPLE_RATE
    spans = windows(meta, duration)
    levels = channel_levels(audio, spans)
    summary = {
        "session": folder.name,
        "label": meta["label"],
        "duration_s": round(duration, 2),
        "events": meta.get("events", {}),
        "expected_azimuth_deg": meta.get("expected_azimuth_deg"),
        "configuration": meta.get("configuration", {}),
        "levels_dbfs": levels,
        "echo_reduction_db": echo_reduction(levels, audio.shape[1]),
        "aec": aec_state(rows, spans),
        "direction": direction(rows, energy_threshold, spans),
        "poll_errors": sum(1 for row in rows if "error" in row),
    }
    if wake_model is not None or detector_factory is not None:
        hits = wake_hits(audio, wake_model, threshold, detector_factory)
        if "during" in spans:
            a, b = spans["during"]
            for found in hits.values():
                for hit in found:
                    # Rustpotter fires at the end of the phrase; allow for the echo tail.
                    hit["during_playback"] = a <= hit["t"] < b + 0.5
        summary["wake_hits"] = hits
    return summary


def mounting_offset(summaries, window="all") -> dict | None:
    """Fit board angle = sign * lamp angle + offset over trials with a known position."""
    pairs = []
    for summary in summaries:
        expected = summary.get("expected_azimuth_deg")
        stats = summary["direction"].get(window) or summary["direction"].get("before") or {}
        measured = (stats.get("auto_select_beam") or {}).get("mean_deg")
        if expected is not None and measured is not None:
            pairs.append((summary["label"], expected, measured))
    if len(pairs) < 3:
        return None
    fits = []
    for sign in (1, -1):
        offset = circular_stats(wrap180(measured - sign * expected) for _, expected, measured in pairs)["mean_deg"]
        residuals = {label: round(wrap180(measured - sign * expected - offset), 1)
                     for label, expected, measured in pairs}
        worst = max(abs(value) for value in residuals.values())
        fits.append({"sign": sign, "offset_deg": offset, "worst_residual_deg": worst, "residuals_deg": residuals})
    return min(fits, key=lambda fit: fit["worst_residual_deg"])


def print_summary(summary):
    print(f"\n== {summary['label']} ({summary['session']}, {summary['duration_s']} s)")
    for channel, levels in summary["levels_dbfs"].items():
        print(f"  {channel} level dBFS: " + ", ".join(f"{k} {v}" for k, v in levels.items()))
    if summary["echo_reduction_db"]:
        print(f"  echo reduction vs raw mics (dB): {summary['echo_reduction_db']}")
    if summary["aec"]:
        print(f"  AEC during playback: {summary['aec']}")
    for window, stats in summary["direction"].items():
        beam = stats["auto_select_beam"]
        print(f"  direction {window}: auto beam {beam['mean_deg']} deg ±{beam['spread_deg']} "
              f"({beam['count']} speech polls), processed DoA {stats['processed_doa']['mean_deg']}")
    for channel, found in summary.get("wake_hits", {}).items():
        print(f"  wake hits {channel}: {found}")


def analyze(args):
    summaries = [analyse_session(Path(folder).expanduser(), energy_threshold=args.energy_threshold,
                                 wake_model=None if args.no_wake else args.wake_model,
                                 threshold=args.threshold)
                 for folder in args.sessions]
    for summary in summaries:
        print_summary(summary)
    fit = mounting_offset(summaries)
    if fit:
        print(f"\nMounting fit: lamp azimuth -> board azimuth sign {fit['sign']}, offset {fit['offset_deg']} deg, "
              f"worst residual {fit['worst_residual_deg']} deg")
    if args.json:
        Path(args.json).expanduser().write_text(json.dumps({"sessions": summaries, "mounting": fit}, indent=2) + "\n")
        print(f"Wrote {args.json}")


# ---------------------------------------------------------------- CLI


def parser():
    root = argparse.ArgumentParser(prog="python -m orion_voice.xvf_measure", description=__doc__.splitlines()[0])
    commands = root.add_subparsers(dest="command", required=True)

    rec = commands.add_parser("record", help="Record one trial")
    rec.add_argument("--label", required=True, help="Short trial name, e.g. hey-1m-front-moving")
    rec.add_argument("--play", type=Path, help="WAV to play through the XVF3800 during the trial")
    rec.add_argument("--seconds", type=float, help="Minimum recording length")
    rec.add_argument("--expected-azimuth", type=float,
                     help="Speaker position in degrees, counter-clockwise from the lamp's front seen from above")
    rec.add_argument("--note", default="", help="Free text stored with the trial")
    rec.add_argument("--channels", type=int, choices=(2, 6), default=2, help="USB capture channels in the firmware")
    rec.add_argument("--device", default=DEFAULT_DEVICE)
    rec.add_argument("--poll-hz", type=float, default=20.0)
    rec.add_argument("--out", default=str(DEFAULT_OUT), help="Folder for trials (outside the repository)")

    ana = commands.add_parser("analyze", help="Summarise recorded trials")
    ana.add_argument("sessions", nargs="+", help="Trial folders written by record")
    ana.add_argument("--wake-model", type=Path, default=DEFAULT_WAKE_MODEL)
    ana.add_argument("--threshold", type=float, default=DEFAULT_WAKE_THRESHOLD,
                     help=f"Rustpotter threshold (the service uses {DEFAULT_WAKE_THRESHOLD})")
    ana.add_argument("--no-wake", action="store_true", help="Skip Rustpotter")
    ana.add_argument("--energy-threshold", type=float, default=0.0,
                     help="Auto-select beam speech energy above which a poll counts as speech")
    ana.add_argument("--json", help="Also write the full summary to this file")
    return root


def main(argv=None):
    args = parser().parse_args(argv)
    if args.command == "record":
        if args.poll_hz <= 0 or args.poll_hz > 100:
            raise SystemExit("--poll-hz must be between 0 and 100")
        record(args)
    else:
        analyze(args)


if __name__ == "__main__":
    main()
