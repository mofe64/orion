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
trap 'rm -rf "$build_stage"' EXIT
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
    --mount "type=bind,source=$output,target=/out" \
    --mount type=volume,source=orion-release-build-cache,target=/cache \
    -e "ORION_RELEASE_VERSION=$version" -e "ORION_BUILD_REVISION=$commit" -e "ORION_SOURCE_DIRTY=$dirty" \
    "orion-release:$codename" python /src/scripts/release_build.py --codename "$codename"
done
# Packaging runs in Bookworm as well; no host Python is needed.
docker run --rm --platform linux/arm64 --mount "type=bind,source=$build_stage,target=/src" \
  --mount "type=bind,source=$output,target=/out" -e "ORION_RELEASE_VERSION=$version" \
  -e "ORION_BUILD_REVISION=$commit" orion-release:bookworm python /src/scripts/release_build.py --pack
