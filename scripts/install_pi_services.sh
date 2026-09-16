#!/usr/bin/env bash
set -euo pipefail

# Keep the old entry point, but never overwrite installed units with base templates.
project_root="${1:?Pi project root is required}"
orion_user="${2:?Pi service user is required}"
user_home="${3:?Pi service user home is required}"
if [[ "${orion_user}" != "$(id -un)" || "${user_home}" != "${HOME}" ]]; then
  echo 'Run as the Pi service user, using their home directory; do not run with sudo.' >&2
  exit 2
fi
export PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${PATH}"
exec python3 "$(dirname -- "${BASH_SOURCE[0]}")/deploy_pi_release.py" \
  --source "${project_root}" --revision HEAD --runtime-project "${project_root}"
