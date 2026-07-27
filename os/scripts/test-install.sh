#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly ISO_PATH="${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso"
readonly DISK_PATH="${OUTPUT_DIR}/andromeda-test.qcow2"
readonly INSTALL_LOG="${OUTPUT_DIR}/install-serial.log"
readonly BOOT_LOG="${OUTPUT_DIR}/boot-serial.log"
readonly OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
readonly OVMF_VARS_TEMPLATE="${OVMF_VARS_TEMPLATE:-/usr/share/OVMF/OVMF_VARS_4M.fd}"
readonly OVMF_VARS="${OUTPUT_DIR}/OVMF_VARS.fd"

http_pid=""
qemu_pid=""
nbd_device=""
esp_mount=""
root_mount=""

# shellcheck disable=SC2317,SC2329 # Invoked indirectly by the EXIT trap.
cleanup() {
    if [[ -n "${qemu_pid}" ]]; then
        kill "${qemu_pid}" 2>/dev/null || true
    fi
    if [[ -n "${http_pid}" ]]; then
        kill "${http_pid}" 2>/dev/null || true
    fi
    if [[ -n "${root_mount}" ]] && mountpoint --quiet "${root_mount}"; then
        umount "${root_mount}" || true
    fi
    if [[ -n "${esp_mount}" ]] && mountpoint --quiet "${esp_mount}"; then
        umount "${esp_mount}" || true
    fi
    if [[ -n "${nbd_device}" ]]; then
        qemu-nbd --disconnect "${nbd_device}" 2>/dev/null || true
    fi
    if [[ -n "${root_mount}" ]]; then
        rmdir "${root_mount}" 2>/dev/null || true
    fi
    if [[ -n "${esp_mount}" ]]; then
        rmdir "${esp_mount}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

test -f "${ISO_PATH}"
test -f "${OUTPUT_DIR}/andromeda-v2.tar"
test -f "${OVMF_CODE}"
test -f "${OVMF_VARS_TEMPLATE}"

rm -f "${DISK_PATH}" "${INSTALL_LOG}" "${BOOT_LOG}" "${OVMF_VARS}"
qemu-img create -f qcow2 "${DISK_PATH}" 32G
cp "${OVMF_VARS_TEMPLATE}" "${OVMF_VARS}"

accel=tcg
cpu=max
if [[ -c /dev/kvm ]]; then
    accel=kvm
    cpu=host
fi

common_qemu=(
    -machine "q35,accel=${accel}"
    -cpu "${cpu}"
    -smp 4
    -m 6144
    -nodefaults
    -no-user-config
    -device virtio-vga
    -device virtio-rng-pci
    -device "virtio-net-pci,netdev=net0"
    -netdev "user,id=net0"
    -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}"
    -drive "if=pflash,format=raw,file=${OVMF_VARS}"
    -drive "if=virtio,format=qcow2,file=${DISK_PATH}"
    -display none
    -monitor none
)

timeout 45m qemu-system-x86_64 \
    "${common_qemu[@]}" \
    -drive "if=ide,media=cdrom,readonly=on,file=${ISO_PATH}" \
    -serial "file:${INSTALL_LOG}"

modprobe nbd max_part=16
for candidate in /dev/nbd{0..15}; do
    if [[ ! -e "/sys/block/${candidate##*/}/pid" ]]; then
        nbd_device="${candidate}"
        break
    fi
done
test -n "${nbd_device}"

qemu-nbd --connect="${nbd_device}" "${DISK_PATH}"
udevadm settle
esp_mount="$(mktemp -d "${OUTPUT_DIR}/esp.XXXXXX")"
root_mount="$(mktemp -d "${OUTPUT_DIR}/root.XXXXXX")"
mount -o ro "${nbd_device}p1" "${esp_mount}"
find "${esp_mount}/EFI" -maxdepth 3 -type f -printf '%P\n' \
    | sort | tee "${OUTPUT_DIR}/esp-tree.txt"
test -f "${esp_mount}/EFI/fedora/shimx64.efi"
test -f "${esp_mount}/EFI/fedora/grubx64.efi"
test -f "${esp_mount}/EFI/BOOT/BOOTX64.EFI"
test -f "${esp_mount}/EFI/BOOT/grubx64.efi"

mount -o ro "${nbd_device}p2" "${root_mount}"
find "${root_mount}" -name andromeda-uefi-fallback.log -type f -print \
    -exec cat {} \; | tee "${OUTPUT_DIR}/install-post.log"
umount "${root_mount}"
umount "${esp_mount}"
qemu-nbd --disconnect "${nbd_device}"
nbd_device=""
rmdir "${root_mount}" "${esp_mount}"
root_mount=""
esp_mount=""

strings --encoding=l "${OVMF_VARS}" \
    | grep Andromeda | tee "${OUTPUT_DIR}/ovmf-vars.txt"

python3 -m http.server 8080 \
    --bind 0.0.0.0 \
    --directory "${OUTPUT_DIR}" \
    >"${OUTPUT_DIR}/update-server.log" 2>&1 &
http_pid="$!"

qemu-system-x86_64 \
    "${common_qemu[@]}" \
    -boot order=c \
    -serial "file:${BOOT_LOG}" &
qemu_pid="$!"

deadline="$((SECONDS + 2700))"
while (( SECONDS < deadline )); do
    if grep -q ANDROMEDA_E2E_OK "${BOOT_LOG}" 2>/dev/null; then
        grep -E 'ANDROMEDA_(FIRST_BOOT|UPDATE|ROLLBACK|E2E)' "${BOOT_LOG}"
        exit 0
    fi
    if grep -q 'Shell>' "${BOOT_LOG}" 2>/dev/null; then
        printf 'UEFI firmware could not find an installed bootloader.\n' >&2
        tail -200 "${BOOT_LOG}" >&2
        exit 1
    fi
    if ! kill -0 "${qemu_pid}" 2>/dev/null; then
        wait "${qemu_pid}"
        printf 'QEMU exited before the end-to-end marker was emitted.\n' >&2
        tail -200 "${BOOT_LOG}" >&2
        exit 1
    fi
    sleep 5
done

printf 'Timed out waiting for ANDROMEDA_E2E_OK.\n' >&2
tail -200 "${BOOT_LOG}" >&2
exit 1
