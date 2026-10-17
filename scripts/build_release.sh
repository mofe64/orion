#!/usr/bin/env bash
# One entry point for local Docker and GitHub ARM runners.
set -euo pipefail
version=${1:-}
if [[ ! $version =~ ^v[0-9][A-Za-z0-9._-]*$ ]]; then
  echo 'Usage: scripts/build_release.sh vVERSION [OUTPUT_DIRECTORY]' >&2; exit 2
fi
checkout=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
output=${2:-$checkout/artifacts}
mkdir -p "$output"
output=$(cd "$output" && pwd)
for codename in bookworm trixie; do
  docker build --platform linux/arm64 --build-arg "CODENAME=$codename" \
    -f "$checkout/scripts/Release.Dockerfile" -t "orion-release:$codename" "$checkout/scripts"
done
build_stage=$(mktemp -d)
cache=${ORION_RELEASE_CACHE:-$checkout/.release-cache}
mkdir -p "$cache"
cache=$(cd "$cache" && pwd)
payload_volume=
build_owner="$(id -u):$(id -g)"
cleanup() {
  build_status=$?
  trap - EXIT
  # Container-created venvs belong to root on Linux bind mounts. Restore only
  # the disposable snapshot's owner; -h never follows its compiler symlinks.
  if docker run --rm --mount "type=bind,source=$build_stage,target=/cleanup" \
      orion-release:bookworm chown -Rh "$build_owner" /cleanup; then
    rm -rf "$build_stage" || build_status=1
  else
    echo "Could not restore ownership of temporary source: $build_stage" >&2
    build_status=1
  fi
  if [[ -n $payload_volume ]]; then docker volume rm "$payload_volume" >/dev/null || build_status=1; fi
  exit "$build_status"
}
trap cleanup EXIT
# Linux distributions contain case-distinct names (including Python terminfo).
# Keep the payload on Linux; export only the final archive to the host.
payload_volume=$(docker volume create)
# Include tracked files and non-ignored working changes for local development.
# Git's inventory keeps ignored credentials, recordings and caches out of Docker.
git -C "$checkout" ls-files --cached --others --exclude-standard -z > "$build_stage/inventory"
while IFS= read -r -d '' path; do
  if [[ -f $checkout/$path || -L $checkout/$path ]]; then printf '%s\0' "$path"; fi
done < "$build_stage/inventory" > "$build_stage/source-files"
tar -C "$checkout" --null --no-recursion -T "$build_stage/source-files" -cf - | tar -C "$build_stage" -xf -
rm "$build_stage/inventory" "$build_stage/source-files"
commit=$(git -C "$checkout" rev-parse HEAD)
dirty=false
if [[ -n $(git -C "$checkout" status --porcelain) ]]; then dirty=true; fi
for codename in bookworm trixie; do
  docker run --rm --platform linux/arm64 \
    --mount "type=bind,source=$build_stage,target=/src" \
    --mount "type=volume,source=$payload_volume,target=/out" \
    --mount "type=bind,source=$cache,target=/cache" \
    -e "ORION_RELEASE_VERSION=$version" -e "ORION_BUILD_REVISION=$commit" -e "ORION_SOURCE_DIRTY=$dirty" \
    "orion-release:$codename" python /src/scripts/release_build.py --codename "$codename"
done
# Packaging runs in Bookworm as well; no host Python is needed.
docker run --rm --platform linux/arm64 --mount "type=bind,source=$build_stage,target=/src" \
  --mount "type=volume,source=$payload_volume,target=/out" \
  --mount "type=bind,source=$output,target=/artifacts" -e ORION_ARTIFACT_DIR=/artifacts -e "ORION_RELEASE_VERSION=$version" \
  -e "ORION_BUILD_REVISION=$commit" orion-release:bookworm python /src/scripts/release_build.py --pack
