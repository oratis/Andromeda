#!/usr/bin/bash
set -euo pipefail

readonly TARGET_ROOT="${1:?target root is required}"
readonly VENDOR_DIR="${TARGET_ROOT}/boot/efi/EFI/fedora"
readonly FALLBACK_DIR="${TARGET_ROOT}/boot/efi/EFI/BOOT"
readonly ESP_MOUNT="${TARGET_ROOT}/boot/efi"
readonly ESP_PARTITION_TYPE="c12a7328-f81f-11d2-ba4b-00a0c93ec93b"

command -v efibootmgr >/dev/null
mountpoint --quiet "${ESP_MOUNT}"

read -r esp_device esp_fstype esp_target < <(
    findmnt --raw --noheadings --mountpoint "${ESP_MOUNT}" \
        --output SOURCE,FSTYPE,TARGET
)
test "${esp_fstype}" = vfat
test "${esp_target}" = "${ESP_MOUNT}"

partition_type="$(lsblk --noheadings --output PARTTYPE "${esp_device}" | xargs)"
test "${partition_type,,}" = "${ESP_PARTITION_TYPE}"

if [[ -c /dev/ttyS0 ]]; then
    printf 'ANDROMEDA_INSTALLER_EFI_START device=%s\n' \
        "${esp_device}" >/dev/ttyS0
fi

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

sync "${FALLBACK_DIR}"

if [[ -c /dev/ttyS0 ]]; then
    printf 'ANDROMEDA_INSTALLER_EFI_OK disk=%s part=%s\n' \
        "${parent_device}" "${partition_number}" >/dev/ttyS0
fi
