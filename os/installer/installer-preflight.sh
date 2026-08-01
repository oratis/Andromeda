#!/usr/bin/bash
set -euo pipefail

readonly SERIAL_DEVICE="/dev/ttyS0"
readonly PAYLOAD_IMAGE="localhost/andromeda:v1"

if [[ -c "${SERIAL_DEVICE}" ]]; then
    exec > >(tee -a "${SERIAL_DEVICE}") 2>&1
fi

preflight_status=0
trap 'preflight_status="$?"
    if (( preflight_status != 0 )); then
        printf "ANDROMEDA_INSTALLER_PREFLIGHT_FAILED status=%s\n" \
            "${preflight_status}"
    fi' EXIT

printf 'ANDROMEDA_INSTALLER_PREFLIGHT_START payload=%s\n' \
    "${PAYLOAD_IMAGE}"

printf 'UEFI environment: '
test -d /sys/firmware/efi
printf 'present\n'

printf 'bootc: '
command -v bootc
bootc --version

printf 'podman: '
command -v podman
podman --version
printf 'embedded images:\n'
podman images --format '{{.Repository}}:{{.Tag}} {{.ID}}'

podman image exists "${PAYLOAD_IMAGE}"
printf 'embedded image digest: '
skopeo inspect --format '{{.Digest}}' \
    "containers-storage:${PAYLOAD_IMAGE}"

printf 'installer resources:\n'
grep --extended-regexp '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' \
    /proc/meminfo || true
df --human-readable /tmp /var/tmp || true

printf 'ANDROMEDA_INSTALLER_PREFLIGHT_OK payload=%s bootc=%s\n' \
    "${PAYLOAD_IMAGE}" "$(bootc --version)"
