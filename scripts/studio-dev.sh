#!/usr/bin/env bash
set -euo pipefail
studio_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../orion_studio" && pwd)"
exec pnpm --dir "$studio_dir" tauri dev "$@"
