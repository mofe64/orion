#!/usr/bin/env python3
"""Retire this checkout's legacy voice workers and archive known downloaded models."""
import argparse
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import time

LEGACY_COMMANDS = {"wake-worker", "listen-worker", "tts-worker"}
MODEL_PATHS = (
    "wake/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01",
    "asr/sherpa-onnx-moonshine-tiny-en-int8", "vad/silero_vad.onnx",
)


def is_legacy_command(arguments, root):
    arguments = [arg.removeprefix("path=").removeprefix("argv[]=") for arg in arguments]
    # Exact checkout ownership, not a global 'python' or 'arecord' match.
    executables = {str(root / "voice/.venv/bin/orion-voice"),
                   str(root / "voice/.venv/bin/python"),
                   str(root / "voice/.venv/bin/python3")}
    return (bool(executables.intersection(arguments))
            and bool(LEGACY_COMMANDS.intersection(arguments))
            and (str(root / "voice/.venv/bin/orion-voice") in arguments
                 or "orion_voice" in arguments or "orion_voice.__main__" in arguments))


def archive_models(root, backup):
    models = root / "voice/models"
    candidates = [models / name for name in MODEL_PATHS]
    # Only identify Piper downloads by their own model configuration.
    for config in models.glob("*.onnx.json"):
        if config.is_symlink():
            continue
        try:
            value = json.loads(config.read_text())
        except (OSError, ValueError):
            continue
        if isinstance(value, dict) and "phoneme_id_map" in value and "audio" in value:
            candidates.extend([config, config.with_suffix("")])
    moved = []
    for source in candidates:
        if not source.exists() or source.is_symlink():
            continue
        # Reject symlinked parent directories rather than traversing outside checkout.
        if not source.resolve().is_relative_to(models.resolve()) or models.is_symlink():
            raise RuntimeError("Legacy model path leaves this checkout")
        destination = backup / "models" / source.relative_to(models)
        destination.parent.mkdir(parents=True, exist_ok=True)
        source.rename(destination)
        moved.append(str(source.relative_to(root)))
    return moved


def service_names(output):
    """Select concrete services; bare templates have no runnable ExecStart."""
    names = set()
    for line in output.splitlines():
        fields = line.split()
        if fields and fields[0].endswith(".service") and not fields[0].endswith("@.service"):
            names.add(fields[0])
    return names


def systemctl_error(action, prefix, result):
    scope = "user" if "--user" in prefix else "system"
    detail = result.stderr.strip() or f"systemctl exited {result.returncode}"
    return RuntimeError(f"Could not {action} ({scope} services): {detail}")


def retire_services(root):
    for user_scope in (False, True):
        prefix = ["systemctl", "--user"] if user_scope else ["systemctl"]
        listed = subprocess.run(prefix + ["list-unit-files", "--type=service", "--no-legend", "--no-pager"],
                                capture_output=True, text=True)
        if listed.returncode:
            if not user_scope:
                raise systemctl_error("list installed units", prefix, listed)
            # User units cannot be silently skipped if this checkout installed any.
            user_units = Path.home() / ".config/systemd/user"
            for unit in user_units.glob("*.service"):
                if str(root / "voice") in unit.read_text() and any(c in unit.read_text() for c in LEGACY_COMMANDS):
                    raise RuntimeError("Start the user systemd manager to retire legacy Orion voice units")
            continue
        # Installed files include templates; loaded units include concrete instances
        # and transient workers which may have no separately installed unit file.
        loaded = subprocess.run(prefix + ["list-units", "--all", "--type=service",
                                          "--no-legend", "--no-pager", "--plain", "--full"],
                                capture_output=True, text=True)
        if loaded.returncode:
            raise systemctl_error("list loaded units", prefix, loaded)
        for unit in sorted(service_names(listed.stdout) | service_names(loaded.stdout)):
            inspected = subprocess.run(prefix + ["show", unit, "--property=ExecStart", "--value"],
                                       capture_output=True, text=True)
            if inspected.returncode:
                raise systemctl_error(f"inspect {unit}", prefix, inspected)
            command = inspected.stdout
            # systemd renders ExecStart with metadata; ownership and command both required.
            if str(root / "voice/.venv/bin/") not in command:
                continue
            words = shlex.split(command.replace(";", " ").replace("}", " "))
            if not is_legacy_command(words, root):
                continue
            stopped = subprocess.run((prefix if user_scope else ["sudo", *prefix]) + ["disable", "--now", unit])
            if stopped.returncode:
                raise RuntimeError(f"Could not disable and stop legacy unit {unit}; "
                                   "refusing to replace its voice environment")
            print(f"Retired legacy unit: {unit}")


def process_snapshot(proc=Path("/proc")):
    processes = {}
    for path in proc.iterdir():
        if not path.name.isdigit():
            continue
        try:
            if path.stat().st_uid != os.getuid():
                continue
            arguments = path.joinpath("cmdline").read_bytes().decode().split("\0")
            cwd = path.joinpath("cwd").resolve()
            arguments = [str((cwd / arg).absolute()) if arg.startswith(("voice/", "./voice/", ".venv/", "./.venv/")) else arg for arg in arguments]
            fields = path.joinpath("stat").read_text().rsplit(")", 1)[1].split()
            if fields[0] == "Z":
                continue  # Exited children no longer own audio devices.
            processes[int(path.name)] = (int(fields[1]), arguments, fields[19])
        except (OSError, ValueError, IndexError, UnicodeError):
            continue
    return processes


def legacy_processes(processes, root):
    selected = {pid for pid, (_, args, _) in processes.items() if is_legacy_command(args, root)}
    # Include ALSA children of those workers, not unrelated capture/playback.
    while True:
        descendants = {pid for pid, (parent, _, _) in processes.items() if parent in selected}
        if descendants <= selected:
            return selected
        selected |= descendants


def stop_processes(root):
    before = process_snapshot()
    selected = legacy_processes(before, root)
    for pid in selected:
        try:
            if process_snapshot().get(pid, (None, None, None))[2] == before[pid][2]:
                os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + 10
    while selected and time.monotonic() < deadline:
        current = process_snapshot()
        selected = {pid for pid in selected if pid in current and current[pid][2] == before[pid][2]}
        if selected:
            time.sleep(.1)
    if selected or legacy_processes(process_snapshot(), root):
        raise RuntimeError("Legacy voice workers remain active; refusing to replace their environment")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--backup", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    if not (root / "voice/pyproject.toml").is_file():
        parser.error("Expected an Orion checkout")
    retire_services(root)
    stop_processes(root)
    for name in archive_models(root, args.backup):
        print(f"Archived legacy model: {name}")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        sys.exit(f"Legacy voice retirement failed: {error}")
