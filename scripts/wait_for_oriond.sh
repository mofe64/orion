#!/usr/bin/env bash
set -euo pipefail

binary="${1:?oriond binary is required}"
socket="${2:-/tmp/oriond.sock}"
timeout_seconds="${3:-20}"

if [[ ! "${timeout_seconds}" =~ ^[0-9]+$ || "${timeout_seconds}" -lt 1 || "${timeout_seconds}" -gt 120 ]]; then
  echo "Invalid oriond wait timeout: ${timeout_seconds}" >&2
  exit 2
fi

for ((attempt = 0; attempt < timeout_seconds * 10; attempt++)); do
  if [[ -x "${binary}" ]] && "${binary}" --status --socket "${socket}" >/dev/null 2>&1; then
    exit 0
  fi
  sleep 0.1
done

echo "oriond did not become ready on ${socket} within ${timeout_seconds} seconds." >&2
exit 1
