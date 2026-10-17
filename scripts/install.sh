#!/usr/bin/env bash
# Bootstrap without invoking the OS Python. Safe to pipe to bash.
set -euo pipefail
version=latest
bundle=
action=install
home_assistant=false
ha_update=false
root=${ORION_INSTALL_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/orion/voice-stack}
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) version=${2:?--version requires vVERSION}; shift 2 ;;
    --from) bundle=${2:?--from requires a tarball}; shift 2 ;;
    --home-assistant) home_assistant=true; shift ;;
    --update) ha_update=true; shift ;;
    --assets) action=assets; shift ;;
    --rollback) action=rollback; shift ;;
    --check) action=check; shift ;;
    --help|-h) echo 'Usage: install.sh [--assets] [--version TAG | --from FILE | --rollback | --check] | --home-assistant [--update]'; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done
if [[ $EUID -eq 0 ]]; then
  echo 'Run as the Pi login user. The installer requests sudo for system packages and units.' >&2; exit 1
fi
# 64-bit Raspberry Pi OS reports ID=debian on ARM64; /etc/rpi-issue
# distinguishes it from generic Debian without relying on the OS Python.
if [[ ! -r /etc/os-release || ! -f /etc/rpi-issue ]]; then
  echo 'Requires Raspberry Pi OS Lite 64-bit Bookworm or Trixie.' >&2; exit 1
fi
. /etc/os-release
if [[ $(uname -m) != aarch64 || ${ID:-} != debian && ${ID:-} != raspbian \
   || ${VERSION_CODENAME:-} != bookworm && ${VERSION_CODENAME:-} != trixie ]]; then
  echo "Unsupported OS: ${PRETTY_NAME:-unknown}, $(uname -m). Requires Raspberry Pi OS Bookworm/Trixie ARM64." >&2; exit 1
fi
python=/opt/orion/python/bin/python3.12
if [[ $home_assistant == true ]]; then
  [[ $action == install && -z $bundle ]] || { echo '--home-assistant must run as a separate step.' >&2; exit 2; }
  source_dir=$(cd "$(dirname "${BASH_SOURCE[0]:-./install.sh}")" 2>/dev/null && pwd)
  if [[ -x $python && -f $root/installation.json ]]; then
    release=$($python -c 'import json,sys; print(json.load(open(sys.argv[1]))["release"])' "$root/installation.json")
    source_dir=$release/scripts
    if [[ ! -f $source_dir/release-home-assistant.json ]]; then
      echo 'This Orion release cannot install Home Assistant. Update Orion first, then run this step again.' >&2; exit 1
    fi
  elif [[ -f $source_dir/release_install.py ]]; then
    python=/usr/bin/python3
  else
    echo 'Install Orion first, or run this step from its scripts directory.' >&2; exit 1
  fi
  args=(--home-assistant --root "$root")
  if [[ $ha_update == true ]]; then args+=(--update); fi
  exec "$python" "$source_dir/release_install.py" "${args[@]}"
fi
[[ $ha_update == false ]] || { echo '--update requires --home-assistant.' >&2; exit 2; }
if [[ $action != install ]]; then
  [[ -x $python && -f $root/installation.json ]] || { echo 'No installed Orion release to check or roll back.' >&2; exit 1; }
  release=$($python -c 'import json,sys; print(json.load(open(sys.argv[1]))["release"])' "$root/installation.json")
  args=("--$action" --root "$root")
  if [[ $action == assets && $version != latest ]]; then args+=(--version "$version"); fi
  exec "$python" "$release/scripts/release_install.py" "${args[@]}"
fi
if [[ $version != latest && ! $version =~ ^v[0-9][A-Za-z0-9._-]*$ ]]; then
  echo 'Invalid release version. Use vX.Y.Z.' >&2; exit 2
fi
sudo -v
sudo apt-get update
sudo apt-get install -y --no-install-recommends ca-certificates curl jq pciutils xz-utils
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
if [[ -z $bundle ]]; then
  if [[ $version == latest ]]; then
    version=$(curl --fail --silent --show-error --location --retry 3 \
      'https://api.github.com/repos/mofe64/orion/releases?per_page=100' | jq -er '[.[] | select(.draft == false and .prerelease == false and (.tag_name | test("^v[0-9]+\\.[0-9]+\\.[0-9]+$")))][0].tag_name // error("No stable app release")' )
  fi
  [[ $version =~ ^v[0-9][A-Za-z0-9._-]*$ ]] || { echo 'Invalid release tag returned by GitHub' >&2; exit 1; }
  name="orion-${version#v}-arm64.tar.gz"
  url="https://github.com/mofe64/orion/releases/download/$version/$name"
  curl --fail --silent --show-error --location --retry 3 "$url" -o "$stage/$name"
  curl --fail --silent --show-error --location --retry 3 "$url.sha256" -o "$stage/$name.sha256"
  bundle=$stage/$name
fi
[[ -f $bundle && -f $bundle.sha256 ]] || { echo 'Tarball and adjacent .sha256 file are required.' >&2; exit 1; }
bundle=$(cd "$(dirname "$bundle")" && pwd)/$(basename "$bundle")
expected=$(awk 'NR==1 {print $1}' "$bundle.sha256")
[[ $expected =~ ^[a-f0-9]{64}$ ]] || { echo 'Invalid archive checksum' >&2; exit 1; }
printf '%s  %s\n' "$expected" "$bundle" | sha256sum --check --status || { echo 'Release checksum mismatch; nothing unpacked.' >&2; exit 1; }
# The release archive contains regular files/directories only, including a
# dereferenced portable interpreter. Reject links and traversal before bootstrap.
tar -tzf "$bundle" > "$stage/paths"
if grep -Eq '(^/|(^|/)\.\.(/|$))' "$stage/paths" \
    || tar -tvzf "$bundle" | awk 'substr($0,1,1)!="-" && substr($0,1,1)!="d" {bad=1} END {exit !bad}'; then
  echo 'Unsafe release archive path or file type' >&2; exit 1
fi
mkdir "$stage/payload"
tar -xzf "$bundle" -C "$stage/payload" --no-same-owner --no-same-permissions
(cd "$stage/payload" && sha256sum --check --quiet SHA256SUMS)
bootstrap=$stage/payload/python/bin/python3.12
[[ -x $bootstrap ]] || { echo 'Release is missing managed Python 3.12' >&2; exit 1; }
exec_status=0
# Keep module imports from adding bytecode to the checksummed payload.
PYTHONDONTWRITEBYTECODE=1 "$bootstrap" -B "$stage/payload/release/scripts/release_install.py" --payload "$stage/payload" \
  --codename "$VERSION_CODENAME" --root "$root" || exec_status=$?
exit "$exec_status"
