#!/usr/bin/bash
set -euo pipefail

readonly TARGET_ROOT="${1:?target root is required}"
readonly VENDOR_DIR="${TARGET_ROOT}/boot/efi/EFI/fedora"
readonly FALLBACK_DIR="${TARGET_ROOT}/boot/efi/EFI/BOOT"

test -f "${VENDOR_DIR}/shimx64.efi"
test -f "${VENDOR_DIR}/grubx64.efi"

install -d -m 0755 "${FALLBACK_DIR}"
install -m 0644 "${VENDOR_DIR}/shimx64.efi" "${FALLBACK_DIR}/BOOTX64.EFI"
install -m 0644 "${VENDOR_DIR}/grubx64.efi" "${FALLBACK_DIR}/grubx64.efi"

if [[ -f "${VENDOR_DIR}/mmx64.efi" ]]; then
    install -m 0644 "${VENDOR_DIR}/mmx64.efi" "${FALLBACK_DIR}/mmx64.efi"
fi

sync "${FALLBACK_DIR}"
