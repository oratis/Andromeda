#!/usr/bin/env bash
set -euo pipefail

# Asserts the os-e2e job's `timeout-minutes` can actually contain the deadlines
# the harness enforces on itself.
#
# The failure this prevents is not a wrong number; it is the *drift* between
# two numbers that live in different files and are edited by different people.
# When a step deadline grows past the job budget, the job is killed mid-step
# and the evidence upload never runs, so the run reports "cancelled" with no
# serial log — the least diagnosable failure the pipeline can produce.
#
# Every bound below is PARSED from the file that owns it rather than restated
# here. A guard that keeps its own copy of the constants is a second source of
# truth and drifts exactly like the thing it guards.
#
# Scope: the GitHub job only. `os/scripts/test-gcp-nested.sh` wraps the same
# scripts in its own `timeout` values for an operator-run GCP VM, which is
# bounded by the instance `--max-run-duration` rather than by
# `timeout-minutes`, and is never invoked from a workflow.

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly WORKFLOW="${REPOSITORY_ROOT}/.github/workflows/os-e2e.yml"
readonly INSTALL_SCRIPT="${REPOSITORY_ROOT}/os/scripts/test-install.sh"
readonly MATRIX_SCRIPT="${REPOSITORY_ROOT}/os/scripts/test-hardware-matrix.sh"

# Wall-clock the job spends outside the three measured steps: runner setup,
# free-disk-space, apt, shellcheck, the layer-budget guard, and artifact
# upload. Deliberately generous; it is the only estimate here.
readonly FIXED_OVERHEAD_MINUTES=15

# The ISO build has no inner `timeout`, so nothing bounds it but the job. This
# is the reserve the budget sets aside for it; if the real build grows past
# this the guard trips, which is the intended signal.
readonly BUILD_RESERVE_MINUTES=45

fail() {
    printf 'test-ci-timeout-budget.sh: %s\n' "$1" >&2
    exit 1
}

require_file() {
    [[ -f "$1" ]] || fail "missing $1"
}

require_file "${WORKFLOW}"
require_file "${INSTALL_SCRIPT}"
require_file "${MATRIX_SCRIPT}"

# --- job budget ------------------------------------------------------------

job_timeout_minutes="$(
    grep -oE '^[[:space:]]*timeout-minutes:[[:space:]]*[0-9]+' "${WORKFLOW}" |
        grep -oE '[0-9]+' | head -n 1
)"
[[ -n "${job_timeout_minutes}" ]] || fail "could not parse timeout-minutes from ${WORKFLOW}"

# --- install step ----------------------------------------------------------

# `timeout 45m qemu-system-x86_64 ...` bounds the install phase.
install_qemu_minutes="$(
    grep -oE '^[[:space:]]*timeout[[:space:]]+[0-9]+m[[:space:]]+qemu-system-x86_64' "${INSTALL_SCRIPT}" |
        grep -oE '[0-9]+' | head -n 1
)"
[[ -n "${install_qemu_minutes}" ]] || fail "could not parse the install qemu timeout from ${INSTALL_SCRIPT}"

# The boot/update/rollback phase is bounded by SECONDS-based deadlines. Sum
# every one of them rather than taking the first match: the script has more
# than one (a short partition probe and the long lifecycle wait), they run in
# sequence within the same step, and an earlier version of this guard silently
# read the 30-second probe as the whole phase.
install_boot_seconds=0
while read -r seconds; do
    install_boot_seconds=$((install_boot_seconds + seconds))
done < <(
    grep -oE '^[a-z_]*deadline="\$\(\(SECONDS \+ [0-9]+\)\)"' "${INSTALL_SCRIPT}" |
        grep -oE 'SECONDS \+ [0-9]+' | grep -oE '[0-9]+'
)
[[ "${install_boot_seconds}" -gt 0 ]] || fail "could not parse any boot deadline from ${INSTALL_SCRIPT}"
install_boot_minutes=$(((install_boot_seconds + 59) / 60))

# The two phases run in sequence within one step.
install_worst_case=$((install_qemu_minutes + install_boot_minutes))

# --- matrix step -----------------------------------------------------------

# KVM per-profile budget; the TCG value is the fallback assignment inside the
# no-KVM branch and is not what CI runs.
matrix_profile_seconds="$(
    grep -oE '^profile_timeout_seconds=[0-9]+' "${MATRIX_SCRIPT}" |
        grep -oE '[0-9]+' | head -n 1
)"
[[ -n "${matrix_profile_seconds}" ]] || fail "could not parse the per-profile timeout from ${MATRIX_SCRIPT}"

matrix_profiles="$(
    grep -cE '^[[:space:]]*(modern-nvme|q35-sata|legacy-i440fx)\)' "${MATRIX_SCRIPT}" || true
)"
[[ "${matrix_profiles}" -gt 0 ]] || fail "could not count hardware-matrix profiles in ${MATRIX_SCRIPT}"

matrix_worst_case=$(((matrix_profile_seconds * matrix_profiles + 59) / 60))

# --- verdict ---------------------------------------------------------------

worst_case=$((FIXED_OVERHEAD_MINUTES + BUILD_RESERVE_MINUTES + install_worst_case + matrix_worst_case))

printf 'os-e2e timeout budget (KVM path)\n'
printf '  job timeout-minutes .......... %s\n' "${job_timeout_minutes}"
printf '  fixed overhead ............... %s\n' "${FIXED_OVERHEAD_MINUTES}"
printf '  ISO build reserve ............ %s (no inner timeout; bounded only by the job)\n' "${BUILD_RESERVE_MINUTES}"
printf '  install: qemu %sm + boot %sm ... %s\n' "${install_qemu_minutes}" "${install_boot_minutes}" "${install_worst_case}"
printf '  matrix: %s profiles x %ss ...... %s\n' "${matrix_profiles}" "${matrix_profile_seconds}" "${matrix_worst_case}"
printf '  worst case ................... %s\n' "${worst_case}"

if [[ "${worst_case}" -gt "${job_timeout_minutes}" ]]; then
    fail "worst case ${worst_case}m exceeds timeout-minutes ${job_timeout_minutes}m; raise the job budget or lower a step deadline (they must move together)"
fi

printf 'ANDROMEDA_CI_TIMEOUT_BUDGET_OK worst_case=%s limit=%s\n' "${worst_case}" "${job_timeout_minutes}"
