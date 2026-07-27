#!/usr/bin/bash
set -euo pipefail

readonly SERIAL_DEVICE="/dev/ttyS0"
readonly PAYLOAD_IMAGE="localhost/andromeda:v1"
readonly LAYER_HEADROOM_BYTES="$((512 * 1024 * 1024))"

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
printf 'embedded image digest: '
skopeo inspect --format '{{.Digest}}' \
    "containers-storage:${PAYLOAD_IMAGE}"

printf 'installer memory:\n'
grep --extended-regexp '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' \
    /proc/meminfo
printf 'installer temporary filesystems:\n'
findmnt --output TARGET,SOURCE,FSTYPE,SIZE,AVAIL /tmp /var/tmp || true
df --block-size=1 /tmp /var/tmp

payload_manifest="$(skopeo inspect --raw "containers-storage:${PAYLOAD_IMAGE}")"
max_compressed_layer_bytes="$(
    jq --raw-output '[.layers[]?.size] | max // 0' <<<"${payload_manifest}"
)"
payload_history="$(
    podman history --human=false --format json "${PAYLOAD_IMAGE}"
)"
max_uncompressed_layer_bytes="$(
    jq --raw-output '[.[].size // 0] | max // 0' <<<"${payload_history}"
)"
var_tmp_available_bytes="$(
    df --output=avail --block-size=1 /var/tmp | tail -n 1 | tr -d ' '
)"
required_var_tmp_bytes="$((max_uncompressed_layer_bytes + LAYER_HEADROOM_BYTES))"

printf 'largest compressed payload layer: %s bytes\n' \
    "${max_compressed_layer_bytes}"
printf 'largest uncompressed payload layer: %s bytes\n' \
    "${max_uncompressed_layer_bytes}"
printf 'required /var/tmp capacity with headroom: %s bytes\n' \
    "${required_var_tmp_bytes}"
printf 'available /var/tmp capacity: %s bytes\n' \
    "${var_tmp_available_bytes}"

if (( var_tmp_available_bytes < required_var_tmp_bytes )); then
    printf 'Insufficient /var/tmp capacity for the largest payload layer: '\
'need %s bytes, found %s bytes.\n' \
        "${required_var_tmp_bytes}" "${var_tmp_available_bytes}" >&2
    exit 1
fi

if [[ -c "${SERIAL_DEVICE}" ]]; then
    printf 'ANDROMEDA_INSTALLER_PREFLIGHT_OK payload=%s bootc=%s '\
'max_uncompressed_layer_bytes=%s var_tmp_available_bytes=%s\n' \
        "${PAYLOAD_IMAGE}" "$(bootc --version)" \
        "${max_uncompressed_layer_bytes}" "${var_tmp_available_bytes}" \
        >"${SERIAL_DEVICE}"
fi
