#!/usr/bin/env bash
set -euo pipefail

project_root="${1:?Pi project root is required}"
branch="${2:?Git branch is required}"
runtime_socket="/tmp/oriond.sock"
calibration_file="${HOME}/.config/orion/servo_calibration.json"
token_file="${HOME}/.config/orion/studio-token"
state_directory="${HOME}/.local/state/orion"
daemon_pid_file="${state_directory}/oriond.pid"
gateway_pid_file="${state_directory}/studio-gateway.pid"
daemon_log="${state_directory}/oriond.log"
gateway_log="${state_directory}/studio-gateway.log"

cd "${project_root}"
mkdir -p "${state_directory}" "$(dirname "${token_file}")"

for command in git cargo python3 nohup pgrep pkill ps; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "Required Pi command is unavailable: ${command}" >&2
    exit 1
  fi
done
if [[ ! -f "${calibration_file}" ]]; then
  echo "Missing Orion calibration: ${calibration_file}" >&2
  exit 1
fi

old_binary="${project_root}/runtime/target/release/oriond"
active_binary="${old_binary}"
daemon_available=false

start_source_daemon() {
  local binary=$1
  local log_file=$2
  local pid_file=$3

  nohup "${binary}" --serve \
    --backend hardware \
    --port /dev/ttyACM0 \
    --baud-rate 1000000 \
    --calibration "${calibration_file}" \
    >"${log_file}" 2>&1 </dev/null &
  daemon_pid=$!
  echo "${daemon_pid}" > "${pid_file}"

  daemon_available=false
  for _ in {1..100}; do
    if "${binary}" --status --socket "${runtime_socket}" >/dev/null 2>&1; then
      daemon_available=true
      return 0
    fi
    if ! kill -0 "${daemon_pid}" 2>/dev/null; then
      echo "oriond exited while starting; see ${log_file}." >&2
      return 1
    fi
    sleep 0.1
  done
  echo "oriond did not become ready; see ${log_file}." >&2
  return 1
}

safe_shutdown() {
  local exit_code=$?
  trap - ERR INT TERM
  set +e
  echo "Deployment failed; attempting Orion's safe resting shutdown..." >&2
  if [[ "${daemon_available}" == true && -x "${active_binary}" ]]; then
    failure_status="$("${active_binary}" --status --socket "${runtime_socket}" 2>/dev/null)"
    torque_enabled="$(python3 -c 'import json,sys; print(str(json.loads(sys.argv[1]).get("torque_enabled", False)).lower())' "${failure_status}" 2>/dev/null)"
    if [[ "${torque_enabled}" == "true" ]]; then
      "${active_binary}" --stop-scene --socket "${runtime_socket}" >/dev/null 2>&1
      "${active_binary}" --stop --socket "${runtime_socket}" >/dev/null 2>&1
      if "${active_binary}" --run-scene return_to_rest --wait --socket "${runtime_socket}" >/dev/null 2>&1; then
        "${active_binary}" --disable --socket "${runtime_socket}" >/dev/null 2>&1
      else
        echo "Rest could not be confirmed; Orion remains holding. Do not disable torque until its posture is physically safe." >&2
      fi
    fi
  fi
  exit "${exit_code}"
}
trap safe_shutdown ERR INT TERM

current_branch="$(git branch --show-current)"
if [[ "${current_branch}" != "${branch}" ]]; then
  echo "Pi checkout is on '${current_branch:-detached HEAD}', expected '${branch}'. Refusing to switch branches." >&2
  exit 1
fi

echo "Fetching origin/${branch}..."
git fetch --prune origin "${branch}"
git merge --ff-only "origin/${branch}"
revision="$(git rev-parse --short=12 HEAD)"

if [[ ! -x "${old_binary}" ]]; then
  echo "An existing source-built oriond release is required for the pre-update rest sequence." >&2
  exit 1
fi
if ! "${old_binary}" --status --socket "${runtime_socket}" >/dev/null 2>&1; then
  echo "No daemon is running; starting the existing release in observe mode..."
  start_source_daemon "${old_binary}" "${daemon_log}" "${daemon_pid_file}"
else
  daemon_available=true
fi

echo "Returning Orion to rest with the currently running daemon..."
"${old_binary}" --stop-scene --socket "${runtime_socket}" >/dev/null 2>&1 || true
"${old_binary}" --stop --socket "${runtime_socket}" >/dev/null 2>&1 || true
current_status="$("${old_binary}" --status --socket "${runtime_socket}")"
current_mode="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["mode"])' "${current_status}")"
if [[ "${current_mode}" == "observe" ]]; then
  "${old_binary}" --configure --socket "${runtime_socket}"
  current_mode="configured"
fi
if [[ "${current_mode}" == "configured" ]]; then
  "${old_binary}" --enable --socket "${runtime_socket}"
fi
"${old_binary}" --run-scene return_to_rest --wait --socket "${runtime_socket}"
"${old_binary}" --disable --socket "${runtime_socket}"

echo "Stopping the previous source-run daemon..."
if [[ -f "${daemon_pid_file}" ]]; then
  daemon_pid="$(cat "${daemon_pid_file}")"
  if [[ "$(ps -p "${daemon_pid}" -o comm= 2>/dev/null)" == "oriond" ]]; then
    kill -TERM "${daemon_pid}" 2>/dev/null || true
  fi
fi
pkill -TERM -x oriond 2>/dev/null || true
for _ in {1..50}; do
  pgrep -x oriond >/dev/null || break
  sleep 0.1
done
if pgrep -x oriond >/dev/null; then
  echo "The previous oriond process did not stop." >&2
  exit 1
fi
daemon_available=false

echo "Testing and building Orion revision ${revision}..."
python3 -m py_compile orion_studio/gateway.py
python3 -m unittest discover -s orion_studio/tests -v
cargo test --locked --manifest-path runtime/Cargo.toml --all-targets -- \
  --skip mujoco::tests::rust_runtime_executes_and_settles_in_native_mujoco
cargo build --release --locked --manifest-path runtime/Cargo.toml

active_binary="${project_root}/runtime/target/release/oriond"
echo "Starting the rebuilt source-run daemon..."
start_source_daemon "${active_binary}" "${daemon_log}" "${daemon_pid_file}"

status_json="$("${active_binary}" --status --socket "${runtime_socket}")"
python3 -c 'import json,sys; value=json.loads(sys.argv[1]); expected=sys.argv[2]; actual=value.get("build_revision"); raise SystemExit(0 if actual == expected else f"running build {actual!r} does not match {expected!r}")' "${status_json}" "${revision}"

echo "Configuring, enabling, and moving to zero_reference..."
"${active_binary}" --configure --socket "${runtime_socket}"
"${active_binary}" --enable --socket "${runtime_socket}"
"${active_binary}" --goto zero_reference --duration 3.0 --wait --socket "${runtime_socket}"

echo "Testing Orion's RGBW shield and acknowledge cue..."
"${active_binary}" --run-scene deployment_smoke --wait --socket "${runtime_socket}"

echo "Returning Orion to rest, fading lights off, and disabling torque..."
"${active_binary}" --run-scene return_to_rest --wait --socket "${runtime_socket}"
"${active_binary}" --disable --socket "${runtime_socket}"

final_status="$("${active_binary}" --status --socket "${runtime_socket}")"
python3 -c 'import json,sys; value=json.loads(sys.argv[1]); assert value.get("torque_enabled") is False, value; assert value.get("mode") == "configured", value' "${final_status}"

if [[ ! -f "${token_file}" ]]; then
  echo "Creating the Studio pairing token; save the following value in Studio:"
  python3 orion_studio/gateway.py create-token --token-file "${token_file}"
fi

if [[ -f "${gateway_pid_file}" ]]; then
  gateway_pid="$(cat "${gateway_pid_file}")"
  gateway_command="$(ps -p "${gateway_pid}" -o args= 2>/dev/null)"
  if [[ "${gateway_command}" == *"orion_studio/gateway.py serve"* ]]; then
    kill -TERM "${gateway_pid}" 2>/dev/null || true
  fi
fi
pkill -TERM -f 'orion_studio/gateway.py serve' 2>/dev/null || true
nohup python3 orion_studio/gateway.py serve \
  --bind 0.0.0.0 \
  --port 7447 \
  --token-file "${token_file}" \
  --project-root "${project_root}" \
  >"${gateway_log}" 2>&1 </dev/null &
gateway_pid=$!
echo "${gateway_pid}" > "${gateway_pid_file}"

gateway_ready=false
for _ in {1..50}; do
  if python3 -c 'import socket; connection=socket.create_connection(("127.0.0.1", 7447), timeout=0.2); connection.close()' 2>/dev/null; then
    gateway_ready=true
    break
  fi
  if ! kill -0 "${gateway_pid}" 2>/dev/null; then
    echo "The Studio gateway exited; see ${gateway_log}." >&2
    exit 1
  fi
  sleep 0.1
done
if [[ "${gateway_ready}" != true ]]; then
  echo "The Studio gateway did not become reachable; see ${gateway_log}." >&2
  exit 1
fi

trap - ERR INT TERM
echo "Deployment ${revision} passed. Orion is at rest, lights are off, and torque is disabled."
echo "Studio gateway: http://orion.local:7447"
echo "oriond log: ${daemon_log}"
echo "gateway log: ${gateway_log}"
