#!/usr/bin/env bash
set -euo pipefail

binary="${1:?oriond binary is required}"
socket="${2:-/tmp/oriond.sock}"

if [[ ! -x "${binary}" || ! -S "${socket}" ]]; then
  exit 0
fi

status="$(${binary} --status --socket "${socket}")"
torque_enabled="$(python3 -c 'import json,sys; print(str(json.loads(sys.argv[1]).get("torque_enabled", False)).lower())' "${status}")"
if [[ "${torque_enabled}" != "true" ]]; then
  exit 0
fi

"${binary}" --stop-scene --socket "${socket}" >/dev/null 2>&1 || true
"${binary}" --stop-speech --socket "${socket}" >/dev/null 2>&1 || true
"${binary}" --stop --socket "${socket}" >/dev/null 2>&1 || true

if ! "${binary}" --goto rest --duration 3.0 --wait --socket "${socket}"; then
  echo "Orion could not confirm mechanical rest before service shutdown." >&2
  exit 1
fi
"${binary}" --disable --socket "${socket}"
