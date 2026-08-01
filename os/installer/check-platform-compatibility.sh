#!/usr/bin/bash
set -euo pipefail

readonly PLATFORM_FILE="${1:-/usr/lib/andromeda/platform.json}"
readonly INSTALL_MODE="${2:-interactive}"
readonly DMI_ROOT="${ANDROMEDA_DMI_ROOT:-/sys/class/dmi/id}"
readonly MACHINE="${ANDROMEDA_UNAME_MACHINE:-$(uname -m)}"

fail() {
    local code="$1"
    shift
    printf 'ANDROMEDA_PLATFORM_CHECK_FAILED code=%s' "${code}" >&2
    if (( $# > 0 )); then
        printf ' %s' "$*" >&2
    fi
    printf '\n' >&2
    exit 1
}

read_dmi() {
    local name="$1"
    if [[ -r "${DMI_ROOT}/${name}" ]]; then
        tr -d '\000\r\n' < "${DMI_ROOT}/${name}"
    fi
}

if ! jq --exit-status \
    '.schema_version == 1
     and (.variant | type == "string" and length > 0)
     and (.architecture | type == "string" and length > 0)
     and (.boot_provider | type == "string" and length > 0)
     and (.hep_id | type == "string" and length > 0)' \
    "${PLATFORM_FILE}" >/dev/null; then
    fail invalid_platform_manifest "path=${PLATFORM_FILE}"
fi

variant="$(jq --raw-output '.variant' "${PLATFORM_FILE}")"
architecture="$(jq --raw-output '.architecture' "${PLATFORM_FILE}")"
boot_provider="$(jq --raw-output '.boot_provider' "${PLATFORM_FILE}")"
manufacturer="$(read_dmi sys_vendor)"
product_name="$(read_dmi product_name)"

case "${variant}" in
    pc_x86_64)
        if [[ "${manufacturer}" =~ [Aa][Pp][Pp][Ll][Ee] \
            || "${product_name}" == Mac* \
            || "${product_name}" == iMac* ]]; then
            fail apple_requires_dedicated_image \
                "manufacturer=${manufacturer:-unknown} model=${product_name:-unknown}"
        fi
        if [[ "${architecture}" != x86_64 || "${MACHINE}" != x86_64 ]]; then
            fail architecture_mismatch \
                "expected=${architecture} actual=${MACHINE}"
        fi
        if [[ "${boot_provider}" != pc_uefi_shim ]]; then
            fail boot_provider_mismatch \
                "expected=pc_uefi_shim actual=${boot_provider}"
        fi
        ;;
    *)
        fail unsupported_platform_variant "variant=${variant}"
        ;;
esac

case "${INSTALL_MODE}" in
    interactive)
        ;;
    ci)
        virtualization="$(
            if [[ -n "${ANDROMEDA_VIRTUALIZATION+x}" ]]; then
                printf '%s\n' "${ANDROMEDA_VIRTUALIZATION}"
            else
                systemd-detect-virt --vm 2>/dev/null || true
            fi
        )"
        if [[ -z "${virtualization}" || "${virtualization}" == none ]]; then
            fail destructive_install_requires_vm
        fi
        ;;
    *)
        fail invalid_install_mode "mode=${INSTALL_MODE}"
        ;;
esac

printf 'ANDROMEDA_PLATFORM_CHECK_OK variant=%s architecture=%s boot_provider=%s mode=%s\n' \
    "${variant}" "${architecture}" "${boot_provider}" "${INSTALL_MODE}"
