#!/usr/bin/bash
set -Eeuo pipefail

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

test -c /dev/kvm
grep -q vmx /proc/cpuinfo
test "$(grep -cw vmx /proc/cpuinfo)" -ge 4
test "$(nproc)" -ge 4
test "$(awk '/MemTotal/ { print $2 }' /proc/meminfo)" -ge 16000000
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
    timeout 100m os/scripts/build-iso.sh "${OUTPUT_DIR}"
) 2>&1 | tee "${EVIDENCE_DIR}/build.log"

(
    cd "${SOURCE_DIR}"
    timeout 60m os/scripts/test-install.sh "${OUTPUT_DIR}"
) 2>&1 | tee "${EVIDENCE_DIR}/test.log"

sha256sum "${OUTPUT_DIR}"/*.iso > "${EVIDENCE_DIR}/iso.sha256"
LC_ALL=C grep -aoE \
    'ANDROMEDA_(SELINUX_LABELS|DAILY_DRIVER|FIRST_BOOT|UPDATE|ROLLBACK|E2E)[[:print:]]*' \
    "${OUTPUT_DIR}/boot-serial.log" \
    > "${EVIDENCE_DIR}/lifecycle-markers.txt"
grep -qx 'ANDROMEDA_E2E_OK' "${EVIDENCE_DIR}/lifecycle-markers.txt"
