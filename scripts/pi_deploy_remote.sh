#!/usr/bin/env bash
set -euo pipefail

# Extract the selected revision's deployment code, never the dirty checkout's copy.
extract_revision_scripts() {
  local revision=$1 destination=$2
  git archive "${revision}" scripts | tar -x -C "${destination}"
}

project_root="${1:?Pi project root is required}"
branch="${2:?Git branch is required}"
if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo "Refusing unsafe Pi project path" >&2; exit 2
fi
if [[ ! "${branch}" =~ ^[A-Za-z0-9._/-]+$ || "${branch}" == -* || "${branch}" == *".."* ]]; then
  echo "Refusing unsafe Git branch" >&2; exit 2
fi
export PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${PATH}"
cd "${project_root}"
# Fetch/validation/build failures happen before any service is stopped.
git fetch origin "+refs/heads/${branch}:refs/remotes/origin/${branch}"
revision="$(git rev-parse --verify "origin/${branch}^{commit}")"
bootstrap="$(mktemp -d /tmp/orion-release.XXXXXXXXXX)"
cleanup() { rm -rf -- "${bootstrap}"; }
trap cleanup EXIT
extract_revision_scripts "${revision}" "${bootstrap}"
python3 "${bootstrap}/scripts/deploy_pi_release.py" \
  --source "${project_root}" --revision "${revision}" --runtime-project "${project_root}"
