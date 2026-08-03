#!/usr/bin/env bash
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly ANDROMEDA_SCRIPT_DIR
# Resolved from BASH_SOURCE so it works from CI, from `sudo os/scripts/...`, and
# from the GCP path where the repository is untarred into ~/andromeda.
# shellcheck source=os/scripts/lib/assert.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/assert.sh"
# shellcheck source=os/scripts/lib/markers.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/markers.sh"
# Sourced after assert.sh, which it uses. It owns OVMF_CODE/OVMF_VARS_TEMPLATE,
# the KVM-vs-TCG gate and the pflash wiring, all shared with test-install.sh;
# see its header for what is shared and what is deliberately not.
# shellcheck source=os/scripts/lib/qemu.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/qemu.sh"
readonly OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly BASE_DISK="${OUTPUT_DIR}/andromeda-test.qcow2"
readonly MATRIX_DIR="${OUTPUT_DIR}/hardware-matrix"

qemu_pid=""

cleanup() {
    stop_child_process "${qemu_pid}"
}
trap cleanup EXIT

require_file "${BASE_DISK}" \
    'the matrix boots overlays on the disk test-install.sh leaves behind; run os/scripts/test-install.sh first (its qcow2 is the backing file for every profile)'
require_ovmf_firmware

rm -rf "${MATRIX_DIR}"
mkdir -p "${MATRIX_DIR}"
qemu-system-x86_64 --version > "${MATRIX_DIR}/qemu-version.txt"
qemu-img --version > "${MATRIX_DIR}/qemu-img-version.txt"

# TIMEOUT BUDGET INVARIANT (docs/reviews/e2e-pipeline-review.md P0 #4,
# re-derived for the P1 #7 job split).
# This is the matrix stage budget: 3 profiles x 600 s = 30 min, one term of
# `sum of the lifecycle job's stage budgets < its timeout-minutes: 140` in
# os-e2e.yml (the image build has had its own job budget since the split).
# The full arithmetic lives next to the 2700 s boot deadline in
# test-install.sh.
#
# NOTE: this 600 s host cap deliberately sits BELOW the guest's
# andromeda-ci-verify.service TimeoutStartSec=12min (720 s). The verifier does
# NOT short-circuit on state=complete: os/files/usr/libexec/andromeda-ci-verify
# runs andromeda-daily-driver-verify unconditionally BEFORE its state case, and
# that runs inside the unit's 12 min timeout, so in principle the unit may
# still be within budget when this host kill fires. The inversion is
# intentional:
#   - on a state=complete disk the whole per-profile pass is fast (observed
#     ~40 s each, 2.0-2.5 min for all three profiles together), so 600 s is
#     already ~15x headroom;
#   - raising this to 840 s (12 min unit + ~2 min firmware/kernel/sddm/plasma
#     boot) would grow the matrix stage to 42 min and push the lifecycle job's
#     stage sum past its `timeout-minutes: 140`;
#   - a genuinely hung profile is diagnosed by THIS host-side timeout plus the
#     normalized serial log it dumps, not by waiting out the unit timeout.
profile_timeout_seconds=600
# The accelerator gate itself is shared with test-install.sh (lib/qemu.sh); it
# mirrors test-gcp-nested.sh's `test -c /dev/kvm` precheck. Only the second
# argument is this harness's own: what TCG would cost HERE, which is the TCG
# matrix budget (3600s x 3 profiles = 180 min) against the lifecycle job's
# timeout-minutes.
select_qemu_acceleration 'test-hardware-matrix.sh' \
    'Full-system TCG emulation is ~10x slower and its per-profile budget (3 x 3600s = 180m) exceeds the os-e2e lifecycle job timeout (140m).'
if [[ "${ANDROMEDA_QEMU_ACCEL}" == tcg ]]; then
    # The per-profile budget is NOT shared: it belongs to this file, which is
    # where test-ci-timeout-budget.sh parses it back out. Full-system TCG is
    # roughly an order of magnitude slower than KVM, so the 600 s KVM budget
    # would always time out.
    profile_timeout_seconds=3600
    printf 'Widening the per-profile timeout to %s s for the TCG path.\n' \
        "${profile_timeout_seconds}" >&2
fi
readonly profile_timeout_seconds

run_profile() {
    local scenario="$1"
    local machine="$2"
    local topology="$3"
    local memory_mib="$4"
    local profile_dir="${MATRIX_DIR}/${scenario}"
    local overlay="${profile_dir}/disk.qcow2"
    local ovmf_vars="${profile_dir}/OVMF_VARS.fd"
    local serial_log="${profile_dir}/serial.log"
    # Every marker check below reads this NUL/CR/ANSI-stripped copy rather than
    # the raw serial log. The report assertion is the longest exact-match string
    # in the pipeline, which makes it the single most likely victim of agetty
    # cursor-control residue; grepping the raw log was a latent source of false
    # 600 s timeouts. Same helper as test-install.sh -- see lib/markers.sh and
    # docs/reviews/e2e-pipeline-review.md P0 #3.
    local serial_log_normalized="${profile_dir}/serial.normalized.log"
    local qemu_argv="${profile_dir}/qemu-argv.txt"
    local image_check_status=0
    local deadline
    local -a devices
    local -a qemu_command

    mkdir -p "${profile_dir}"
    qemu-img create -f qcow2 -F qcow2 -b "${BASE_DISK}" "${overlay}"
    cp "${OVMF_VARS_TEMPLATE}" "${ovmf_vars}"

    case "${scenario}" in
        modern-nvme)
            devices=(
                -device virtio-vga
                -device virtio-rng-pci
                -device "qemu-xhci,id=xhci"
                -device "usb-kbd,bus=xhci.0"
                -device "usb-tablet,bus=xhci.0"
                -device "e1000e,netdev=net0"
                -netdev "user,id=net0"
                -audiodev "driver=none,id=audio0"
                -device ich9-intel-hda
                -device "hda-duplex,audiodev=audio0"
                -drive "if=none,format=qcow2,file=${overlay},id=disk0"
                -device "nvme,drive=disk0,serial=andromeda-nvme"
            )
            ;;
        q35-sata)
            devices=(
                -device virtio-vga
                -device virtio-rng-pci
                -device "qemu-xhci,id=xhci"
                -device "usb-kbd,bus=xhci.0"
                -device "e1000e,netdev=net0"
                -netdev "user,id=net0"
                -audiodev "driver=none,id=audio0"
                -device ich9-intel-hda
                -device "hda-duplex,audiodev=audio0"
                -device "ich9-ahci,id=sata"
                -drive "if=none,format=qcow2,file=${overlay},id=disk0"
                -device "ide-hd,drive=disk0,bus=sata.2"
            )
            ;;
        legacy-i440fx)
            devices=(
                -device virtio-vga
                -device virtio-rng-pci
                -device "piix3-usb-uhci,id=uhci"
                -device "usb-kbd,bus=uhci.0"
                -device "e1000,netdev=net0"
                -netdev "user,id=net0"
                -audiodev "driver=none,id=audio0"
                -device "AC97,audiodev=audio0"
                -drive "if=ide,format=qcow2,file=${overlay}"
            )
            ;;
        *)
            printf 'Unknown hardware matrix scenario: %s\n' "${scenario}" >&2
            return 2
            ;;
    esac

    # Firmware wiring is shared with test-install.sh; the machine type, CPU
    # topology, memory and the ${devices[@]} block above are deliberately not --
    # varying them across profiles is the entire point of this harness.
    ovmf_pflash_args "${ovmf_vars}"
    qemu_command=(
        qemu-system-x86_64
        -machine "${machine},accel=${ANDROMEDA_QEMU_ACCEL}"
        -cpu "${ANDROMEDA_QEMU_CPU}"
        -smp "${topology}"
        -m "${memory_mib}"
        -nodefaults
        -no-user-config
        -fw_cfg "name=opt/andromeda/scenario,string=${scenario}"
        "${ANDROMEDA_OVMF_PFLASH_ARGS[@]}"
        -boot "order=c,strict=on"
        -display none
        -monitor none
        -serial "file:${serial_log}"
        "${devices[@]}"
    )
    printf '%q ' "${qemu_command[@]}" > "${qemu_argv}"
    printf '\n' >> "${qemu_argv}"

    printf 'ANDROMEDA_MATRIX_START scenario=%s\n' "${scenario}"
    "${qemu_command[@]}" &
    qemu_pid="$!"
    deadline="$((SECONDS + profile_timeout_seconds))"

    while (( SECONDS < deadline )); do
        normalize_serial_log "${serial_log}" "${serial_log_normalized}"
        if LC_ALL=C grep --text -qE 'ANDROMEDA_.*_FAILED' \
            "${serial_log_normalized}"; then
            printf 'Guest failure marker in scenario %s.\n' "${scenario}" >&2
            LC_ALL=C grep --text -E 'ANDROMEDA_.*_(FAILED|OK)' \
                "${serial_log_normalized}" >&2
            return 1
        fi
        if LC_ALL=C grep --text -q 'ANDROMEDA_E2E_OK' \
            "${serial_log_normalized}"; then
            break
        fi
        if ! kill -0 "${qemu_pid}" 2>/dev/null; then
            wait "${qemu_pid}" || true
            qemu_pid=""
            printf 'QEMU exited before matrix success in scenario %s.\n' \
                "${scenario}" >&2
            tail -200 "${serial_log_normalized}" >&2
            return 1
        fi
        sleep 3
    done

    normalize_serial_log "${serial_log}" "${serial_log_normalized}"
    if ! LC_ALL=C grep --text -q 'ANDROMEDA_E2E_OK' \
        "${serial_log_normalized}"; then
        printf 'Timed out waiting for matrix scenario %s after %s s.\n' \
            "${scenario}" "${profile_timeout_seconds}" >&2
        tail -200 "${serial_log_normalized}" >&2
        return 1
    fi
    require_marker_fixed "${serial_log_normalized}" \
        "ANDROMEDA_HARDWARE_REPORT_OK readiness=ready boot_critical_missing=0 scenario=${scenario}" \
        "scenario ${scenario} booted but andromeda-hardware-report did not report readiness=ready with zero boot-critical drivers missing for this exact scenario (a scenario= mismatch means the fw_cfg tag did not reach the guest)"
    # `grep -c` counts LINES, not occurrences: two markers collapsed onto one
    # line by CR-stripping would count as 1. -o | wc -l counts occurrences.
    require "scenario ${scenario} must emit ANDROMEDA_E2E_OK exactly once; more than one occurrence means the guest rebooted into the verifier again instead of settling" \
        test "$(LC_ALL=C grep --text -o 'ANDROMEDA_E2E_OK' \
            "${serial_log_normalized}" | wc -l)" -eq 1

    kill "${qemu_pid}"
    wait "${qemu_pid}" 2>/dev/null || true
    qemu_pid=""

    qemu-img check "${overlay}" > "${profile_dir}/qemu-img-check.txt" 2>&1 \
        || image_check_status="$?"
    printf '%s\n' "${image_check_status}" \
        > "${profile_dir}/qemu-img-check-status.txt"
    require "the qcow2 overlay for scenario ${scenario} failed qemu-img check; see ${profile_dir}/qemu-img-check.txt" \
        test "${image_check_status}" -eq 0
    printf 'ANDROMEDA_MATRIX_OK scenario=%s\n' "${scenario}"
}

run_profile modern-nvme q35 "4,sockets=1,cores=2,threads=2" 8192
run_profile q35-sata q35 "4,sockets=2,cores=2,threads=1" 8192
run_profile legacy-i440fx pc "2,sockets=1,cores=2,threads=1" 8192

printf 'ANDROMEDA_HARDWARE_MATRIX_OK scenarios=3\n'
