#!/usr/bin/env bash
set -euo pipefail

project_root="${1:?Pi project root is required}"
bundle="${2:?Prepared release bundle is required}"
digest="${3:?Bundle checksum is required}"
prepare_only="${4:-false}"
hardware="${5:?Select hardware v1 or v2}"
if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo 'Refusing unsafe Pi project path' >&2; exit 2
fi
if [[ ! "${bundle}" =~ ^/tmp/orion-deploy\.[A-Za-z0-9]+/release\.tar\.gz$ \
      || ! "${digest}" =~ ^[0-9a-f]{64}$ \
      || ( "${hardware}" != v1 && "${hardware}" != v2 ) \
      || ( "${prepare_only}" != true && "${prepare_only}" != false ) ]]; then
  echo 'Invalid release bundle arguments' >&2; exit 2
fi
export PATH="${HOME}/.local/bin:${PATH}"
bootstrap="$(mktemp -d /tmp/orion-release.XXXXXXXXXX)"
trap 'rm -rf -- "${bootstrap}"' EXIT
# Verify the upload before executing its committed receiver. The receiver then
# validates all archive paths, checks platform compatibility and unpacks it.
/usr/bin/python3 - "${bundle}" "${digest}" "${bootstrap}" <<'PY'
import hashlib
from pathlib import Path, PurePosixPath
import sys
import tarfile
bundle, expected, destination = sys.argv[1:]
with open(bundle, 'rb') as source:
    if hashlib.file_digest(source, 'sha256').hexdigest() != expected:
        raise RuntimeError('Transferred release checksum does not match')
with tarfile.open(bundle) as archive:
    for member in archive.getmembers():
        path = PurePosixPath(member.name)
        if path.parts[:2] == ('release', 'scripts') and path.suffix == '.py' and len(path.parts) == 3:
            if not member.isfile() or path.is_absolute() or '..' in path.parts:
                raise RuntimeError('Unsafe deployment helper in bundle')
            target = Path(destination) / path.name
            target.write_bytes(archive.extractfile(member).read())
PY
log_directory="${HOME}/.local/share/orion/voice-stack/logs"
mkdir -p "${log_directory}"
log="${log_directory}/deploy-$(date +%Y%m%d-%H%M%S)-${digest:0:12}.log"
release_args=(--hardware "${hardware}" --bundle "${bundle}" --sha256 "${digest}" --runtime-project "${project_root}")
if [[ "${prepare_only}" == true ]]; then release_args+=(--prepare-only); fi
set +e
/usr/bin/python3 "${bootstrap}/deploy_pi_release.py" "${release_args[@]}" 2>&1 | tee "${log}"
status=${PIPESTATUS[0]}
set -e
echo "Deployment log: ${log}"
exit "${status}"
