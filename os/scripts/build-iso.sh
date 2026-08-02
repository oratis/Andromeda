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

# ---------------------------------------------------------------------------
# Cross-run payload layer cache (docs/reviews/e2e-pipeline-review.md P1 #6).
#
# Measured on run 30694919736: the 23 payload dnf layers cost 11.1 min and the
# installer stage another 2.9 min, and BOTH are re-spent from zero on every CI
# run. The 73 `Using cache` lines in a build log are all intra-run (v1 -> v2,
# v1 -> installer); a fresh runner has an empty store, so the cross-run hit rate
# is exactly zero. Pointing --cache-from/--cache-to at a registry repository
# lets those layers survive between runs.
#
# Disabled unless ANDROMEDA_PAYLOAD_CACHE_REPO is set, so a developer build and
# the GCP path behave exactly as before with no registry involved.
#
#   ANDROMEDA_PAYLOAD_CACHE_REPO  base repo, e.g. ghcr.io/oratis/andromeda-payload-cache
#                                 payload-cache-key.sh appends the ISO week
#   ANDROMEDA_PAYLOAD_CACHE_PUSH  1 to also publish cache (needs a write token)
#   ANDROMEDA_PAYLOAD_CACHE_TTL   how stale a reusable layer may be (default 7d)
#
# The caller must have logged the ENGINE's identity in to the registry already
# (`sudo podman login` when this script runs under sudo, since rootful podman
# reads root's auth store, not the invoking user's).
# ---------------------------------------------------------------------------
readonly PAYLOAD_CACHE_BASE_REPO="${ANDROMEDA_PAYLOAD_CACHE_REPO:-}"
readonly PAYLOAD_CACHE_PUSH="${ANDROMEDA_PAYLOAD_CACHE_PUSH:-0}"
# 168h = 7 days. Second, finer-grained half of the mandatory time bound; the
# other half is the ISO-week repository rotation performed by
# payload-cache-key.sh. Both exist because the payload installs ~300 packages
# from a rolling Fedora repo: a cache with no expiry would freeze that package
# set and silently stop security updates from reaching the image. See the long
# rationale at the top of payload-cache-key.sh.
readonly PAYLOAD_CACHE_TTL="${ANDROMEDA_PAYLOAD_CACHE_TTL:-168h}"

cache_build_args=()
if [[ -n "${PAYLOAD_CACHE_BASE_REPO}" ]]; then
    cache_identity="$(
        ANDROMEDA_PAYLOAD_CACHE_REPO="${PAYLOAD_CACHE_BASE_REPO}" \
            "${REPOSITORY_ROOT}/os/scripts/payload-cache-key.sh" \
            --print repo,ref,content-hash
    )"
    read -r payload_cache_repo payload_cache_ref payload_cache_hash \
        <<< "${cache_identity}"

    # Probe the flags rather than assuming a podman version. --cache-from /
    # --cache-to / --cache-ttl landed together in podman 4.5 (buildah 1.30) and
    # ubuntu-latest is well past that, but this script also runs on Fedora hosts
    # and on whatever ubuntu-latest becomes next; an unsupported flag must
    # degrade to an uncached build, never to a hard failure.
    build_help="$("${engine[@]}" build --help 2>&1 || true)"
    if grep -q -- '--cache-to' <<< "${build_help}"; then
        # A REPOSITORY, deliberately untagged: buildah trims any tag handed to
        # --cache-from/--cache-to and applies its own per-layer cache-key tags.
        cache_build_args+=(--cache-from "${payload_cache_repo}")
        if [[ "${PAYLOAD_CACHE_PUSH}" == "1" ]]; then
            cache_build_args+=(--cache-to "${payload_cache_repo}")
        fi
        if grep -q -- '--cache-ttl' <<< "${build_help}"; then
            cache_build_args+=(--cache-ttl "${PAYLOAD_CACHE_TTL}")
        fi
        printf 'ANDROMEDA_PAYLOAD_CACHE mode=layers repo=%s ref=%s push=%s ttl=%s\n' \
            "${payload_cache_repo}" "${payload_cache_ref}" \
            "${PAYLOAD_CACHE_PUSH}" "${PAYLOAD_CACHE_TTL}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_CONTENT_HASH=%s\n' "${payload_cache_hash}"
    else
        printf 'ANDROMEDA_PAYLOAD_CACHE mode=disabled reason=no-cache-to-flag\n'
        printf 'WARNING: this podman has no --cache-to; building without the cross-run layer cache.\n' >&2
    fi
else
    printf 'ANDROMEDA_PAYLOAD_CACHE mode=disabled reason=no-repo-configured\n'
fi

# run_cached_build <description> <podman build args...>
#
# A cache problem must never be able to fail the build. If the cached attempt
# fails for ANY reason -- unreachable registry, missing package, an auth token
# that turned out to be read-only, a buildah cache bug -- the cache is disabled
# for the remainder of this script and the identical build is retried once,
# cold. That costs one extra build attempt on a genuinely broken build, but the
# retry is also a clean cache-free reproduction, which is what a human wants to
# look at anyway. The banner is deliberately loud so a failure that only the
# retry survived is never mistaken for a healthy run.
run_cached_build() {
    local description="$1"
    shift

    if (( ${#cache_build_args[@]} == 0 )); then
        "${engine[@]}" build "$@"
        return
    fi

    if "${engine[@]}" build "${cache_build_args[@]}" "$@"; then
        return 0
    fi

    printf '\n' >&2
    printf '================================================================\n' >&2
    printf 'PAYLOAD CACHE FALLBACK: %s failed with the cross-run layer cache attached.\n' \
        "${description}" >&2
    printf 'Retrying the identical build with no cache. If this retry succeeds the\n' >&2
    printf 'cache was at fault; if it fails too, the build itself is broken.\n' >&2
    printf '================================================================\n' >&2
    cache_build_args=()
    "${engine[@]}" build "$@"
}

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

run_cached_build 'payload v1' \
    --tag localhost/andromeda:v1 \
    --target payload \
    --build-arg IMAGE_REVISION=1 \
    --build-arg "PLATFORM_VARIANT=${PLATFORM_VARIANT}" \
    --build-arg "PLATFORM_ARCHITECTURE=${PLATFORM_ARCHITECTURE}" \
    --build-arg "BOOT_PROVIDER=${BOOT_PROVIDER}" \
    --build-arg "HEP_ID=${HEP_ID}" \
    --file "${REPOSITORY_ROOT}/os/Containerfile" \
    "${REPOSITORY_ROOT}"

run_cached_build 'payload v2' \
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

run_cached_build 'installer stage' \
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
