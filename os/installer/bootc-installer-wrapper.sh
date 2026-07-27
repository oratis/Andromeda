#!/usr/bin/bash
set -uo pipefail

readonly REAL_BOOTC="/usr/libexec/andromeda-bootc-real"
readonly INSTALL_STORAGE="/run/bootc/storage"
readonly INSTALL_TMPDIR="${INSTALL_STORAGE}/andromeda-tmp"

if [[ "${1:-}" != "install" || "${2:-}" != "to-filesystem" ]]; then
    exec "${REAL_BOOTC}" "$@"
fi

storage_mount="$(
    findmnt --noheadings --raw --mountpoint "${INSTALL_STORAGE}" \
        --output TARGET
)"
if [[ "${storage_mount}" != "${INSTALL_STORAGE}" ]]; then
    printf 'Expected target-backed bootc storage at %s; found %s.\n' \
        "${INSTALL_STORAGE}" "${storage_mount:-nothing}" >&2
    exit 1
fi

mkdir -p "${INSTALL_TMPDIR}"
chmod 0700 "${INSTALL_TMPDIR}"
export TMPDIR="${INSTALL_TMPDIR}"

printf 'Using target-backed bootc temporary storage: %s\n' "${TMPDIR}"
bootc_status=0
"${REAL_BOOTC}" "$@" || bootc_status="$?"
rmdir "${INSTALL_TMPDIR}" 2>/dev/null || true
exit "${bootc_status}"
