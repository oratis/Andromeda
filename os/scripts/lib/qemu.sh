#!/usr/bin/env bash
# Shared QEMU host wiring for the os/scripts E2E harnesses.
#
# Why this exists: docs/reviews/e2e-pipeline-review.md P2 #13 names the
# copy-paste between test-install.sh and test-hardware-matrix.sh as the
# STRUCTURAL root cause of P0 #3 -- a fix (CR/ANSI marker stripping) that landed
# in one poller and not the other. lib/markers.sh closed that specific hole;
# this file closes the two remaining decisions with the same property.
#
# WHAT BELONGS HERE, AND WHAT DELIBERATELY DOES NOT.
#
# The test is not "do these two blocks look alike" -- it is "would a future fix
# have to be applied to BOTH call sites to be correct". Only three things pass
# that test:
#
#   1. The KVM-vs-TCG gate. Both harnesses must agree on what counts as an
#      available accelerator, on which env var overrides the refusal, and on the
#      accel/cpu pair each answer implies. A harness that silently accepted TCG
#      while its sibling refused would reintroduce exactly the confusing-timeout
#      failure the gate was added to prevent (review evaluation 2).
#   2. Where the OVMF firmware lives. Both boot the SAME firmware through the
#      SAME UEFI path, and UEFI boot is the thing under test. A packaging move
#      (review evaluation 5 notes non-Debian hosts must already override these)
#      fixed in one harness and not the other means the matrix silently
#      exercises different firmware than the lifecycle test.
#   3. How that firmware is wired onto the pflash bus -- specifically that the
#      code image is readonly=on and the variable store is a per-boot writable
#      COPY. Drop readonly=on in one harness and a guest can scribble on the
#      shared firmware image and poison every later boot on the host.
#   4. Reaping the QEMU child. The matrix boots overlays whose backing file is
#      the qcow2 test-install.sh leaves behind, so a QEMU that outlives its
#      harness still holds that disk open -- a leak in EITHER script corrupts
#      the OTHER one's input.
#
# What is NOT here, on purpose: machine type, -smp, memory, and the whole device
# block. Those differ between the two harnesses because they are SUPPOSED to --
# varying the emulated controller topology is the entire point of the hardware
# matrix, and test-install.sh's virtio-only baseline is a different experiment.
# Nor are the timeout constants: each is owned by the script that enforces it
# and is parsed back out of that script by test-ci-timeout-budget.sh, so moving
# them here would create the second source of truth that guard exists to
# prevent.
#
# Source it from a sibling script with:
#     ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#     # shellcheck source=os/scripts/lib/qemu.sh
#     # shellcheck disable=SC1091
#     . "${ANDROMEDA_SCRIPT_DIR}/lib/qemu.sh"
# which resolves correctly from CI, from `sudo os/scripts/...`, and from a
# checkout unpacked anywhere (the GCP path untars the repo into ~/andromeda).
#
# Requires lib/assert.sh to have been sourced first (require_file).

# Idempotent: sourcing twice must not clobber anything.
if [[ -n "${ANDROMEDA_QEMU_SH_LOADED:-}" ]]; then
    return 0
fi
readonly ANDROMEDA_QEMU_SH_LOADED=1

# The accelerator probe target. Overridable ONLY so the decision below can be
# exercised on a host that has no KVM at all -- the maintainer's main platform
# is darwin (review evaluation 5), where every branch of this function would
# otherwise be untestable. CI never sets it, so CI always probes /dev/kvm.
: "${ANDROMEDA_KVM_DEVICE:=/dev/kvm}"

# Debian/Ubuntu ovmf package layout, which is what the CI runner has. A host
# with a different layout (Fedora, Arch) overrides these in the environment;
# both harnesses read the same two names so one override covers the whole run.
: "${OVMF_CODE:=/usr/share/OVMF/OVMF_CODE_4M.fd}"
: "${OVMF_VARS_TEMPLATE:=/usr/share/OVMF/OVMF_VARS_4M.fd}"
readonly OVMF_CODE
readonly OVMF_VARS_TEMPLATE

# require_ovmf_firmware
#
# Asserts both firmware files exist, naming the package to install and the
# variable to override. Every UEFI boot in the pipeline depends on these, so the
# failure has to say so rather than surfacing as a QEMU startup error.
require_ovmf_firmware() {
    require_file "${OVMF_CODE}" \
        'the OVMF firmware image is required for the UEFI boot path; install the ovmf package or set OVMF_CODE'
    require_file "${OVMF_VARS_TEMPLATE}" \
        'the OVMF variable-store template is required to give every boot a fresh NVRAM copy; install the ovmf package or set OVMF_VARS_TEMPLATE'
}

# ovmf_pflash_args <writable-vars-file>
#
# Leaves the two -drive arguments that attach OVMF to the pflash bus in the
# array ANDROMEDA_OVMF_PFLASH_ARGS, for splicing into a caller's argv:
#
#     ovmf_pflash_args "${OVMF_VARS}"
#     qemu-system-x86_64 ... "${ANDROMEDA_OVMF_PFLASH_ARGS[@]}" ...
#
# An array rather than stdout because these arguments contain paths: capturing
# them through a command substitution would word-split a directory with a space
# in it, and the GCP path runs out of an untarred bundle in an operator's home.
#
# Order is load-bearing: pflash units are assigned in the order the drives
# appear, and the firmware expects code at unit 0 and variables at unit 1.
# readonly=on is equally load-bearing -- see the header.
ovmf_pflash_args() {
    local vars_file="$1"

    # shellcheck disable=SC2034 # Read by the sourcing harness, not by this file.
    ANDROMEDA_OVMF_PFLASH_ARGS=(
        -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}"
        -drive "if=pflash,format=raw,file=${vars_file}"
    )
}

# select_qemu_acceleration <harness-name> <what-tcg-would-cost-here>
#
# Leaves the chosen accelerator in ANDROMEDA_QEMU_ACCEL and the matching CPU
# model in ANDROMEDA_QEMU_CPU:
#
#   /dev/kvm present            -> kvm / host, silently
#   absent, no override         -> prints the refusal below and EXITS 1
#   absent, ANDROMEDA_ALLOW_TCG=1 -> tcg / max, after a WARNING
#
# The refusal exists because the TCG software fallback is roughly an order of
# magnitude slower, and every harness that would then blow through its budget
# reports that as an unattributable timeout rather than "this host has no KVM".
# Callers pass the one sentence that differs: their own arithmetic for what TCG
# costs THEM. Everything else -- the probe, the override name, the accel/cpu
# pairs, the "set ANDROMEDA_ALLOW_TCG=1" instruction -- is identical on purpose
# and must stay that way.
#
# A caller that also has to widen a timeout on the TCG path branches on
# ANDROMEDA_QEMU_ACCEL afterwards, so the timeout stays owned by the script that
# enforces it.
select_qemu_acceleration() {
    local harness="$1"
    local tcg_consequence="$2"

    if [[ -c "${ANDROMEDA_KVM_DEVICE}" ]]; then
        ANDROMEDA_QEMU_ACCEL=kvm
        ANDROMEDA_QEMU_CPU=host
        return 0
    fi

    if [[ "${ANDROMEDA_ALLOW_TCG:-0}" != "1" ]]; then
        printf '%s: %s is unavailable.\n' "${harness}" "${ANDROMEDA_KVM_DEVICE}" >&2
        printf '%s\n' "${tcg_consequence}" >&2
        printf 'Set ANDROMEDA_ALLOW_TCG=1 to force the slow TCG path for local debugging.\n' >&2
        exit 1
    fi

    # shellcheck disable=SC2034 # Read by the sourcing harness, not by this file.
    ANDROMEDA_QEMU_ACCEL=tcg
    # shellcheck disable=SC2034 # Read by the sourcing harness, not by this file.
    ANDROMEDA_QEMU_CPU=max
    printf 'WARNING: %s is unavailable; ANDROMEDA_ALLOW_TCG=1 is set, using slow TCG emulation. Expect a very slow run that may exceed CI budgets.\n' \
        "${ANDROMEDA_KVM_DEVICE}" >&2
}

# stop_child_process <pid>
#
# Terminates and reaps a background child, tolerating one that has already
# exited. Empty pid is a no-op so cleanup traps can call it unconditionally.
#
# `wait` is not optional politeness: without it the harness can return while
# QEMU is still unmapping the qcow2 it had open, and the next stage
# (test-hardware-matrix.sh overlays the disk test-install.sh produced) then
# opens a file another process still holds. The same reasoning applies to
# test-install.sh's update server, which keeps its listening socket until it is
# actually reaped.
stop_child_process() {
    local pid="$1"

    [[ -n "${pid}" ]] || return 0
    kill "${pid}" 2>/dev/null || true
    wait "${pid}" 2>/dev/null || true
}
