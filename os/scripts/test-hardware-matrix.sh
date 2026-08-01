#!/usr/bin/env bash
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly BASE_DISK="${OUTPUT_DIR}/andromeda-test.qcow2"
readonly MATRIX_DIR="${OUTPUT_DIR}/hardware-matrix"
readonly OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
readonly OVMF_VARS_TEMPLATE="${OVMF_VARS_TEMPLATE:-/usr/share/OVMF/OVMF_VARS_4M.fd}"

qemu_pid=""

cleanup() {
    if [[ -n "${qemu_pid}" ]]; then
        kill "${qemu_pid}" 2>/dev/null || true
        wait "${qemu_pid}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

test -f "${BASE_DISK}"
test -f "${OVMF_CODE}"
test -f "${OVMF_VARS_TEMPLATE}"

rm -rf "${MATRIX_DIR}"
mkdir -p "${MATRIX_DIR}"
qemu-system-x86_64 --version > "${MATRIX_DIR}/qemu-version.txt"
qemu-img --version > "${MATRIX_DIR}/qemu-img-version.txt"

accel=tcg
cpu=max
profile_timeout_seconds=600
if [[ -c /dev/kvm ]]; then
    accel=kvm
    cpu=host
elif [[ "${ANDROMEDA_ALLOW_TCG:-0}" != "1" ]]; then
    # Mirror test-gcp-nested.sh:41's `test -c /dev/kvm` precheck. Full-system TCG
    # emulation is ~10x slower; the TCG per-profile budget (3600s x 3 profiles)
    # can exceed the os-e2e job `timeout-minutes: 150`, turning a missing
    # /dev/kvm into confusing timeouts rather than a clear failure. Fail fast;
    # set ANDROMEDA_ALLOW_TCG=1 to force the slow TCG path for local debugging.
    printf 'test-hardware-matrix.sh: /dev/kvm is unavailable.\n' >&2
    printf 'Full-system TCG emulation is ~10x slower and its per-profile budget can exceed the os-e2e job timeout (150m).\n' >&2
    printf 'Set ANDROMEDA_ALLOW_TCG=1 to force the slow TCG path for local debugging.\n' >&2
    exit 1
else
    # Full-system TCG emulation is roughly an order of magnitude slower than
    # KVM; the 600 s per-profile budget would always time out.
    profile_timeout_seconds=3600
    printf 'WARNING: /dev/kvm is unavailable; ANDROMEDA_ALLOW_TCG=1 is set, falling back to TCG emulation with a %s s per-profile timeout. Expect a very slow run.\n' \
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

    qemu_command=(
        qemu-system-x86_64
        -machine "${machine},accel=${accel}"
        -cpu "${cpu}"
        -smp "${topology}"
        -m "${memory_mib}"
        -nodefaults
        -no-user-config
        -fw_cfg "name=opt/andromeda/scenario,string=${scenario}"
        -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}"
        -drive "if=pflash,format=raw,file=${ovmf_vars}"
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
        if grep -qE 'ANDROMEDA_.*_FAILED' "${serial_log}" 2>/dev/null; then
            printf 'Guest failure marker in scenario %s.\n' "${scenario}" >&2
            grep -E 'ANDROMEDA_.*_(FAILED|OK)' "${serial_log}" >&2
            return 1
        fi
        if grep -q "ANDROMEDA_E2E_OK" "${serial_log}" 2>/dev/null; then
            break
        fi
        if ! kill -0 "${qemu_pid}" 2>/dev/null; then
            wait "${qemu_pid}" || true
            qemu_pid=""
            printf 'QEMU exited before matrix success in scenario %s.\n' \
                "${scenario}" >&2
            tail -200 "${serial_log}" >&2
            return 1
        fi
        sleep 3
    done

    if ! grep -q "ANDROMEDA_E2E_OK" "${serial_log}" 2>/dev/null; then
        printf 'Timed out waiting for matrix scenario %s.\n' "${scenario}" >&2
        tail -200 "${serial_log}" >&2
        return 1
    fi
    grep -q \
        "ANDROMEDA_HARDWARE_REPORT_OK readiness=ready boot_critical_missing=0 scenario=${scenario}" \
        "${serial_log}"
    test "$(grep -c 'ANDROMEDA_E2E_OK' "${serial_log}")" -eq 1

    kill "${qemu_pid}"
    wait "${qemu_pid}" 2>/dev/null || true
    qemu_pid=""

    qemu-img check "${overlay}" > "${profile_dir}/qemu-img-check.txt" 2>&1 \
        || image_check_status="$?"
    printf '%s\n' "${image_check_status}" \
        > "${profile_dir}/qemu-img-check-status.txt"
    test "${image_check_status}" -eq 0
    printf 'ANDROMEDA_MATRIX_OK scenario=%s\n' "${scenario}"
}

run_profile modern-nvme q35 "4,sockets=1,cores=2,threads=2" 8192
run_profile q35-sata q35 "4,sockets=2,cores=2,threads=1" 8192
run_profile legacy-i440fx pc "2,sockets=1,cores=2,threads=1" 8192

printf 'ANDROMEDA_HARDWARE_MATRIX_OK scenarios=3\n'
