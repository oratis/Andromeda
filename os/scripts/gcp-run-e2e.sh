#!/usr/bin/env bash
set -Eeuo pipefail

# Thin, auditable gcloud lifecycle wrapper for the nested-KVM daily-driver
# E2E. It creates exactly one disposable, labeled Compute Engine instance
# with a hard --max-run-duration cap, runs os/scripts/test-gcp-nested.sh on
# it, downloads the evidence directory, and always deletes the instance on
# exit. Even if this script is killed, the platform enforces the run
# duration cap and terminates (deletes) the instance.
#
# Requirements: an authenticated gcloud CLI with compute permissions, and a
# project that allows N2 instances with nested virtualization.
#
# Usage:
#   ANDROMEDA_GCP_PROJECT=my-project os/scripts/gcp-run-e2e.sh [evidence-dir]

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly EVIDENCE_DIR="${1:-${REPOSITORY_ROOT}/output/gcp-evidence}"
readonly PROJECT="${ANDROMEDA_GCP_PROJECT:?set ANDROMEDA_GCP_PROJECT to the Google Cloud project id}"
readonly ZONE="${ANDROMEDA_GCP_ZONE:-us-central1-a}"
readonly MACHINE_TYPE="${ANDROMEDA_GCP_MACHINE_TYPE:-n2-standard-16}"
# Hard platform-side lifetime cap; the instance is deleted when it expires.
readonly MAX_RUN_DURATION="${ANDROMEDA_GCP_MAX_RUN_DURATION:-6h}"
readonly IMAGE_FAMILY="${ANDROMEDA_GCP_IMAGE_FAMILY:-ubuntu-2404-lts-amd64}"
readonly IMAGE_PROJECT="${ANDROMEDA_GCP_IMAGE_PROJECT:-ubuntu-os-cloud}"
readonly BOOT_DISK_SIZE="${ANDROMEDA_GCP_BOOT_DISK_SIZE:-200GB}"

INSTANCE="andromeda-e2e-$(date --utc +%Y%m%d-%H%M%S 2>/dev/null \
    || date -u +%Y%m%d-%H%M%S)"
readonly INSTANCE

gcloud_compute() {
    gcloud compute --project "${PROJECT}" "$@"
}

instance_created=0
cleanup() {
    local status="$?"
    if (( instance_created )); then
        printf 'Deleting instance %s (exit status %s)...\n' \
            "${INSTANCE}" "${status}" >&2
        if ! gcloud_compute instances delete "${INSTANCE}" \
            --zone "${ZONE}" --quiet; then
            printf 'FAILED to delete %s in %s; delete it manually:\n' \
                "${INSTANCE}" "${ZONE}" >&2
            printf '  gcloud compute instances delete %s --project %s --zone %s\n' \
                "${INSTANCE}" "${PROJECT}" "${ZONE}" >&2
            exit 1
        fi
    fi
    exit "${status}"
}
trap cleanup EXIT

command -v gcloud >/dev/null
command -v git >/dev/null
mkdir -p "${EVIDENCE_DIR}"

source_revision="$(git -C "${REPOSITORY_ROOT}" rev-parse HEAD)"
source_bundle="$(mktemp -t andromeda-src.XXXXXX.tar.gz)"
git -C "${REPOSITORY_ROOT}" archive --format=tar.gz \
    --output "${source_bundle}" HEAD

printf 'Creating %s (%s, %s, max-run-duration=%s) in %s...\n' \
    "${INSTANCE}" "${MACHINE_TYPE}" "${ZONE}" "${MAX_RUN_DURATION}" \
    "${PROJECT}" >&2
gcloud_compute instances create "${INSTANCE}" \
    --zone "${ZONE}" \
    --machine-type "${MACHINE_TYPE}" \
    --image-family "${IMAGE_FAMILY}" \
    --image-project "${IMAGE_PROJECT}" \
    --boot-disk-size "${BOOT_DISK_SIZE}" \
    --boot-disk-type pd-ssd \
    --enable-nested-virtualization \
    --provisioning-model STANDARD \
    --max-run-duration "${MAX_RUN_DURATION}" \
    --instance-termination-action DELETE \
    --no-restart-on-failure \
    --labels "purpose=andromeda-e2e,owner=andromeda-ci,disposable=true"
instance_created=1

printf 'Waiting for SSH...\n' >&2
ssh_ready=0
for _ in $(seq 1 30); do
    if gcloud_compute ssh "${INSTANCE}" --zone "${ZONE}" \
        --command 'true' >/dev/null 2>&1; then
        ssh_ready=1
        break
    fi
    sleep 10
done
test "${ssh_ready}" -eq 1

gcloud_compute scp "${source_bundle}" \
    "${INSTANCE}:/tmp/andromeda-src.tar.gz" --zone "${ZONE}"
rm -f "${source_bundle}"

gcloud_compute ssh "${INSTANCE}" --zone "${ZONE}" --command "
    set -Eeuo pipefail
    sudo apt-get update
    sudo DEBIAN_FRONTEND=noninteractive apt-get install --yes \
        binutils jq ovmf podman qemu-system-x86 qemu-utils runc
    mkdir -p ~/andromeda
    tar -xzf /tmp/andromeda-src.tar.gz -C ~/andromeda
    cd ~/andromeda
    sudo env ANDROMEDA_SOURCE_REVISION='${source_revision}' \
        os/scripts/test-gcp-nested.sh \"\$PWD\" \"\$PWD/output\"
"

gcloud_compute scp --recurse \
    "${INSTANCE}:~/andromeda/output/gcp-evidence" \
    "${EVIDENCE_DIR}/" --zone "${ZONE}"

printf 'Evidence downloaded to %s\n' "${EVIDENCE_DIR}" >&2
printf 'ANDROMEDA_GCP_E2E_WRAPPER_OK instance=%s revision=%s\n' \
    "${INSTANCE}" "${source_revision}"
