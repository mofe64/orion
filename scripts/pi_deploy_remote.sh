#!/usr/bin/env bash
set -euo pipefail

# Keep this upgrade step in the uploaded bootstrap: the Pi checkout may predate it.
merge_updated_checkout() {
  local incoming_ref=$1
  if git merge-base --is-ancestor HEAD "${incoming_ref}" \
    && [[ -f voice/uv.lock && ! -L voice/uv.lock && ! -L voice ]] \
    && ! git ls-files --error-unmatch -- voice/uv.lock >/dev/null 2>&1 \
    && ! git cat-file -e HEAD:voice/uv.lock 2>/dev/null \
    && git cat-file -e "${incoming_ref}:voice/uv.lock" 2>/dev/null; then
    echo "Removing the generated, untracked voice/uv.lock before installing the committed lockfile..."
    rm -- voice/uv.lock
  fi
  git merge --ff-only "${incoming_ref}"
}

project_root="${1:?Pi project root is required}"
branch="${2:?Git branch is required}"
runtime_socket="/tmp/oriond.sock"
orion_user="$(id -un)"
user_home="${HOME}"
calibration_file="${user_home}/.config/orion/servo_calibration.json"
token_file="${user_home}/.config/orion/studio-token"

if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo "Refusing unsafe Pi project path: ${project_root}" >&2
  exit 2
fi
if [[ ! "${branch}" =~ ^[A-Za-z0-9._/-]+$ || "${branch}" == -* || "${branch}" == *".."* ]]; then
  echo "Refusing unsafe Git branch: ${branch}" >&2
  exit 2
fi

# Non-interactive SSH shells may not load the profile that adds Rustup tools.
export PATH="${user_home}/.cargo/bin:${PATH}"

cd "${project_root}"
mkdir -p "$(dirname "${token_file}")"

for command in git cargo python3 pgrep pkill ps systemctl sudo id; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "Required Pi command is unavailable: ${command}" >&2
    exit 1
  fi
done
if [[ ! -f "${calibration_file}" ]]; then
  echo "Missing Orion calibration: ${calibration_file}" >&2
  exit 1
fi
if ! sudo -v; then
  echo "Pi sudo authentication failed. Run deployment from a terminal and enter the Pi user password when prompted." >&2
  exit 1
fi

old_binary="${project_root}/runtime/target/release/oriond"
active_binary="${old_binary}"
daemon_available=false

unit_exists() {
  systemctl cat "$1" >/dev/null 2>&1
}

start_temporary_daemon() {
  local binary=$1

  nohup "${binary}" --serve \
    --backend hardware \
    --port /dev/ttyACM0 \
    --baud-rate 1000000 \
    --calibration "${calibration_file}" \
    >"${user_home}/oriond-deploy.log" 2>&1 </dev/null &
  temporary_daemon_pid=$!

  daemon_available=false
  for _ in {1..100}; do
    if "${binary}" --status --socket "${runtime_socket}" >/dev/null 2>&1; then
      daemon_available=true
      return 0
    fi
    if ! kill -0 "${temporary_daemon_pid}" 2>/dev/null; then
      echo "Temporary oriond exited while starting; see ${user_home}/oriond-deploy.log." >&2
      return 1
    fi
    sleep 0.1
  done
  echo "Temporary oriond did not become ready; see ${user_home}/oriond-deploy.log." >&2
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
      if "${active_binary}" --goto rest --duration 3.0 --wait --socket "${runtime_socket}" >/dev/null 2>&1; then
        "${active_binary}" --disable --socket "${runtime_socket}" >/dev/null 2>&1
      else
        echo "Rest could not be confirmed; Orion remains holding. Support it physically before stopping the service." >&2
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

if [[ ! -x "${old_binary}" ]]; then
  echo "An existing source-built oriond release is required for the pre-update rest sequence." >&2
  exit 1
fi

if unit_exists orion-listener.service; then
  sudo systemctl stop orion-listener.service
fi

if unit_exists orion-studio-gateway.service; then
  sudo systemctl stop orion-studio-gateway.service
fi
pkill -TERM -f 'orion_studio/gateway.py serve' 2>/dev/null || true

if ! "${old_binary}" --status --socket "${runtime_socket}" >/dev/null 2>&1; then
  echo "No daemon is running; starting the existing release temporarily for the resting shutdown..."
  start_temporary_daemon "${old_binary}"
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
"${old_binary}" --goto rest --duration 3.0 --wait --socket "${runtime_socket}"
"${old_binary}" --disable --socket "${runtime_socket}"

echo "Stopping the previous runtime before replacing the legacy catalog..."
if unit_exists oriond.service; then
  sudo systemctl stop oriond.service
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

echo "Fetching origin/${branch}..."
git fetch --prune origin "${branch}"
merge_updated_checkout "origin/${branch}"
revision="$(git rev-parse --short=12 HEAD)"

echo "Archiving non-v2 Pi user assets before loading the breaking catalog..."
bash scripts/migrate_v2_user_assets.sh \
  "${project_root}" "${user_home}/.local/share/orion/backups"

echo "Testing and building Orion v2 revision ${revision} while torque remains off..."
python3 -m py_compile orion_studio/gateway.py
python3 -m unittest discover -s orion_studio/tests -v
cargo test --locked --manifest-path runtime/Cargo.toml --all-targets -- \
  --skip mujoco::tests::rust_runtime_executes_and_settles_in_native_mujoco
cargo build --release --locked --manifest-path runtime/Cargo.toml
active_binary="${project_root}/runtime/target/release/oriond"
"${project_root}/runtime/target/release/orion-trajectory" \
  --motion look_at_left_expressive \
  --start-pose attentive \
  --pose-file "${project_root}/motion/config/poses.yaml" \
  --motions-directory "${project_root}/motion/motions" \
  --calibration "${calibration_file}" >/dev/null

if [[ ! -f "${token_file}" ]]; then
  echo "Creating the Studio pairing token; save the following value in Studio:"
  python3 orion_studio/gateway.py create-token --token-file "${token_file}"
fi

echo "Installing Rustpotter voice and retiring the legacy voice stack..."
scripts/install_pi_voice.sh "${project_root}" "${user_home}"

echo "Installing and enabling Orion's boot services..."
scripts/install_pi_services.sh "${project_root}" "${orion_user}" "${user_home}"
sudo systemctl restart oriond.service
scripts/wait_for_oriond.sh "${active_binary}" "${runtime_socket}" 20
daemon_available=true

status_json="$("${active_binary}" --status --socket "${runtime_socket}")"
python3 -c 'import json,sys; value=json.loads(sys.argv[1]); expected=sys.argv[2]; actual=value.get("build_revision"); raise SystemExit(0 if actual == expected else f"running build {actual!r} does not match {expected!r}")' "${status_json}" "${revision}"

echo "Preparing the powered character for the supervised smoke sequence..."
# Default startup may still be homing. Cancel that run explicitly before the
# foreground test; configure/enable only if startup did not already do so.
"${active_binary}" --stop --socket "${runtime_socket}"
current_status="$("${active_binary}" --status --socket "${runtime_socket}")"
current_mode="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["mode"])' "${current_status}")"
if [[ "${current_mode}" == "observe" ]]; then
  "${active_binary}" --configure --socket "${runtime_socket}"
  current_mode="configured"
fi
if [[ "${current_mode}" == "configured" ]]; then
  "${active_binary}" --enable --socket "${runtime_socket}"
fi
"${active_binary}" --goto zero_reference --duration 3.0 --wait --socket "${runtime_socket}"

echo "Testing Orion's RGBW shield and acknowledge cue..."
"${active_binary}" --run-scene deployment_smoke --wait --socket "${runtime_socket}"

echo "Testing both continuous expressive arcs and marker-synchronised expression..."
"${active_binary}" --run-scene acknowledge_left --wait --socket "${runtime_socket}"
"${active_binary}" --run-scene acknowledge_right --wait --socket "${runtime_socket}"
"${active_binary}" --run-scene return_home --wait --socket "${runtime_socket}"

echo "Returning Orion to rest, fading lights off, and disabling torque..."
"${active_binary}" --goto rest --duration 3.0 --wait --socket "${runtime_socket}"
"${active_binary}" --disable --socket "${runtime_socket}"

final_status="$("${active_binary}" --status --socket "${runtime_socket}")"
python3 -c 'import json,sys; value=json.loads(sys.argv[1]); assert value.get("torque_enabled") is False, value; assert value.get("mode") == "configured", value' "${final_status}"

sudo systemctl restart orion-studio-gateway.service
sudo systemctl restart orion-listener.service
sudo systemctl is-active --quiet orion-listener.service
sudo systemctl is-enabled --quiet orion-listener.service
# Verify the authenticated protocol with no real capture: a wrong token must be rejected.
"${project_root}/voice/.venv/bin/python" "${project_root}/scripts/check_pi_listener.py"
sudo systemctl is-active --quiet oriond.service
sudo systemctl is-active --quiet orion-studio-gateway.service
sudo systemctl is-enabled --quiet oriond.service
sudo systemctl is-enabled --quiet orion-studio-gateway.service

gateway_ready=false
for _ in {1..50}; do
  if python3 -c 'import socket; connection=socket.create_connection(("127.0.0.1", 7447), timeout=0.2); connection.close()' 2>/dev/null; then
    gateway_ready=true
    break
  fi
  sleep 0.1
done
if [[ "${gateway_ready}" != true ]]; then
  echo "The Studio gateway did not become reachable; inspect its journal." >&2
  exit 1
fi

trap - ERR INT TERM
echo "Deployment ${revision} passed. Orion is at rest, lights are off, and torque is disabled."
echo "oriond, the Studio gateway and the Rustpotter listener are enabled for boot."
echo "Studio gateway: http://orion.local:7447"
echo "Logs: journalctl -u oriond.service -u orion-studio-gateway.service -u orion-listener.service"
