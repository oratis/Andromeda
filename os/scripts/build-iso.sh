#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly IMAGE_BUILDER_IMAGE="${IMAGE_BUILDER_IMAGE:-ghcr.io/osbuild/image-builder-cli@sha256:67f1c248cf18acbcf8f716ccf29ba5a9b65352b8c5c4996f745efd176c41ee5c}"
readonly PLATFORM_VARIANT=pc_x86_64
readonly PLATFORM_ARCHITECTURE=x86_64
readonly BOOT_PROVIDER=pc_uefi_shim
readonly HEP_ID=pc-mainline

# INSTALLER_DEFAULT selects the GRUB default entry of the built ISO:
#   0 (default) - interactive graphical installer; safe for humans.
#   1           - destructive automated CI install; boots and wipes the first
#                 disk after the GRUB timeout. CI only. The resulting ISO is
#                 named *-ci.iso so it can never be mistaken for the
#                 developer-facing artifact.
INSTALLER_DEFAULT="${INSTALLER_DEFAULT:-0}"
readonly INSTALLER_DEFAULT
case "${INSTALLER_DEFAULT}" in
    0)
        ISO_BASENAME=Andromeda-Developer-Preview-x86_64
        ;;
    1)
        ISO_BASENAME=Andromeda-Developer-Preview-x86_64-ci
        ;;
    *)
        printf 'INSTALLER_DEFAULT must be 0 or 1, got: %s\n' \
            "${INSTALLER_DEFAULT}" >&2
        exit 1
        ;;
esac
readonly ISO_BASENAME

if [[ "${EUID}" -eq 0 ]]; then
    engine=(podman)
else
    engine=(sudo podman)
fi
if [[ -n "${PODMAN_RUNTIME:-}" ]]; then
    engine+=(--runtime "${PODMAN_RUNTIME}")
fi

mkdir -p "${OUTPUT_DIR}"
OUTPUT_DIR="$(cd "${OUTPUT_DIR}" && pwd)"
readonly OUTPUT_DIR
rm -f \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso" \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso.sha256" \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.manifest.json" \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64-ci.iso" \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64-ci.iso.sha256" \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64-ci.manifest.json"

"${engine[@]}" build \
    --tag localhost/andromeda:v1 \
    --target payload \
    --build-arg IMAGE_REVISION=1 \
    --build-arg "PLATFORM_VARIANT=${PLATFORM_VARIANT}" \
    --build-arg "PLATFORM_ARCHITECTURE=${PLATFORM_ARCHITECTURE}" \
    --build-arg "BOOT_PROVIDER=${BOOT_PROVIDER}" \
    --build-arg "HEP_ID=${HEP_ID}" \
    --file "${REPOSITORY_ROOT}/os/Containerfile" \
    "${REPOSITORY_ROOT}"

"${engine[@]}" build \
    --tag localhost/andromeda:v2 \
    --target payload \
    --build-arg IMAGE_REVISION=2 \
    --build-arg "PLATFORM_VARIANT=${PLATFORM_VARIANT}" \
    --build-arg "PLATFORM_ARCHITECTURE=${PLATFORM_ARCHITECTURE}" \
    --build-arg "BOOT_PROVIDER=${BOOT_PROVIDER}" \
    --build-arg "HEP_ID=${HEP_ID}" \
    --file "${REPOSITORY_ROOT}/os/Containerfile" \
    "${REPOSITORY_ROOT}"

"${engine[@]}" history \
    --human=false \
    --format json \
    localhost/andromeda:v1 \
    | tee "${OUTPUT_DIR}/andromeda-v1-history.json"

"${engine[@]}" save \
    --format oci-archive \
    --output "${OUTPUT_DIR}/andromeda-v2.tar" \
    localhost/andromeda:v2

"${engine[@]}" build \
    --tag localhost/andromeda-installer:ci \
    --target installer \
    --build-arg IMAGE_REVISION=1 \
    --build-arg "INSTALLER_DEFAULT=${INSTALLER_DEFAULT}" \
    --build-arg "PLATFORM_VARIANT=${PLATFORM_VARIANT}" \
    --build-arg "PLATFORM_ARCHITECTURE=${PLATFORM_ARCHITECTURE}" \
    --build-arg "BOOT_PROVIDER=${BOOT_PROVIDER}" \
    --build-arg "HEP_ID=${HEP_ID}" \
    --file "${REPOSITORY_ROOT}/os/Containerfile" \
    "${REPOSITORY_ROOT}"

"${engine[@]}" run --rm --privileged \
    --volume /var/lib/containers/storage:/var/lib/containers/storage \
    --volume "${OUTPUT_DIR}:/output:Z" \
    "${IMAGE_BUILDER_IMAGE}" \
    build \
    --output-dir /output \
    --bootc-ref localhost/andromeda-installer:ci \
    --bootc-installer-payload-ref localhost/andromeda:v1 \
    --bootc-default-fs ext4 \
    bootc-generic-iso

shopt -s nullglob
built_isos=("${OUTPUT_DIR}"/*.iso)
shopt -u nullglob
if [[ "${#built_isos[@]}" -ne 1 ]]; then
    printf 'Expected exactly one image-builder ISO, found %d.\n' \
        "${#built_isos[@]}" >&2
    exit 1
fi
mv -f "${built_isos[0]}" "${OUTPUT_DIR}/${ISO_BASENAME}.iso"
(
    cd "${OUTPUT_DIR}"
    sha256sum "${ISO_BASENAME}.iso" \
        | tee "${ISO_BASENAME}.iso.sha256"
)

payload_digest="$(
    "${engine[@]}" image inspect \
        --format '{{.Digest}}' localhost/andromeda:v1
)"
test "${payload_digest}" != "<none>"
test -n "${payload_digest}"
iso_sha256="$(
    awk '{ print $1 }' \
        "${OUTPUT_DIR}/${ISO_BASENAME}.iso.sha256"
)"
jq --null-input \
    --arg variant "${PLATFORM_VARIANT}" \
    --arg architecture "${PLATFORM_ARCHITECTURE}" \
    --arg boot_provider "${BOOT_PROVIDER}" \
    --arg hep_id "${HEP_ID}" \
    --arg payload_ref "localhost/andromeda:v1" \
    --arg payload_digest "${payload_digest}" \
    --arg iso_name "${ISO_BASENAME}.iso" \
    --arg iso_sha256 "${iso_sha256}" \
    --arg installer_default "${INSTALLER_DEFAULT}" \
    '{schema_version: 1,
      variant: $variant,
      architecture: $architecture,
      boot_provider: $boot_provider,
      hep_id: $hep_id,
      installer_default: ($installer_default | tonumber),
      payload: {reference: $payload_ref, digest: $payload_digest},
      iso: {name: $iso_name, sha256: $iso_sha256}}' \
    | tee "${OUTPUT_DIR}/${ISO_BASENAME}.manifest.json"
