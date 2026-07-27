#!/usr/bin/bash
set -uo pipefail

readonly REAL_BOOTC="/usr/libexec/andromeda-bootc-real"

if [[ "${1:-}" != "install" || "${2:-}" != "to-filesystem" ]]; then
    exec "${REAL_BOOTC}" "$@"
fi

target_root="${!#}"
if [[ ! -d "${target_root}" ]]; then
    printf 'Expected bootc installation target directory; found %s.\n' \
        "${target_root}" >&2
    exit 1
fi

install_tmpdir="${target_root}/ostree/bootc/storage/andromeda-tmp"
mkdir -p "${install_tmpdir}"
chmod 0700 "${install_tmpdir}"
export TMPDIR="${install_tmpdir}"

printf 'Using target-backed bootc temporary storage: %s\n' "${TMPDIR}"
bootc_status=0
"${REAL_BOOTC}" "$@" || bootc_status="$?"
rmdir "${install_tmpdir}" 2>/dev/null || true
exit "${bootc_status}"
