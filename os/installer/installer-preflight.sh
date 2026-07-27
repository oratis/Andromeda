#!/usr/bin/bash
set -euo pipefail

readonly SERIAL_DEVICE="/dev/ttyS0"
readonly PAYLOAD_IMAGE="localhost/andromeda:v1"

if [[ -c "${SERIAL_DEVICE}" ]]; then
    printf 'ANDROMEDA_INSTALLER_PREFLIGHT_START payload=%s\n' \
        "${PAYLOAD_IMAGE}" >"${SERIAL_DEVICE}"
fi

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

if [[ -c "${SERIAL_DEVICE}" ]]; then
    printf 'ANDROMEDA_INSTALLER_PREFLIGHT_OK payload=%s bootc=%s\n' \
        "${PAYLOAD_IMAGE}" "$(bootc --version)" >"${SERIAL_DEVICE}"
fi
