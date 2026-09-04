#!/usr/bin/env bash
set -euo pipefail

project_root="${1:?Pi project root is required}"
orion_user="${2:?Pi service user is required}"
user_home="${3:?Pi service user home is required}"

if [[ ! "${project_root}" =~ ^/[A-Za-z0-9._/-]+$ || "${project_root}" == *".."* ]]; then
  echo "Refusing unsafe Pi project path: ${project_root}" >&2
  exit 2
fi
if [[ ! "${user_home}" =~ ^/[A-Za-z0-9._/-]+$ || "${user_home}" == *".."* ]]; then
  echo "Refusing unsafe Pi home path: ${user_home}" >&2
  exit 2
fi
if [[ ! "${orion_user}" =~ ^[A-Za-z_][A-Za-z0-9_-]*$ ]]; then
  echo "Refusing unsafe Pi user: ${orion_user}" >&2
  exit 2
fi

for command in sed install systemctl sudo mktemp; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "Required service installer command is unavailable: ${command}" >&2
    exit 1
  fi
done
sudo -n true

template_directory="${project_root}/scripts/systemd"
temporary_directory="$(mktemp -d)"
cleanup() {
  rm -r -- "${temporary_directory}"
}
trap cleanup EXIT

for service in oriond orion-studio-gateway orion-listener; do
  template="${template_directory}/${service}.service.in"
  output="${temporary_directory}/${service}.service"
  if [[ ! -f "${template}" ]]; then
    echo "Missing Orion service template: ${template}" >&2
    exit 1
  fi
  sed \
    -e "s|@PROJECT_ROOT@|${project_root}|g" \
    -e "s|@ORION_USER@|${orion_user}|g" \
    -e "s|@USER_HOME@|${user_home}|g" \
    "${template}" > "${output}"
  sudo -n install -o root -g root -m 0644 "${output}" "/etc/systemd/system/${service}.service"
done

sudo -n systemctl daemon-reload
sudo -n systemctl enable oriond.service orion-studio-gateway.service
if [[ -x "${project_root}/voice/.venv/bin/orion-listener" && -f "${user_home}/.config/orion/studio-token" ]]; then
  sudo -n systemctl enable orion-listener.service
fi
