#!/usr/bin/bash
set -euo pipefail

target_root_argument="${1:?target root is required}"
system_root_argument="${2:?system root is required}"
install_mode="${3:-interactive}"
TARGET_ROOT="$(realpath -e "${target_root_argument}")"
SYSTEM_ROOT="$(realpath -e "${system_root_argument}")"
readonly TARGET_ROOT
readonly SYSTEM_ROOT
readonly install_mode
readonly VENDOR_DIR="${TARGET_ROOT}/boot/efi/EFI/fedora"
readonly FALLBACK_DIR="${TARGET_ROOT}/boot/efi/EFI/BOOT"
readonly ESP_MOUNT="${TARGET_ROOT}/boot/efi"
readonly ESP_PARTITION_TYPE="c12a7328-f81f-11d2-ba4b-00a0c93ec93b"

case "${install_mode}" in
    interactive|ci)
        ;;
    *)
        printf 'Unknown install mode: %s\n' "${install_mode}" >&2
        exit 1
        ;;
esac

if [[ -c /dev/ttyS0 ]]; then
    exec > >(tee -a /dev/ttyS0) 2>&1
fi

printf 'ANDROMEDA_INSTALLER_EFI_START target=%s\n' "${ESP_MOUNT}"

command -v efibootmgr >/dev/null
mountpoint --quiet "${ESP_MOUNT}"

read -r esp_device esp_fstype esp_target < <(
    findmnt --raw --noheadings --target "${ESP_MOUNT}" \
        --output SOURCE,FSTYPE,TARGET
)
test "${esp_fstype}" = vfat
test "${esp_target}" = "${ESP_MOUNT}"

partition_type="$(lsblk --noheadings --output PARTTYPE "${esp_device}" | xargs)"
test "${partition_type,,}" = "${ESP_PARTITION_TYPE}"

printf 'validated EFI system partition: %s\n' "${esp_device}"

test -f "${VENDOR_DIR}/shimx64.efi"
test -f "${VENDOR_DIR}/grubx64.efi"

install -d -m 0755 "${FALLBACK_DIR}"
install -m 0644 "${VENDOR_DIR}/shimx64.efi" "${FALLBACK_DIR}/BOOTX64.EFI"
install -m 0644 "${VENDOR_DIR}/grubx64.efi" "${FALLBACK_DIR}/grubx64.efi"

if [[ -f "${VENDOR_DIR}/mmx64.efi" ]]; then
    install -m 0644 "${VENDOR_DIR}/mmx64.efi" "${FALLBACK_DIR}/mmx64.efi"
fi

parent_name="$(lsblk --noheadings --output PKNAME "${esp_device}" | xargs)"
partition_number="$(lsblk --noheadings --output PARTN "${esp_device}" | xargs)"
test -n "${parent_name}"
test -n "${partition_number}"
parent_device="/dev/${parent_name}"

efibootmgr \
    --create \
    --disk "${parent_device}" \
    --part "${partition_number}" \
    --label Andromeda \
    --loader '\EFI\fedora\shimx64.efi'

install -d -m 0755 "${SYSTEM_ROOT}/etc/systemd/system"
ln -sfn /usr/lib/systemd/system/graphical.target \
    "${SYSTEM_ROOT}/etc/systemd/system/default.target"
test "$(readlink "${SYSTEM_ROOT}/etc/systemd/system/default.target")" \
    = /usr/lib/systemd/system/graphical.target

target_kargs=()
if [[ "${install_mode}" == ci ]]; then
    target_kargs=(
        "andromeda.ci=1"
        "console=tty0"
        "console=ttyS0,115200n8"
    )
fi
OSTREE_SYSROOT="${TARGET_ROOT}" ostree admin instutil set-kargs \
    "${target_kargs[@]}"
if grep -R -E -- '(^|[[:space:]])selinux=0([[:space:]]|$)' \
    "${SYSTEM_ROOT}/boot/loader/entries"; then
    printf 'Target boot entries still disable SELinux.\n' >&2
    exit 1
fi
printf 'ANDROMEDA_INSTALLER_KARGS_OK mode=%s\n' "${install_mode}"

sync "${FALLBACK_DIR}"

if [[ -c /dev/ttyS0 ]]; then
    printf 'ANDROMEDA_INSTALLER_EFI_OK disk=%s part=%s\n' \
        "${parent_device}" "${partition_number}"
fi
