#!/usr/bin/env bash
set -euo pipefail

project_root="${1:?Orion project root is required}"
archive_root="${2:-${HOME}/.local/share/orion/backups}"

if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo "Refusing unsafe Orion project path: ${project_root}" >&2
  exit 2
fi
if [[ ! "${archive_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${archive_root}" == *".."* ]]; then
  echo "Refusing unsafe Orion archive path: ${archive_root}" >&2
  exit 2
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
archive="${archive_root}/user-assets-pre-v2-${stamp}"
moved=0

for relative_directory in scenes/v1/user scenes/v2/user motion/user/poses/v1 motion/user/poses/v2 motion/motions/v1/user motion/motions/v2/user; do
  source_directory="${project_root}/${relative_directory}"
  [[ -d "${source_directory}" ]] || continue
  while IFS= read -r -d '' asset; do
    if grep -Eq '^[[:space:]]*format_version:[[:space:]]*2([[:space:]]|$)' "${asset}"; then
      continue
    fi
    relative_path="${asset#"${project_root}/"}"
    destination="${archive}/${relative_path}"
    mkdir -p "$(dirname "${destination}")"
    mv -- "${asset}" "${destination}"
    moved=$((moved + 1))
  done < <(find "${source_directory}" -maxdepth 1 -type f \( -name '*.yaml' -o -name '*.yml' \) -print0)
done

if (( moved == 0 )); then
  echo "No legacy or malformed Orion user assets required migration."
  exit 0
fi

{
  echo "Orion v2 breaking-release user asset archive"
  echo "created_utc=${stamp}"
  echo "source=${project_root}"
  echo "files=${moved}"
} > "${archive}/MANIFEST.txt"

echo "Archived ${moved} non-v2 user asset(s) to ${archive}."
