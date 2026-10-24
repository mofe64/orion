"""Private, resumable wake training capture. Also runs with the Pi's system Python."""
from __future__ import annotations

import argparse
from array import array
from contextlib import contextmanager
from datetime import datetime
import json
import math
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import wave

RATE = 16000
COMMANDS = (
    "what time is it", "turn on the lights", "set a timer for five minutes", "stop",
    "what is the weather", "play some music", "turn the volume down", "pause the music",
    "set an alarm for seven", "tell me a joke", "turn the lights off", "how are you",
    "cancel the timer", "what day is it", "turn the volume up", "resume the music",
    "dim the lights", "add milk to my list", "read my reminders", "go to sleep",
)
NEAR_MISSES = ("Hey Ryan", "Orion", "Hey on", "Hey, oh no", "Hey Siri", "Hey Google",
               "O'Brien", "Hey Orin", "Ryan", "hey, are you on", "Oregon", "Hey Arya")


def speaker_name(value):
    if not re.fullmatch(r"[A-Za-z0-9]+", value):
        raise argparse.ArgumentTypeError("Speaker names use letters and digits only")
    return value


def sets():
    conditions = (
        ("normal", "1 m, normal voice", 1), ("quiet", "1 m, quiet voice", 1),
        ("loud", "1 m, loud voice", 1), ("three-metres", "3 m, normal voice", 3),
        ("across-room", "Across the room (4 m or more), normal voice", "4+"),
        ("facing-away", "1 m, facing away from the lamp", 1),
        ("sitting", "1 m, sitting down", 1), ("fast", '1 m, fast "HeyOrion"', 1),
        ("slow", '1 m, slow "Hey... Orion"', 1),
        ("music", "With music playing on Orion", 1), ("tv", "With the TV or other talk on", 1),
    )
    result = [dict(name=name, label=label, distance=distance, music=name == "music",
                   prompts=["Hey Orion"] * 10, seconds=3.5) for name, label, distance in conditions]
    result.append(dict(name="command", label="Say the wake word and command without a pause", distance=1,
                       music=False, prompts=["Hey Orion, " + command for command in COMMANDS], seconds=6))
    result.extend(dict(name=f"near-miss-{i+1:02d}", label="Say only the displayed phrase", distance=1,
                       music=False, prompts=[phrase] * 3, seconds=3.5) for i, phrase in enumerate(NEAR_MISSES))
    return result


class Manifest:
    def __init__(self, directory, speaker):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.path = self.directory / "manifest.json"
        self.data = json.loads(self.path.read_text()) if self.path.exists() else {"speaker": speaker, "takes": []}
        if self.data["speaker"] != speaker:
            raise ValueError("Manifest speaker does not match")

    def save(self):
        temporary = self.path.with_suffix(".tmp")
        temporary.write_text(json.dumps(self.data, indent=2) + "\n")
        temporary.replace(self.path)

    def complete(self, name, slot):
        return any(t["set"] == name and t["slot"] == slot and t["status"] == "kept" for t in self.data["takes"])

    def add(self, condition, slot, prompt, pcm):
        take = 1
        while (self.directory / f'{condition["name"]}-{take:03d}.wav').exists():
            take += 1
        filename = f'{condition["name"]}-{take:03d}.wav'
        # Exclusive creation also protects recordings not yet in the manifest.
        with (self.directory / filename).open("xb") as stream, wave.open(stream, "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(RATE)
            output.writeframes(pcm)
        samples = array("h", pcm)
        if sys.byteorder != "little":
            samples.byteswap()
        peak = max((abs(x) for x in samples), default=0)
        rms = sorted(math.sqrt(sum(x*x for x in samples[i:i+320]) / len(samples[i:i+320]))
                     for i in range(0, len(samples), 320))
        dbfs = lambda value: round(20 * math.log10(max(value / 32768, 1e-6)), 2)
        record = dict(speaker=self.data["speaker"], set=condition["name"], take=take, slot=slot,
                      prompt=prompt, distance=condition["distance"], music=condition["music"],
                      status="pending", peak_dbfs=dbfs(peak), floor_dbfs=dbfs(rms[len(rms)//10] if rms else 0),
                      timestamp=datetime.now().astimezone().isoformat(), file=filename)
        self.data["takes"].append(record)
        self.save()
        return record


def listener_settings():
    start = subprocess.run(["systemctl", "show", "-p", "ExecStart", "--value", "orion-listener"],
                           capture_output=True, text=True, check=True).stdout
    match = re.search(r"argv\[\]=(.*?) ;", start)
    args = shlex.split(match[1] if match else start)
    def value(flag, default):
        for i, arg in enumerate(args):
            if arg == flag:
                return args[i+1]
            if arg.startswith(flag + "="):
                return arg.split("=", 1)[1]
        return default
    hardware = value("--hardware", "v1")
    executable = Path(args[0]).resolve()
    # Console scripts live at <release>/voice/.venv/bin/orion-listener.
    release = executable.parents[3]
    return dict(hardware=hardware, device=value("--device", "plughw:CARD=Array,DEV=0" if hardware == "v2"
                else "plughw:CARD=seeed2micvoicec,DEV=0"), channels=int(value("--capture-channels", "2")),
                processed_channel=int(value("--processed-channel", "0")),
                configurator=release / "hardware/audio/configure-capture.sh")


@contextmanager
def microphone(settings):
    # The finally covers stop failures and signals that Python turns into exceptions.
    try:
        subprocess.run(["sudo", "systemctl", "stop", "orion-listener"], check=True)
        if settings["hardware"] == "v1":
            subprocess.run([str(settings["configurator"]), "seeed2micvoicec"], check=True)
        yield
    finally:
        subprocess.run(["sudo", "systemctl", "start", "orion-listener"], check=True)


def capture(settings, seconds, countdown=False):
    channels = settings["channels"]
    command = ["arecord", "-q", "-D", settings["device"], "-t", "raw", "-f", "S16_LE", "-r", str(RATE),
               "-c", str(channels)]
    process = subprocess.Popen(command, stdout=subprocess.PIPE)
    try:
        def read_frames(count):
            needed = count * channels * 2
            chunk = process.stdout.read(needed)
            if len(chunk) != needed:
                raise RuntimeError("Microphone capture ended early")
            return chunk
        read_frames(4800)
        if settings["hardware"] == "v1":
            subprocess.run([str(settings["configurator"]), "seeed2micvoicec"], check=True)
            read_frames(4800)
        print("\n" + "=" * 48 + "\n                 SPEAK NOW\n" + "=" * 48, flush=True)
        mono = array("h")
        remaining = round(seconds * RATE)
        while remaining:
            count = min(remaining, RATE * 60)
            samples = array("h", read_frames(count))
            if sys.byteorder != "little":
                samples.byteswap()
            if settings["hardware"] == "v2":
                mono.extend(samples[settings["processed_channel"]::channels])
            else:
                # Match SatelliteSession's signed integer downmix.
                mono.extend((samples[i] + samples[i+1]) // 2 for i in range(0, len(samples), 2))
            remaining -= count
            if countdown:
                print(f"{math.ceil(remaining / RATE / 60)} minutes remaining", flush=True)
        if sys.byteorder != "little":
            mono.byteswap()
        return mono.tobytes()
    finally:
        process.stdout.close()
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main(argv=None):
    def interrupt(_signum, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupt)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--speaker", required=True, type=speaker_name)
    parser.add_argument("--out", type=Path, default=Path.home() / ".local/share/orion/wake-samples")
    parser.add_argument("--background", nargs="?", const=30, type=float, metavar="MINUTES")
    args = parser.parse_args(argv)
    if args.background is not None and (not math.isfinite(args.background) or args.background <= 0):
        parser.error("--background must be a positive number of minutes")
    if input("Has this person agreed to be recorded for wake-word training? [y/n] ").strip().lower() != "y":
        return
    manifest = Manifest(args.out.expanduser() / args.speaker / datetime.now().strftime("%Y%m%d"), args.speaker)
    settings = listener_settings()
    try:
        with microphone(settings):
            if args.background is not None:
                print("Live normally: talk, TV, music on Orion. Do not say Hey Orion.")
                input("Press Enter to record. ")
                condition = dict(name="background", distance=None, music=True)
                record = manifest.add(condition, 1, "Do not say Hey Orion", capture(settings, args.background * 60, True))
                record["status"] = "kept"
                manifest.save()
                return
            plan = sets()
            for index, condition in enumerate(plan, 1):
                pending = [slot for slot in range(1, len(condition["prompts"])+1) if not manifest.complete(condition["name"], slot)]
                if not pending:
                    continue
                print("\n" + condition["label"])
                if condition["name"] == "music":
                    input("Start music on Orion from Spotify now. Press Enter when it plays.")
                elif condition["name"] == "tv":
                    input("Turn on the TV or other talk. Press Enter when it plays. ")
                for slot in pending:
                    while True:
                        print(f'set {index} of {len(plan)}, take {slot} of {len(condition["prompts"])}: {condition["prompts"][slot-1]}')
                        input("Press Enter to record. ")
                        record = manifest.add(condition, slot, condition["prompts"][slot-1], capture(settings, condition["seconds"]))
                        choice = input("Enter = keep, r = redo, q = quit and save progress: ").strip().lower()
                        record["status"] = "redone" if choice == "r" else "kept"
                        manifest.save()
                        if choice == "q":
                            return
                        if choice != "r":
                            break
        print(f"Saved in {manifest.directory}. Nothing was uploaded.")
    except KeyboardInterrupt:
        print(f"\nProgress saved in {manifest.directory}.")


if __name__ == "__main__":
    main()
