#!/usr/bin/env python3
"""Shared experiment guards. Importing this module never contacts hardware."""
import argparse
import json
import math
from pathlib import Path
import socket
import subprocess
import time

LATCH = ".experiment-safety-hold.json"

class TemperatureGuard:
    """Confirm three fresh >55 C observations; reject >5 C jumps within 1 s.

    Rejected observations never replace the trusted reference. A stable high
    plateau can become a reference after one second, then needs three plausible
    observations: a persistent hot reading must not be hidden indefinitely.
    """
    def __init__(self):
        self.states = {}

    def observe(self, snapshot, log):
        timestamp = snapshot["sampled_at_unix_ns"] / 1e9
        reasons = []
        for joint in snapshot["joints"]:
            name, value = joint["name"], joint["temperature_c"]
            previous = self.states.get(name)
            if previous and timestamp <= previous["seen"]:
                continue  # Polling the same 50 Hz snapshot is not another observation.
            if not math.isfinite(value) or not 0 <= value <= 100:
                log(dict(kind="temperature_glitch", joint=name, temperature_c=value,
                         sampled_at_unix_ns=snapshot["sampled_at_unix_ns"], reason="invalid sensor range"))
                self.states.pop(name, None)
                continue
            if previous is None:
                previous = dict(seen=timestamp, raw=value, trusted=value, trusted_at=timestamp, high=0)
            raw_jump = abs(value - previous["raw"]) > 5 and timestamp - previous["seen"] < 1
            rejected = abs(value - previous["trusted"]) > 5 and timestamp - previous["trusted_at"] < 1
            if raw_jump or rejected:
                log(dict(kind="temperature_glitch", joint=name, temperature_c=value,
                         previous_temperature_c=previous["raw"], trusted_temperature_c=previous["trusted"],
                         sampled_at_unix_ns=snapshot["sampled_at_unix_ns"],
                         elapsed_seconds=timestamp-previous["seen"], rejected=rejected))
            previous["seen"], previous["raw"] = timestamp, value
            if rejected:
                previous["high"] = 0
            else:
                previous["trusted"], previous["trusted_at"] = value, timestamp
                previous["high"] = previous["high"] + 1 if value > 55 else 0
                if previous["high"] == 3:
                    reasons.append(f"Sustained plausible temperature above 55C: {name} {value}C")
            self.states[name] = previous
        return reasons


def stop_and_hold(request):
    """Cancel motion/audio, retaining the driver's last goal and holding torque.

    Do not use character stop: that schedules a return_home movement. A wrapper
    freezes the daemon after a latched abort to prevent future autonomous work.
    """
    failures = []
    for command in ("stop", "speech stop", "stop"):
        try:
            request(command)
        except (OSError, RuntimeError) as error:
            if "no movement is active" not in str(error):
                failures.append(f"{command}: {error}")
    return failures


def latch_hold(request, directory, reason, log):
    marker = Path(directory) / LATCH
    # Persist before issuing commands, so an exception cannot bypass teardown's gate.
    marker.write_text(json.dumps(dict(reason=reason, observed_unix_ns=time.time_ns())) + "\n")
    failures = stop_and_hold(request)
    log(dict(kind="safety_hold", reason=reason, command_failures=failures,
             torque_policy="retain holding torque; supervised recovery required"))


def measured_rest_complete(state):
    last = state.get("last_motion") or {}
    return (state.get("motion") is None and not state["torque_enabled"]
            and last.get("name") == "rest" and last.get("state") == "completed"
            and bool(state.get("joints"))
            and all(j["status"] == 0 and math.isfinite(j["velocity_rad_s"])
                    and abs(j["velocity_rad_s"]) < .05 for j in state["joints"]))


def socket_request(command):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect("/tmp/oriond.sock")
        connection.sendall(command.encode() + b"\n")
        data = bytearray()
        while not data.endswith(b"\n"):
            part = connection.recv(65536)
            if not part:
                break
            data.extend(part)
    response = json.loads(data)
    if command != "status" and not response.get("ok"):
        raise RuntimeError(f"{command}: {response}")
    return response


def freeze_daemon():
    # SIGSTOP suspends software scheduling without running driver shutdown/Drop.
    subprocess.run(["systemctl", "kill", "--kill-whom=main", "--signal=SIGSTOP", "oriond.service"], check=True)


def before_shutdown(directory, trial_exit):
    """Only a successful experiment may request normal rest automatically."""
    directory = Path(directory)
    def log(row):
        with (directory / "safety-cleanup.jsonl").open("a") as stream:
            stream.write(json.dumps(row) + "\n")
        print(json.dumps(row), flush=True)
    guard = TemperatureGuard()
    try:
        if trial_exit or (directory / LATCH).exists():
            raise RuntimeError("Experiment aborted; automatic rest/restoration is prohibited")
        state = socket_request("status")
        if not measured_rest_complete(state):
            response = socket_request("character rest")
            rest_id = response.get("run_id")
            deadline = time.monotonic() + 35
            while time.monotonic() < deadline:
                state = socket_request("status")
                reasons = guard.observe(state, log)
                if reasons:
                    raise RuntimeError("; ".join(reasons))
                last = state.get("last_motion") or {}
                if last.get("state") == "timed_out":
                    raise RuntimeError("Rest settle timeout")
                if measured_rest_complete(state) and (rest_id is None or last["run_id"] == rest_id):
                    break
                time.sleep(.1)
            else:
                raise RuntimeError("Measured rest completion not reached")
        (directory / "pre-shutdown-rest.json").write_text(json.dumps(state, indent=2) + "\n")
    except BaseException as error:
        try:
            latch_hold(socket_request, directory, str(error), log)
        finally:
            freeze_daemon()
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--trial-exit", type=int, default=0)
    arguments = parser.parse_args()
    before_shutdown(arguments.directory, arguments.trial_exit)
