#!/usr/bin/env bash
set -euo pipefail
host=${ORION_PI_HOST:-}
bundle=
version=latest
while [[ $# -gt 0 ]]; do
  case $1 in
    --host) host=${2:?}; shift 2 ;;
    --from) bundle=${2:?}; shift 2 ;;
    --version) version=${2:?}; shift 2 ;;
    --help|-h) echo 'Usage: scripts/deploy_pi.sh --host USER@HOST [--version vX.Y.Z | --from FILE]'; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done
[[ $host =~ ^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+$ ]] || { echo 'Pass --host USER@HOST' >&2; exit 2; }
[[ $version == latest || $version =~ ^v[0-9][A-Za-z0-9._-]*$ ]] || { echo 'Invalid release version' >&2; exit 2; }
if [[ -n $bundle ]]; then
  [[ -f $bundle && -f $bundle.sha256 ]] || { echo 'Bundle and .sha256 file required' >&2; exit 1; }
  remote=$(ssh "$host" mktemp -d /tmp/orion-upload.XXXXXXXX)
  [[ $remote =~ ^/tmp/orion-upload\.[A-Za-z0-9]+$ ]] || exit 1
  trap 'ssh "$host" rm -rf -- "$remote"' EXIT
  scp "$bundle" "$host:$remote/release.tar.gz"
  scp "$bundle.sha256" "$host:$remote/release.tar.gz.sha256"
  scp "$(dirname "$0")/install.sh" "$host:$remote/install.sh"
  ssh -t "$host" "bash '$remote/install.sh' --from '$remote/release.tar.gz'"
else
  ssh -t "$host" "curl -fsSL https://raw.githubusercontent.com/mofe64/orion/main/scripts/install.sh | bash -s -- --version '$version'"
fi
