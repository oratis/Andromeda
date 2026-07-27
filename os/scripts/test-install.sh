#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly ISO_PATH="${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64.iso"
readonly DISK_PATH="${OUTPUT_DIR}/andromeda-test.qcow2"
readonly INSTALL_LOG="${OUTPUT_DIR}/install-serial.log"
readonly BOOT_LOG="${OUTPUT_DIR}/boot-serial.log"
readonly DIAGNOSTICS_DIR="${OUTPUT_DIR}/diagnostics"
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
        wait "${qemu_pid}" 2>/dev/null || true
    fi
    if [[ -n "${http_pid}" ]]; then
        kill "${http_pid}" 2>/dev/null || true
        wait "${http_pid}" 2>/dev/null || true
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
    if [[ -f "${OVMF_VARS}" && -d "${DIAGNOSTICS_DIR}/nvram" ]]; then
        cp -f "${OVMF_VARS}" \
            "${DIAGNOSTICS_DIR}/nvram/OVMF_VARS.final.fd" 2>/dev/null || true
    fi
}
trap cleanup EXIT

test -f "${ISO_PATH}"
test -f "${OUTPUT_DIR}/andromeda-v2.tar"
test -f "${OVMF_CODE}"
test -f "${OVMF_VARS_TEMPLATE}"

rm -rf "${DIAGNOSTICS_DIR}"
mkdir -p \
    "${DIAGNOSTICS_DIR}/host" \
    "${DIAGNOSTICS_DIR}/nvram" \
    "${DIAGNOSTICS_DIR}/root"
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

install_status=0
timeout 45m qemu-system-x86_64 \
    "${common_qemu[@]}" \
    -drive "if=ide,media=cdrom,readonly=on,file=${ISO_PATH}" \
    -serial "file:${INSTALL_LOG}" || install_status="$?"

printf '%s\n' "${install_status}" \
    >"${DIAGNOSTICS_DIR}/host/install-exit-status.txt"
qemu-img info "${DISK_PATH}" \
    >"${DIAGNOSTICS_DIR}/host/qemu-img-info.txt" 2>&1 || true
qemu-img check "${DISK_PATH}" \
    >"${DIAGNOSTICS_DIR}/host/qemu-img-check.txt" 2>&1 || true
cp -f "${OVMF_VARS}" \
    "${DIAGNOSTICS_DIR}/nvram/OVMF_VARS.after-install.fd"

modprobe nbd max_part=16
for candidate in /dev/nbd{0..15}; do
    if [[ ! -e "/sys/block/${candidate##*/}/pid" ]]; then
        nbd_device="${candidate}"
        break
    fi
done
test -n "${nbd_device}"

qemu-nbd --read-only --connect="${nbd_device}" "${DISK_PATH}"
udevadm settle
lsblk --paths --output \
    NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,PARTTYPE,UUID "${nbd_device}" \
    | tee "${DIAGNOSTICS_DIR}/host/lsblk.txt"
blkid "${nbd_device}"* \
    >"${DIAGNOSTICS_DIR}/host/blkid.txt" 2>&1 || true
sfdisk --dump "${nbd_device}" \
    >"${DIAGNOSTICS_DIR}/host/partition-table.sfdisk" 2>&1 || true

esp_partition="$(
    lsblk -prno NAME,PARTTYPE "${nbd_device}" \
        | awk 'tolower($2) == "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" {
            print $1
        }'
)"
root_partition="$(
    lsblk -prno NAME,LABEL "${nbd_device}" \
        | awk '$2 == "andromeda-root" { print $1 }'
)"
test "$(wc -w <<<"${esp_partition}")" -eq 1
test "$(wc -w <<<"${root_partition}")" -eq 1

esp_mount="$(mktemp -d "${OUTPUT_DIR}/esp.XXXXXX")"
root_mount="$(mktemp -d "${OUTPUT_DIR}/root.XXXXXX")"
mount -o ro "${esp_partition}" "${esp_mount}"
find "${esp_mount}" -maxdepth 4 -type f -printf '%P\n' \
    | sort | tee "${OUTPUT_DIR}/esp-tree.txt"

mount -o ro,noload "${root_partition}" "${root_mount}"
find "${root_mount}" -type f \
    \( -path '*/var/log/anaconda/*' \
    -o -name anaconda-ks.cfg \
    -o -name original-ks.cfg \
    -o -name andromeda-uefi-fallback.log \) \
    -print | sort >"${DIAGNOSTICS_DIR}/root/log-paths.txt"
: >"${DIAGNOSTICS_DIR}/root/log-excerpts.txt"
while IFS= read -r installed_log; do
    printf '\n===== %s =====\n' \
        "${installed_log#"${root_mount}"}" \
        >>"${DIAGNOSTICS_DIR}/root/log-excerpts.txt"
    tail -n 1000 "${installed_log}" \
        >>"${DIAGNOSTICS_DIR}/root/log-excerpts.txt" 2>&1 || true
done <"${DIAGNOSTICS_DIR}/root/log-paths.txt"
find "${root_mount}" -name andromeda-uefi-fallback.log -type f -print \
    -exec cat {} \; | tee "${OUTPUT_DIR}/install-post.log"

test -f "${esp_mount}/EFI/fedora/shimx64.efi"
test -f "${esp_mount}/EFI/fedora/grubx64.efi"
test -f "${esp_mount}/EFI/BOOT/BOOTX64.EFI"
test -f "${esp_mount}/EFI/BOOT/grubx64.efi"

umount "${root_mount}"
umount "${esp_mount}"
qemu-nbd --disconnect "${nbd_device}"
nbd_device=""
rmdir "${root_mount}" "${esp_mount}"
root_mount=""
esp_mount=""

strings --encoding=l "${OVMF_VARS}" \
    >"${DIAGNOSTICS_DIR}/nvram/strings-after-install.txt"
grep Andromeda "${DIAGNOSTICS_DIR}/nvram/strings-after-install.txt" \
    | tee "${OUTPUT_DIR}/ovmf-vars.txt"

if [[ "${install_status}" -ne 0 ]]; then
    printf 'Installer exited with status %s.\n' "${install_status}" >&2
    exit "${install_status}"
fi

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
