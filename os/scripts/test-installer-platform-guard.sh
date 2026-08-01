#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly GUARD="${REPOSITORY_ROOT}/os/installer/check-platform-compatibility.sh"
FIXTURE_ROOT="$(mktemp -d)"
readonly FIXTURE_ROOT

cleanup() {
    rm -rf "${FIXTURE_ROOT}"
}
trap cleanup EXIT

write_manifest() {
    local variant="$1"
    local architecture="$2"
    local boot_provider="$3"
    jq --null-input \
        --arg variant "${variant}" \
        --arg architecture "${architecture}" \
        --arg boot_provider "${boot_provider}" \
        '{schema_version: 1,
          variant: $variant,
          architecture: $architecture,
          boot_provider: $boot_provider,
          hep_id: "fixture"}' \
        > "${FIXTURE_ROOT}/platform.json"
}

write_dmi() {
    local manufacturer="$1"
    local product_name="$2"
    mkdir -p "${FIXTURE_ROOT}/dmi"
    printf '%s\n' "${manufacturer}" > "${FIXTURE_ROOT}/dmi/sys_vendor"
    printf '%s\n' "${product_name}" > "${FIXTURE_ROOT}/dmi/product_name"
}

expect_ok() {
    local expected="$1"
    local output
    output="$(
        ANDROMEDA_DMI_ROOT="${FIXTURE_ROOT}/dmi" \
        ANDROMEDA_UNAME_MACHINE=x86_64 \
        bash "${GUARD}" "${FIXTURE_ROOT}/platform.json"
    )"
    grep -q "${expected}" <<<"${output}"
}

expect_failure() {
    local machine="$1"
    local expected_code="$2"
    local output
    local status=0
    output="$(
        ANDROMEDA_DMI_ROOT="${FIXTURE_ROOT}/dmi" \
        ANDROMEDA_UNAME_MACHINE="${machine}" \
        bash "${GUARD}" "${FIXTURE_ROOT}/platform.json" 2>&1
    )" || status="$?"
    test "${status}" -ne 0
    grep -q "ANDROMEDA_PLATFORM_CHECK_FAILED code=${expected_code}" \
        <<<"${output}"
}

expect_ci() {
    local virtualization="$1"
    local expected="$2"
    local output
    local status=0
    output="$(
        ANDROMEDA_DMI_ROOT="${FIXTURE_ROOT}/dmi" \
        ANDROMEDA_UNAME_MACHINE=x86_64 \
        ANDROMEDA_VIRTUALIZATION="${virtualization}" \
        bash "${GUARD}" "${FIXTURE_ROOT}/platform.json" ci 2>&1
    )" || status="$?"
    if [[ "${expected}" == ok ]]; then
        test "${status}" -eq 0
        grep -q 'ANDROMEDA_PLATFORM_CHECK_OK.*mode=ci' <<<"${output}"
    else
        test "${status}" -ne 0
        grep -q "ANDROMEDA_PLATFORM_CHECK_FAILED code=${expected}" \
            <<<"${output}"
    fi
}

write_manifest pc_x86_64 x86_64 pc_uefi_shim
write_dmi "QEMU" "Standard PC (Q35 + ICH9, 2009)"
expect_ok "ANDROMEDA_PLATFORM_CHECK_OK variant=pc_x86_64"
expect_ci kvm ok

write_dmi "Dell Inc." "Latitude 7450"
expect_ci none destructive_install_requires_vm

expect_ok "ANDROMEDA_PLATFORM_CHECK_OK variant=pc_x86_64"

for apple_model in MacBookPro12,1 MacBookPro16,1 MacBookAir10,1 Mac99,1; do
    write_dmi "Apple Inc." "${apple_model}"
    expect_failure x86_64 apple_requires_dedicated_image
done

write_dmi "Apple Inc." "MacBookAir10,1"
expect_failure aarch64 apple_requires_dedicated_image

write_dmi "Generic" "ARM virtual machine"
expect_failure aarch64 architecture_mismatch

write_manifest t2_x86_64 x86_64 t2_apple_efi
write_dmi "Apple Inc." "MacBookPro16,1"
expect_failure x86_64 unsupported_platform_variant

write_dmi "QEMU" "Standard PC (Q35 + ICH9, 2009)"
printf '%s\n' 'this is not json' > "${FIXTURE_ROOT}/platform.json"
expect_failure x86_64 invalid_platform_manifest

write_manifest pc_x86_64 x86_64 ""
expect_failure x86_64 invalid_platform_manifest

write_manifest pc_x86_64 x86_64 t2_apple_efi
expect_failure x86_64 boot_provider_mismatch

printf 'ANDROMEDA_PLATFORM_GUARD_TEST_OK cases=14\n'
