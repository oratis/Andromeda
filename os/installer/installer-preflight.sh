#!/usr/bin/bash
set -euo pipefail

readonly SERIAL_DEVICE="/dev/ttyS0"
readonly PAYLOAD_IMAGE="localhost/andromeda:v1"
readonly PLATFORM_FILE="/usr/lib/andromeda/platform.json"
readonly INSTALL_MODE="${1:-interactive}"

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
payload_inspect="$(
    skopeo inspect "containers-storage:${PAYLOAD_IMAGE}"
)"
printf 'embedded image digest: '
printf '%s\n' "${payload_inspect}" | jq --raw-output '.Digest'

printf 'platform compatibility:\n'
/usr/libexec/andromeda-check-platform-compatibility \
    "${PLATFORM_FILE}" "${INSTALL_MODE}"
platform_variant="$(jq --raw-output '.variant' "${PLATFORM_FILE}")"
platform_architecture="$(jq --raw-output '.architecture' "${PLATFORM_FILE}")"
platform_boot_provider="$(jq --raw-output '.boot_provider' "${PLATFORM_FILE}")"
platform_hep_id="$(jq --raw-output '.hep_id' "${PLATFORM_FILE}")"
payload_variant="$(
    jq --raw-output '.Labels["io.andromeda.platform.variant"]' \
        <<<"${payload_inspect}"
)"
payload_architecture="$(
    jq --raw-output '.Labels["io.andromeda.platform.architecture"]' \
        <<<"${payload_inspect}"
)"
payload_boot_provider="$(
    jq --raw-output '.Labels["io.andromeda.platform.boot-provider"]' \
        <<<"${payload_inspect}"
)"
payload_hep_id="$(
    jq --raw-output '.Labels["io.andromeda.platform.hep-id"]' \
        <<<"${payload_inspect}"
)"
if [[ "${payload_variant}" != "${platform_variant}" \
    || "${payload_architecture}" != "${platform_architecture}" \
    || "${payload_boot_provider}" != "${platform_boot_provider}" \
    || "${payload_hep_id}" != "${platform_hep_id}" ]]; then
    printf 'ANDROMEDA_INSTALLER_PREFLIGHT_FAILED reason=payload_platform_mismatch\n'
    exit 1
fi
printf 'embedded platform identity: %s %s %s %s\n' \
    "${payload_variant}" "${payload_architecture}" \
    "${payload_boot_provider}" "${payload_hep_id}"

printf 'installer resources:\n'
grep --extended-regexp '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' \
    /proc/meminfo || true
df --human-readable /tmp /var/tmp || true

printf 'ANDROMEDA_INSTALLER_PREFLIGHT_OK payload=%s platform=%s mode=%s bootc=%s\n' \
    "${PAYLOAD_IMAGE}" "${platform_variant}" "${INSTALL_MODE}" "$(bootc --version)"
