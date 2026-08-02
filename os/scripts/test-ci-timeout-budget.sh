#!/usr/bin/env bash
set -euo pipefail

# Asserts the os-e2e job's `timeout-minutes` can actually contain the deadlines
# the harness enforces on itself.
#
# The failure this prevents is not a wrong number; it is the *drift* between
# numbers that live in different files and are edited by different people.
# When a step deadline grows past the job budget, the job is killed mid-step
# and the evidence upload never runs, so the run reports "cancelled" with no
# serial log — the least diagnosable failure the pipeline can produce.
#
# Every bound below is PARSED from the file that owns it rather than restated
# here. A guard that keeps its own copy of the constants is a second source of
# truth and drifts exactly like the thing it guards. That includes the two
# estimate terms (prep overhead and the ISO-build reserve): they are read from
# the BUDGET_PREP_MINUTES= / BUDGET_BUILD_MINUTES= markers inside the TIMEOUT
# BUDGET INVARIANT comment in the workflow, so the human-readable comment and
# this guard cannot disagree.
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

# Sanity ceiling for the job budget itself. Raising `timeout-minutes` is the
# documented lever when a step legitimately grows, but past this point the
# budget is no longer a tight invariant and can hide a regression (for
# example, an accidental TCG fallback) inside the slack.
readonly JOB_TIMEOUT_CEILING_MINUTES=240

# Deadlines this guard expects to find in test-install.sh: the 30 s partition
# probe and the boot/update/rollback lifecycle wait. An exact count means an
# added or removed deadline trips the guard instead of silently shifting the
# sum.
readonly EXPECTED_INSTALL_DEADLINES=2

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

# Exactly one job in the workflow, so exactly one `timeout-minutes:`. If a
# second job (or a step-level timeout) appears, `head -n 1` would silently
# pick whichever comes first in file order; refuse to guess.
timeout_lines=()
while IFS= read -r line; do
    timeout_lines+=("${line}")
done < <(grep -nE '^[[:space:]]*timeout-minutes:[[:space:]]*[0-9]+' "${WORKFLOW}" || true)
[[ "${#timeout_lines[@]}" -gt 0 ]] || fail "could not parse timeout-minutes from ${WORKFLOW}"
if [[ "${#timeout_lines[@]}" -ne 1 ]]; then
    fail "expected exactly one timeout-minutes in ${WORKFLOW}, found ${#timeout_lines[@]}: ${timeout_lines[*]}; this guard only models the single install-update-rollback job"
fi
job_timeout_minutes="$(grep -oE '[0-9]+' <<<"${timeout_lines[0]}" | tail -n 1)"

if [[ "${job_timeout_minutes}" -gt "${JOB_TIMEOUT_CEILING_MINUTES}" ]]; then
    fail "timeout-minutes ${job_timeout_minutes} exceeds the ${JOB_TIMEOUT_CEILING_MINUTES}m sanity ceiling; a budget this loose can absorb a real regression instead of surfacing it"
fi

# --- estimate terms owned by the workflow comment ---------------------------

# The two terms with no inner `timeout` live as machine-readable markers
# inside the TIMEOUT BUDGET INVARIANT comment block next to `timeout-minutes`.
budget_marker() {
    local marker="$1" matches
    matches="$(grep -oE "${marker}=[0-9]+" "${WORKFLOW}" || true)"
    [[ -n "${matches}" ]] || fail "could not find ${marker}=<minutes> in the TIMEOUT BUDGET INVARIANT comment of ${WORKFLOW}"
    [[ "$(wc -l <<<"${matches}")" -eq 1 ]] || fail "${marker} appears more than once in ${WORKFLOW}; keep a single authoritative marker"
    grep -oE '[0-9]+' <<<"${matches}"
}

# Wall-clock outside the measured steps: runner setup, free-disk-space, apt,
# shellcheck, the layer-budget guard, and artifact upload.
prep_minutes="$(budget_marker BUDGET_PREP_MINUTES)"

# The ISO build has no inner `timeout`, so nothing bounds it but the job.
# This is the reserve the budget sets aside for it; if the real build grows
# past this the guard trips, which is the intended signal.
build_minutes="$(budget_marker BUDGET_BUILD_MINUTES)"

# --- install step ----------------------------------------------------------

# `timeout 45m qemu-system-x86_64 ...` bounds the install phase.
install_qemu_minutes="$(
    grep -oE '^[[:space:]]*timeout[[:space:]]+[0-9]+m[[:space:]]+qemu-system-x86_64' "${INSTALL_SCRIPT}" |
        grep -oE '[0-9]+' | head -n 1
)"
[[ -n "${install_qemu_minutes}" ]] || fail "could not parse the install qemu timeout from ${INSTALL_SCRIPT}"

# The boot/update/rollback phase is bounded by SECONDS-based deadlines. Sum
# every one of them rather than taking the first match: they run in sequence
# within the same step, and an earlier version of this guard silently read the
# 30-second probe as the whole phase. The count is pinned so an added or
# removed deadline fails loudly instead of shifting the sum.
deadline_lines=()
while IFS= read -r line; do
    deadline_lines+=("${line}")
done < <(grep -nE '^[[:space:]]*[a-z_]*deadline="\$\(\(SECONDS \+ [0-9]+\)\)"' "${INSTALL_SCRIPT}" || true)
if [[ "${#deadline_lines[@]}" -ne "${EXPECTED_INSTALL_DEADLINES}" ]]; then
    fail "expected exactly ${EXPECTED_INSTALL_DEADLINES} SECONDS-based deadlines in ${INSTALL_SCRIPT} (the partition probe and the lifecycle wait), found ${#deadline_lines[@]}: ${deadline_lines[*]:-none}; if a deadline was added or removed, update EXPECTED_INSTALL_DEADLINES so the sum stays audited"
fi
install_boot_seconds=0
for deadline_line in "${deadline_lines[@]}"; do
    seconds="$(grep -oE 'SECONDS \+ [0-9]+' <<<"${deadline_line}" | grep -oE '[0-9]+')"
    install_boot_seconds=$((install_boot_seconds + seconds))
done
install_boot_minutes=$(((install_boot_seconds + 59) / 60))

# The two phases run in sequence within one step.
install_worst_case=$((install_qemu_minutes + install_boot_minutes))

# --- matrix step -----------------------------------------------------------

# KVM per-profile budget; the TCG value is the fallback assignment inside the
# no-KVM branch (indented, so the anchored grep skips it) and is not what CI
# runs.
matrix_profile_seconds="$(
    grep -oE '^profile_timeout_seconds=[0-9]+' "${MATRIX_SCRIPT}" |
        grep -oE '[0-9]+' | head -n 1
)"
[[ -n "${matrix_profile_seconds}" ]] || fail "could not parse the per-profile timeout from ${MATRIX_SCRIPT}"

# Count the actual `run_profile <name> ...` invocations rather than case arms,
# so adding a profile is counted no matter how its name is spelled.
matrix_profiles="$(grep -cE '^run_profile ' "${MATRIX_SCRIPT}" || true)"
[[ "${matrix_profiles}" -gt 0 ]] || fail "could not count run_profile invocations in ${MATRIX_SCRIPT}"

matrix_worst_case=$(((matrix_profile_seconds * matrix_profiles + 59) / 60))

# --- hardcoded-copy drift check --------------------------------------------

# test-install.sh's no-KVM error message quotes the job budget so the operator
# reading it knows what TCG would blow through. That message is the last
# hardcoded copy of the budget in the tree; keep it honest.
tcg_message_minutes="$(
    grep -oE 'os-e2e job timeout \([0-9]+m\)' "${INSTALL_SCRIPT}" |
        grep -oE '\([0-9]+m\)' | grep -oE '[0-9]+' || true
)"
[[ -n "${tcg_message_minutes}" ]] || fail "could not find the 'os-e2e job timeout (<N>m)' TCG fallback message in ${INSTALL_SCRIPT}"
[[ "$(wc -l <<<"${tcg_message_minutes}")" -eq 1 ]] || fail "found more than one 'os-e2e job timeout (<N>m)' message in ${INSTALL_SCRIPT}"
if [[ "${tcg_message_minutes}" -ne "${job_timeout_minutes}" ]]; then
    fail "the TCG fallback message in ${INSTALL_SCRIPT} says the job timeout is ${tcg_message_minutes}m but the workflow sets ${job_timeout_minutes}m; update the message"
fi

# --- verdict ---------------------------------------------------------------

worst_case=$((prep_minutes + build_minutes + install_worst_case + matrix_worst_case))
headroom=$((job_timeout_minutes - worst_case))

printf 'os-e2e timeout budget (KVM path)\n'
printf '  job timeout-minutes .......... %s\n' "${job_timeout_minutes}"
printf '  prep overhead ................ %s (BUDGET_PREP_MINUTES, workflow comment)\n' "${prep_minutes}"
printf '  ISO build reserve ............ %s (BUDGET_BUILD_MINUTES, workflow comment; no inner timeout)\n' "${build_minutes}"
printf '  install: qemu %sm + boot %sm ... %s\n' "${install_qemu_minutes}" "${install_boot_minutes}" "${install_worst_case}"
printf '  matrix: %s profiles x %ss ...... %s\n' "${matrix_profiles}" "${matrix_profile_seconds}" "${matrix_worst_case}"
printf '  worst case ................... %s\n' "${worst_case}"
printf '  headroom ..................... %s\n' "${headroom}"

if [[ "${headroom}" -lt 1 ]]; then
    fail "worst case ${worst_case}m leaves ${headroom}m headroom under timeout-minutes ${job_timeout_minutes}m; raise the job budget or lower a step deadline (they must move together)"
fi

printf 'ANDROMEDA_CI_TIMEOUT_BUDGET_OK worst_case=%s limit=%s headroom=%s\n' "${worst_case}" "${job_timeout_minutes}" "${headroom}"
