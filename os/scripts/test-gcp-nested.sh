#!/usr/bin/bash
set -Eeuo pipefail

ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly ANDROMEDA_SCRIPT_DIR
# Resolved from BASH_SOURCE, not from SOURCE_DIR: gcp-run-e2e.sh untars a
# `git archive` of the repository into ~/andromeda and runs this script from
# there, so the library must be found relative to this file.
# shellcheck source=os/scripts/lib/assert.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/assert.sh"
# This script used to grep the RAW boot serial log -- the third and last
# un-normalized marker path in the pipeline (docs/reviews/e2e-pipeline-review.md
# evaluation 7 counts three copies of the normalization: bash, Python, gcp).
# It now runs the same transform as both host pollers.
# shellcheck source=os/scripts/lib/markers.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/markers.sh"

SOURCE_DIR="${1:-$(pwd)}"
OUTPUT_DIR="${2:-${SOURCE_DIR}/output}"
EVIDENCE_DIR="${3:-${OUTPUT_DIR}/gcp-evidence}"
readonly SOURCE_DIR
readonly OUTPUT_DIR
readonly EVIDENCE_DIR

mkdir -p "${OUTPUT_DIR}" "${EVIDENCE_DIR}"

started_at="$(date --utc --iso-8601=seconds)"
on_exit() {
    local exit_status="$?"
    printf '%s\n' "${exit_status}" > "${EVIDENCE_DIR}/exit-status.txt"
    printf '%s\n' "${started_at}" > "${EVIDENCE_DIR}/started-at.txt"
    date --utc --iso-8601=seconds > "${EVIDENCE_DIR}/finished-at.txt"
    if [[ -d "${OUTPUT_DIR}/diagnostics" ]]; then
        cp -a "${OUTPUT_DIR}/diagnostics" "${EVIDENCE_DIR}/" 2>/dev/null || true
    fi
    if [[ -d "${OUTPUT_DIR}/hardware-matrix" ]]; then
        cp -a "${OUTPUT_DIR}/hardware-matrix" "${EVIDENCE_DIR}/" 2>/dev/null || true
    fi
    for artifact in \
        boot-serial.log \
        esp-tree.txt \
        install-post.log \
        install-serial.log \
        ovmf-vars.txt \
        update-server.log; do
        if [[ -f "${OUTPUT_DIR}/${artifact}" ]]; then
            cp -f "${OUTPUT_DIR}/${artifact}" "${EVIDENCE_DIR}/"
        fi
    done
    chmod -R a+rX "${EVIDENCE_DIR}" 2>/dev/null || true
    exit "${exit_status}"
}
trap on_exit EXIT

# Host preconditions for the nested-KVM run. This is the most expensive path in
# the whole pipeline -- by the time these run, a billed Compute Engine instance
# already exists -- so every one of them says what it wanted and what it found
# instead of leaving the operator with a bare exit code.
# See docs/reviews/e2e-pipeline-review.md P0 #2.
require 'nested virtualization is not available: /dev/kvm is missing, so the instance was created without --enable-nested-virtualization or on a machine type that does not support it' \
    test -c /dev/kvm
require 'the CPU does not expose the vmx flag in /proc/cpuinfo; nested virtualization is off for this instance' \
    grep -q vmx /proc/cpuinfo
require "at least 4 vmx-capable CPUs are required for the guest's -smp 4; this host reports $(grep -cw vmx /proc/cpuinfo)" \
    test "$(grep -cw vmx /proc/cpuinfo)" -ge 4
require "at least 4 host CPUs are required; nproc reports $(nproc)" \
    test "$(nproc)" -ge 4
require "at least ~16 GiB of RAM is required (guest -m 8192 plus the podman build); /proc/meminfo reports $(awk '/MemTotal/ { print $2 }' /proc/meminfo) kB" \
    test "$(awk '/MemTotal/ { print $2 }' /proc/meminfo)" -ge 16000000
require "at least 100 GiB free is required in ${OUTPUT_DIR} (4 GiB ISO + 4 GiB OCI archive + a 64 GiB qcow2 + matrix overlays); df reports $(df --output=avail --block-size=1 "${OUTPUT_DIR}" | tail -1 | xargs) bytes" \
    test "$(df --output=avail --block-size=1 "${OUTPUT_DIR}" | tail -1 | xargs)" \
    -ge 107374182400

{
    printf 'source_revision=%s\n' \
        "${ANDROMEDA_SOURCE_REVISION:-uncommitted-source-bundle}"
    printf 'vmx_cpus=%s\n' "$(grep -cw vmx /proc/cpuinfo)"
    printf 'kvm_device=%s\n' "$(stat --format='%t:%T' /dev/kvm)"
    uname -a
    lscpu
    qemu-system-x86_64 --version | head -1
    podman --version
} > "${EVIDENCE_DIR}/host-environment.txt"

(
    cd "${SOURCE_DIR}"
    # The automated lifecycle needs the destructive CI entry as the ISO's
    # GRUB default; build-iso.sh names that variant *-ci.iso.
    timeout 100m env INSTALLER_DEFAULT=1 os/scripts/build-iso.sh "${OUTPUT_DIR}"
) 2>&1 | tee "${EVIDENCE_DIR}/build.log"

(
    cd "${SOURCE_DIR}"
    timeout 60m os/scripts/test-install.sh "${OUTPUT_DIR}"
) 2>&1 | tee "${EVIDENCE_DIR}/test.log"

(
    cd "${SOURCE_DIR}"
    timeout 30m os/scripts/test-hardware-matrix.sh "${OUTPUT_DIR}"
) 2>&1 | tee "${EVIDENCE_DIR}/hardware-matrix.log"

sha256sum "${OUTPUT_DIR}"/*.iso > "${EVIDENCE_DIR}/iso.sha256"

# Normalize once, then read only the normalized copy -- exactly what the two
# host pollers do. The raw log is preserved beside it by the EXIT trap above, so
# no evidence is lost.
readonly BOOT_LOG_NORMALIZED="${EVIDENCE_DIR}/boot-serial.normalized.log"
normalize_serial_log "${OUTPUT_DIR}/boot-serial.log" "${BOOT_LOG_NORMALIZED}"
# `|| true` because the redirect has already created the file: a grep that
# matches nothing must reach the assertion below, which says WHAT was missing
# and shows the log, rather than aborting the script on grep's exit 1 with the
# bare exit code this review pass exists to eliminate.
LC_ALL=C grep -aoE \
    'ANDROMEDA_(SELINUX_LABELS|DAILY_DRIVER|FIRST_BOOT|UPDATE|ROLLBACK|E2E)[[:print:]]*' \
    "${BOOT_LOG_NORMALIZED}" \
    > "${EVIDENCE_DIR}/lifecycle-markers.txt" || true

# Exactly once, not merely present: this was the loosest of the pipeline's three
# marker checks (review evaluation 7). A second ANDROMEDA_E2E_OK would mean the
# guest re-entered the verifier instead of settling, which mere existence cannot
# see.
#
# It asserts ONE marker rather than restating test-install.sh's ten-marker
# sequence on purpose. test-install.sh ran that check itself a few lines above
# -- under `set -Eeuo pipefail` its failure aborts this script, so this line is
# unreachable unless the strict validator already passed. A second copy of that
# list here would be a second thing to keep in step with the guest, which is the
# drift this whole change is removing.
require_marker_sequence "${BOOT_LOG_NORMALIZED}" \
    'the guest never reached the end of the install/update/rollback lifecycle on this nested-KVM host, or reached it more than once' \
    "ANDROMEDA_E2E_OK"
require_marker "${EVIDENCE_DIR}/hardware-matrix.log" \
    '^ANDROMEDA_HARDWARE_MATRIX_OK scenarios=3$' \
    'the hardware matrix did not complete all three controller profiles on this nested-KVM host'
