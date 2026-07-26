#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly IMAGE_BUILDER_IMAGE="${IMAGE_BUILDER_IMAGE:-ghcr.io/osbuild/image-builder-cli:latest}"

if [[ "${EUID}" -eq 0 ]]; then
    engine=(podman)
else
    engine=(sudo podman)
fi

mkdir -p "${OUTPUT_DIR}"
OUTPUT_DIR="$(cd "${OUTPUT_DIR}" && pwd)"
readonly OUTPUT_DIR
rm -f \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso" \
    "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso.sha256"

"${engine[@]}" build \
    --tag localhost/andromeda:v1 \
    --target payload \
    --build-arg IMAGE_REVISION=1 \
    --file "${REPOSITORY_ROOT}/os/Containerfile" \
    "${REPOSITORY_ROOT}"

"${engine[@]}" build \
    --tag localhost/andromeda:v2 \
    --target payload \
    --build-arg IMAGE_REVISION=2 \
    --file "${REPOSITORY_ROOT}/os/Containerfile" \
    "${REPOSITORY_ROOT}"

"${engine[@]}" save \
    --format oci-archive \
    --output "${OUTPUT_DIR}/andromeda-v2.tar" \
    localhost/andromeda:v2

"${engine[@]}" build \
    --tag localhost/andromeda-installer:ci \
    --target installer \
    --build-arg IMAGE_REVISION=1 \
    --build-arg INSTALLER_DEFAULT=1 \
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
mv -f "${built_isos[0]}" "${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso"
(
    cd "${OUTPUT_DIR}"
    sha256sum Andromeda-Developer-Preview-x86_64.iso \
        > Andromeda-Developer-Preview-x86_64.iso.sha256
)
