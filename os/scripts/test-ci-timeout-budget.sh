#!/usr/bin/env bash
set -euo pipefail

# Derives the os-e2e timeout budget and asserts every layer of it nests.
#
# The invariant (docs/reviews/e2e-pipeline-review.md P0 #4, re-derived for P1 #7
# when the job was split) is:
#
#   unit timeout < per-boot budget
#   sum of per-boot budgets < host boot deadline
#   sum of a job's stage budgets < that job's timeout-minutes
#
# The failure this prevents is not a wrong number, it is DRIFT between numbers
# that live in four different files and are edited by different people at
# different times. When a step deadline grows past the job budget, the runner
# kills the job mid-step, the `if: always()` evidence upload never runs, and the
# run reports "cancelled" with no serial log -- the least diagnosable failure
# this pipeline can produce.
#
# WHAT IS PARSED AND WHAT IS ASSUMED
#
# Every bound that some file already enforces at runtime is PARSED from that
# file. Restating such a bound here would create a second source of truth that
# drifts exactly like the thing it guards, so this script does not do it:
#
#   .github/workflows/os-e2e.yml    timeout-minutes, per job id
#   os/scripts/test-install.sh      `timeout 45m qemu-...`, boot deadline,
#                                   guest boot count
#   os/scripts/test-hardware-matrix.sh  per-profile timeout, profile count
#   .../andromeda-ci-verify.service TimeoutStartSec
#
# The remaining terms are wall-clock that NOTHING bounds: runner setup, apt,
# artifact transfer, and above all the ISO build, which has no inner `timeout`
# at all. There is no file to parse them out of, so they are explicit named
# ASSUMPTIONS with a single definition point each, in the two blocks below.
# They are assumptions, not measurements and not enforced ceilings; each carries
# the observed value it was calibrated against. They intentionally hold the same
# values as the per-job derivation comments in os-e2e.yml, so this guard and
# those comments must be edited together -- which is the point.
#
# WHY PER JOB. Since the P1 #7 split there is no single "the E2E job". The build
# reserve is incurred by `build`; the install, boot, and matrix deadlines are
# incurred by `lifecycle`. Summing them and comparing the total against either
# job's timeout-minutes is wrong in both directions, so each job is checked
# against its own budget with only the terms it actually pays for.
#
# SCOPE: the GitHub jobs only. os/scripts/test-gcp-nested.sh wraps the same
# scripts in its own `timeout` values for an operator-run GCP VM bounded by the
# instance --max-run-duration, and is never invoked from a workflow.
#
# No YAML library is used: PyYAML is not in the Python standard library and is
# not guaranteed on a hosted runner, and this guard exists precisely to run
# everywhere cheaply. The workflow is scanned with an indentation-aware line
# parser that fails closed -- if it cannot find a job or its timeout, it errors
# instead of guessing.

ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly ANDROMEDA_SCRIPT_DIR
# shellcheck source=os/scripts/lib/assert.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/assert.sh"

REPOSITORY_ROOT="$(cd "${ANDROMEDA_SCRIPT_DIR}/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly WORKFLOW="${REPOSITORY_ROOT}/.github/workflows/os-e2e.yml"
readonly INSTALL_SCRIPT="${REPOSITORY_ROOT}/os/scripts/test-install.sh"
readonly MATRIX_SCRIPT="${REPOSITORY_ROOT}/os/scripts/test-hardware-matrix.sh"
readonly VERIFY_UNIT="${REPOSITORY_ROOT}/os/files/usr/lib/systemd/system/andromeda-ci-verify.service"

readonly BUILD_JOB=build
readonly LIFECYCLE_JOB=lifecycle

# ---------------------------------------------------------------------------
# ASSUMPTIONS, build job. Nothing in the repository bounds any of these; they
# are the values the `TIMEOUT BUDGET INVARIANT, build half` comment in
# os-e2e.yml derives from, restated once here because there is no file that
# owns them. Observed figures are from the three successful runs tabulated in
# docs/reviews/e2e-pipeline-review.md.
# ---------------------------------------------------------------------------

# free-disk-space action (observed 2.10-4.50 min; the 4.50 is run 30732275955,
# which is why this is 5 and not the 4 it started at).
readonly BUILD_FREE_DISK_MINUTES=5
# apt-get update + podman/runc/shellcheck (observed 0.21-0.36 min).
readonly BUILD_APT_MINUTES=2
# The validate-scripts step: shellcheck, platform guard, layer-count floor
# (observed 0.01-0.03 min). Do not start this comment with the word shellcheck;
# a comment whose first token is that word is read as a shellcheck directive.
readonly BUILD_VALIDATE_MINUTES=1
# The ISO build. THE one term with no enforced ceiling anywhere: build-iso.sh
# has no inner `timeout`, so only the job budget stops it. Observed max 42.25
# min cold; a warm payload layer cache is expected to reach ~18 min, but the
# cold path (every fork PR, every week rollover, every cache miss) is supported
# and must still fit, so this reserve is sized for cold. If a cold build
# regresses past this, this number and the job budget move together.
readonly BUILD_IMAGE_RESERVE_MINUTES=50
# `podman push` of the payload layer cache to ghcr (P1 #6, no history yet).
readonly BUILD_CACHE_PUBLISH_MINUTES=5
# Layer-size budget against the produced history JSON.
readonly BUILD_LAYER_BUDGET_MINUTES=1
# Upload of the 4.0 GiB ISO plus the ~4 GiB rev2 OCI archive (ISO alone
# observed 0.31-0.74 min; the rev2 tar is new with the split and unmeasured).
readonly BUILD_UPLOAD_MINUTES=6
# actions/checkout plus the ghcr login.
readonly BUILD_CHECKOUT_MINUTES=1

# ---------------------------------------------------------------------------
# ASSUMPTIONS, lifecycle job. Same status: unbounded wall-clock with no owning
# file. Matches the `TIMEOUT BUDGET INVARIANT, lifecycle half` comment.
# ---------------------------------------------------------------------------

# Same action, same observed 4.50 min worst case (run 30732275955).
readonly LIFECYCLE_FREE_DISK_MINUTES=5
# apt-get update + binutils/ovmf/qemu-system-x86/qemu-utils.
readonly LIFECYCLE_APT_MINUTES=2
# Download of both build artifacts, ~8 GiB total, plus the sha256 check.
readonly LIFECYCLE_DOWNLOAD_MINUTES=6
# Serial evidence upload, ~2 MiB, `if: always()`.
readonly LIFECYCLE_EVIDENCE_UPLOAD_MINUTES=1
readonly LIFECYCLE_CHECKOUT_MINUTES=1

# ---------------------------------------------------------------------------
# ASSUMPTION, guest side. The only unparseable term in the guest nest.
# ---------------------------------------------------------------------------

# Firmware, kernel, sddm and plasma startup before andromeda-ci-verify.service
# is even allowed to start (it is After= sddm.service). Nothing enforces this,
# so it is an assumption; it is the "+ ~2 min" in the unit file's own comment.
readonly GUEST_BOOT_OVERHEAD_MINUTES=2

# NOT COUNTED, deliberately, so a future reader knows it was considered rather
# than missed: test-install.sh also has a 30 s `partition_probe_deadline` for
# the qemu-nbd partition settle. It runs inside the same step as the two big
# QEMU phases, is half a minute against a 135 minute total, and is absorbed by
# the whole-minute rounding of every term around it. An earlier version of this
# guard summed every `*deadline=` assignment in the file and reported the boot
# phase as 46 rather than 45 minutes, which then disagreed with the workflow
# comment it was supposed to be checking.

# ---------------------------------------------------------------------------
# Parsing helpers. Both fail closed and both refuse to take the first of
# several matches: a positional `head -n 1` is how a guard silently starts
# checking the wrong number.
# ---------------------------------------------------------------------------

# parse_unique <description> <file> <ere>
#
# Requires EXACTLY one line of <file> to match <ere> and leaves that line's
# capture groups in PARSED[1..n].
parse_unique() {
    local description="$1"
    local file="$2"
    local pattern="$3"
    local line
    local matches=0

    while IFS= read -r line || [[ -n "${line}" ]]; do
        if [[ ${line} =~ ${pattern} ]]; then
            matches=$((matches + 1))
            PARSED=("${BASH_REMATCH[@]}")
        fi
    done < "${file}"

    if ((matches != 1)); then
        assert_banner "${description}"
        printf '  file:           %s\n' "${file}" >&2
        printf '  pattern:        %s\n' "${pattern}" >&2
        printf '  matching lines: %s (expected exactly 1)\n' "${matches}" >&2
        printf '  This guard will not read the first of several matches: it would\n' >&2
        printf '  then silently check a bound nobody meant it to check.\n\n' >&2
        exit 1
    fi
}

# count_matches <description> <file> <ere>
#
# Prints how many lines of <file> match <ere>; requires at least one.
count_matches() {
    local description="$1"
    local file="$2"
    local pattern="$3"
    local line
    local matches=0

    while IFS= read -r line || [[ -n "${line}" ]]; do
        if [[ ${line} =~ ${pattern} ]]; then
            matches=$((matches + 1))
        fi
    done < "${file}"

    if ((matches < 1)); then
        assert_banner "${description}"
        printf '  file:    %s\n' "${file}" >&2
        printf '  pattern: %s\n' "${pattern}" >&2
        printf '  no line matched; the guard cannot derive this term.\n\n' >&2
        exit 1
    fi
    printf '%s\n' "${matches}"
}

# to_seconds <value> <unit> <description>
#
# Converts a duration written with a shell/systemd suffix. An empty unit means
# seconds, which is what systemd does with a bare number. An unrecognised unit
# is a hard failure rather than a guess.
to_seconds() {
    local value="$1"
    local unit="$2"
    local description="$3"

    case "${unit}" in
        "" | s | sec | secs | seconds) printf '%s\n' "$((value))" ;;
        m | min | mins | minutes) printf '%s\n' "$((value * 60))" ;;
        h | hr | hrs | hours) printf '%s\n' "$((value * 3600))" ;;
        *)
            assert_banner "${description}"
            printf '  unrecognised duration unit: %s%s\n' "${value}" "${unit}" >&2
            printf '  Teach to_seconds() this unit rather than letting the guard guess.\n\n' >&2
            exit 1
            ;;
    esac
}

# ceil_minutes <seconds>
ceil_minutes() {
    printf '%s\n' "$((($1 + 59) / 60))"
}

# ---------------------------------------------------------------------------
# Parse: timeout-minutes per job id.
#
# Indentation-aware, because position is not identity: the file has two jobs and
# `head -n 1` would silently return the build job's budget for every question
# asked of it. A job-level key sits at exactly four spaces; a step-level one
# would sit at eight, and a commented-out one is skipped outright.
# ---------------------------------------------------------------------------

JOB_IDS=()
JOB_TIMEOUTS=()

parse_job_timeouts() {
    local comment_pattern='^[[:space:]]*#'
    local jobs_key_pattern='^jobs:[[:space:]]*$'
    local top_level_key_pattern='^[^[:space:]#]'
    local job_id_pattern='^  ([A-Za-z0-9_-]+):[[:space:]]*$'
    local job_timeout_pattern='^    timeout-minutes:[[:space:]]*([0-9]+)[[:space:]]*$'
    local line
    local in_jobs=0
    local job=""
    local index

    while IFS= read -r line || [[ -n "${line}" ]]; do
        [[ ${line} =~ ${comment_pattern} ]] && continue
        if [[ ${line} =~ ${jobs_key_pattern} ]]; then
            in_jobs=1
            job=""
            continue
        fi
        ((in_jobs)) || continue
        if [[ ${line} =~ ${top_level_key_pattern} ]]; then
            in_jobs=0
            job=""
            continue
        fi
        if [[ ${line} =~ ${job_id_pattern} ]]; then
            job="${BASH_REMATCH[1]}"
            JOB_IDS+=("${job}")
            JOB_TIMEOUTS+=("")
            continue
        fi
        if [[ ${line} =~ ${job_timeout_pattern} ]]; then
            if [[ -z "${job}" ]]; then
                assert_banner "found a job-level timeout-minutes outside any job in ${WORKFLOW}"
                printf '  line: %s\n\n' "${line}" >&2
                exit 1
            fi
            index=$((${#JOB_IDS[@]} - 1))
            JOB_TIMEOUTS[index]="${BASH_REMATCH[1]}"
        fi
    done < "${WORKFLOW}"

    if ((${#JOB_IDS[@]} == 0)); then
        assert_banner "could not find any job under the jobs: key in ${WORKFLOW}"
        printf '  The workflow layout changed in a way this parser does not understand.\n' >&2
        printf '  Fix the parser rather than dropping the check.\n\n' >&2
        exit 1
    fi
}

# job_timeout <job-id>
job_timeout() {
    local wanted="$1"
    local index

    for index in "${!JOB_IDS[@]}"; do
        if [[ "${JOB_IDS[${index}]}" == "${wanted}" ]]; then
            if [[ -z "${JOB_TIMEOUTS[${index}]}" ]]; then
                assert_banner "job \`${wanted}\` in ${WORKFLOW} has no timeout-minutes"
                printf '  Without one the job inherits the GitHub default of 360 minutes,\n' >&2
                printf '  which is far outside the budget this invariant maintains.\n\n' >&2
                exit 1
            fi
            printf '%s\n' "${JOB_TIMEOUTS[${index}]}"
            return 0
        fi
    done

    assert_banner "job \`${wanted}\` does not exist in ${WORKFLOW}"
    printf '  jobs found: %s\n' "${JOB_IDS[*]}" >&2
    printf '  This guard models the cost of each job by name. A renamed or removed\n' >&2
    printf '  job needs its budget re-derived, not silently skipped.\n\n' >&2
    exit 1
}

# Every job must be modelled. A third job added later would otherwise get a
# timeout-minutes nobody ever checks, which is exactly the drift this guard is
# for.
assert_every_job_modelled() {
    local index
    local job

    for index in "${!JOB_IDS[@]}"; do
        job="${JOB_IDS[${index}]}"
        case "${job}" in
            "${BUILD_JOB}" | "${LIFECYCLE_JOB}") ;;
            *)
                assert_banner "os-e2e job \`${job}\` has no budget model in test-ci-timeout-budget.sh"
                printf '  jobs in workflow: %s\n' "${JOB_IDS[*]}" >&2
                printf '  modelled here:    %s %s\n' "${BUILD_JOB}" "${LIFECYCLE_JOB}" >&2
                printf '  Add the stage costs of the new job above and check them here, or\n' >&2
                printf '  its timeout-minutes is a number nothing verifies.\n\n' >&2
                exit 1
                ;;
        esac
    done
}

# ---------------------------------------------------------------------------
# Collect every bound.
# ---------------------------------------------------------------------------

require_file "${WORKFLOW}" "the os-e2e workflow is the file that owns the per-job timeout-minutes"
require_file "${INSTALL_SCRIPT}" "test-install.sh owns the install timeout and the boot deadline"
require_file "${MATRIX_SCRIPT}" "test-hardware-matrix.sh owns the per-profile timeout and the profile list"
require_file "${VERIFY_UNIT}" "the ci-verify unit owns TimeoutStartSec, the innermost budget of the nest"

parse_job_timeouts
assert_every_job_modelled

build_timeout_minutes="$(job_timeout "${BUILD_JOB}")"
lifecycle_timeout_minutes="$(job_timeout "${LIFECYCLE_JOB}")"

# `timeout 45m qemu-system-x86_64 ...` bounds the Anaconda install phase.
parse_unique \
    'could not read the install-phase qemu timeout from test-install.sh' \
    "${INSTALL_SCRIPT}" \
    '^timeout[[:space:]]+([0-9]+)([a-z]*)[[:space:]]+qemu-system-x86_64'
install_timeout_seconds="$(
    to_seconds "${PARSED[1]}" "${PARSED[2]}" \
        'the install-phase qemu timeout in test-install.sh'
)"
install_timeout_minutes="$(ceil_minutes "${install_timeout_seconds}")"

# The lifecycle boot phase polls until this deadline. Anchored on the bare
# `deadline` variable at column zero so it cannot pick up the indented 30 s
# `partition_probe_deadline` in the same file.
parse_unique \
    'could not read the lifecycle boot deadline from test-install.sh' \
    "${INSTALL_SCRIPT}" \
    '^deadline="[$][(][(]SECONDS[[:space:]]*[+][[:space:]]*([0-9]+)[)][)]"'
boot_deadline_seconds="${PARSED[1]}"
boot_deadline_minutes="$(ceil_minutes "${boot_deadline_seconds}")"

# One guest boot per daily-driver checkpoint. test-install.sh's validator
# requires each of these markers exactly once and in order, so this list is the
# authoritative count of lifecycle boots (first-boot, updating, rolling-back),
# and a fourth boot cannot be added without appearing here.
guest_boots="$(
    count_matches \
        'could not count the guest lifecycle boots in test-install.sh' \
        "${INSTALL_SCRIPT}" \
        '^[[:space:]]*"ANDROMEDA_DAILY_DRIVER_OK phase='
)"

# KVM per-profile budget. Anchored at column zero: the TCG fallback assignment
# is indented inside the no-KVM branch and is not what CI runs.
parse_unique \
    'could not read the per-profile timeout from test-hardware-matrix.sh' \
    "${MATRIX_SCRIPT}" \
    '^profile_timeout_seconds=([0-9]+)[[:space:]]*$'
profile_timeout_seconds="${PARSED[1]}"

# The profile list is the run_profile call sites, not a restated count.
matrix_profiles="$(
    count_matches \
        'could not count the hardware-matrix profiles in test-hardware-matrix.sh' \
        "${MATRIX_SCRIPT}" \
        '^run_profile[[:space:]]+[A-Za-z0-9_-]+'
)"
matrix_minutes="$(ceil_minutes "$((profile_timeout_seconds * matrix_profiles))")"

parse_unique \
    'could not read TimeoutStartSec from andromeda-ci-verify.service' \
    "${VERIFY_UNIT}" \
    '^TimeoutStartSec=([0-9]+)[[:space:]]*([a-z]*)[[:space:]]*$'
unit_timeout_seconds="$(
    to_seconds "${PARSED[1]}" "${PARSED[2]}" \
        'TimeoutStartSec in andromeda-ci-verify.service'
)"
unit_timeout_minutes="$(ceil_minutes "${unit_timeout_seconds}")"

# ---------------------------------------------------------------------------
# Derive.
# ---------------------------------------------------------------------------

per_boot_budget_minutes=$((unit_timeout_minutes + GUEST_BOOT_OVERHEAD_MINUTES))
guest_total_minutes=$((guest_boots * per_boot_budget_minutes))

build_total_minutes=$((
    BUILD_CHECKOUT_MINUTES
    + BUILD_FREE_DISK_MINUTES
    + BUILD_APT_MINUTES
    + BUILD_VALIDATE_MINUTES
    + BUILD_IMAGE_RESERVE_MINUTES
    + BUILD_CACHE_PUBLISH_MINUTES
    + BUILD_LAYER_BUDGET_MINUTES
    + BUILD_UPLOAD_MINUTES
))

lifecycle_total_minutes=$((
    LIFECYCLE_CHECKOUT_MINUTES
    + LIFECYCLE_FREE_DISK_MINUTES
    + LIFECYCLE_APT_MINUTES
    + LIFECYCLE_DOWNLOAD_MINUTES
    + install_timeout_minutes
    + boot_deadline_minutes
    + matrix_minutes
    + LIFECYCLE_EVIDENCE_UPLOAD_MINUTES
))

# ---------------------------------------------------------------------------
# Report, then judge. The report is printed unconditionally and before any
# verdict so that a failing run already carries its own derivation: the first
# screen of CI output must be enough to see which term grew.
# ---------------------------------------------------------------------------

printf 'os-e2e timeout budget, KVM path\n'
printf '\n'
printf 'PARSED BOUNDS (read from the file that enforces them)\n'
printf '  %s\n' ".github/workflows/os-e2e.yml"
printf '    job %-22s timeout-minutes  %3s\n' "${BUILD_JOB}" "${build_timeout_minutes}"
printf '    job %-22s timeout-minutes  %3s\n' "${LIFECYCLE_JOB}" "${lifecycle_timeout_minutes}"
printf '  %s\n' "os/scripts/test-install.sh"
printf '    install phase qemu timeout            %5s s = %3s min\n' \
    "${install_timeout_seconds}" "${install_timeout_minutes}"
printf '    lifecycle boot deadline               %5s s = %3s min\n' \
    "${boot_deadline_seconds}" "${boot_deadline_minutes}"
printf '    guest boots per lifecycle             %5s\n' "${guest_boots}"
printf '  %s\n' "os/scripts/test-hardware-matrix.sh"
printf '    per-profile timeout                   %5s s\n' "${profile_timeout_seconds}"
printf '    profiles (run_profile call sites)     %5s\n' "${matrix_profiles}"
printf '    matrix stage                          %5s s = %3s min\n' \
    "$((profile_timeout_seconds * matrix_profiles))" "${matrix_minutes}"
printf '  %s\n' "os/files/usr/lib/systemd/system/andromeda-ci-verify.service"
printf '    TimeoutStartSec                       %5s s = %3s min\n' \
    "${unit_timeout_seconds}" "${unit_timeout_minutes}"
printf '\n'
printf 'GUEST-SIDE NEST\n'
printf '  TimeoutStartSec                       %3s min\n' "${unit_timeout_minutes}"
printf '  + boot overhead (ASSUMED)             %3s min\n' "${GUEST_BOOT_OVERHEAD_MINUTES}"
printf '  = per-boot budget                     %3s min\n' "${per_boot_budget_minutes}"
printf '  x %s guest boots                       %3s min  vs boot deadline %s min\n' \
    "${guest_boots}" "${guest_total_minutes}" "${boot_deadline_minutes}"
printf '\n'
printf 'JOB %s\n' "${BUILD_JOB}"
printf '  checkout + ghcr login       (ASSUMED) %3s min\n' "${BUILD_CHECKOUT_MINUTES}"
printf '  free runner disk space      (ASSUMED) %3s min\n' "${BUILD_FREE_DISK_MINUTES}"
printf '  apt install host deps       (ASSUMED) %3s min\n' "${BUILD_APT_MINUTES}"
printf '  validate scripts            (ASSUMED) %3s min\n' "${BUILD_VALIDATE_MINUTES}"
printf '  build payloads + ISO        (ASSUMED) %3s min  <- no inner timeout exists\n' \
    "${BUILD_IMAGE_RESERVE_MINUTES}"
printf '  publish layer cache to ghcr (ASSUMED) %3s min\n' "${BUILD_CACHE_PUBLISH_MINUTES}"
printf '  layer-size budget           (ASSUMED) %3s min\n' "${BUILD_LAYER_BUDGET_MINUTES}"
printf '  upload ISO + rev2 payload   (ASSUMED) %3s min\n' "${BUILD_UPLOAD_MINUTES}"
printf '  = worst case                          %3s min  vs timeout-minutes %s\n' \
    "${build_total_minutes}" "${build_timeout_minutes}"
printf '\n'
printf 'JOB %s\n' "${LIFECYCLE_JOB}"
printf '  checkout                    (ASSUMED) %3s min\n' "${LIFECYCLE_CHECKOUT_MINUTES}"
printf '  free runner disk space      (ASSUMED) %3s min\n' "${LIFECYCLE_FREE_DISK_MINUTES}"
printf '  apt install host deps       (ASSUMED) %3s min\n' "${LIFECYCLE_APT_MINUTES}"
printf '  download ISO + rev2 payload (ASSUMED) %3s min\n' "${LIFECYCLE_DOWNLOAD_MINUTES}"
printf '  test-install.sh install      (parsed) %3s min\n' "${install_timeout_minutes}"
printf '  test-install.sh boot         (parsed) %3s min\n' "${boot_deadline_minutes}"
printf '  test-hardware-matrix.sh      (parsed) %3s min\n' "${matrix_minutes}"
printf '  upload serial evidence      (ASSUMED) %3s min\n' "${LIFECYCLE_EVIDENCE_UPLOAD_MINUTES}"
printf '  = worst case                          %3s min  vs timeout-minutes %s\n' \
    "${lifecycle_total_minutes}" "${lifecycle_timeout_minutes}"
printf '\n'

# assert_job_fits <job-id> <worst case minutes> <timeout-minutes>
#
# Strictly less than, never equal: a job that exactly fills its budget has no
# room for the scheduling jitter every term here rounds away.
assert_job_fits() {
    local job="$1"
    local worst_case="$2"
    local budget="$3"

    if ((worst_case < budget)); then
        return 0
    fi

    assert_banner "os-e2e job \`${job}\` cannot contain the deadlines its own steps enforce"
    printf '  job:                  %s\n' "${job}" >&2
    printf '  timeout-minutes:      %s   (%s)\n' "${budget}" "${WORKFLOW}" >&2
    printf '  derived worst case:   %s min\n' "${worst_case}" >&2
    printf '  overshoot:            %s min\n' "$((worst_case - budget + 1))" >&2
    printf '\n' >&2
    printf '  The per-term derivation is printed above this banner; the term that\n' >&2
    printf '  grew is the one to compare against os-e2e.yml.\n' >&2
    printf '\n' >&2
    printf '  When a job is killed by its own timeout the always() evidence upload\n' >&2
    printf '  never runs, so the failure reports as "cancelled" with no serial log.\n' >&2
    printf '  Either raise timeout-minutes for job %s, updating the TIMEOUT BUDGET\n' "${job}" >&2
    printf '  INVARIANT arithmetic in os-e2e.yml in the same change, or lower the\n' >&2
    printf '  step deadline that grew. The two must move together.\n\n' >&2
    exit 1
}

if ((unit_timeout_minutes >= per_boot_budget_minutes)); then
    assert_banner 'the ci-verify unit timeout does not fit inside a single guest boot'
    printf '  TimeoutStartSec:    %s min   (%s)\n' \
        "${unit_timeout_minutes}" "${VERIFY_UNIT}" >&2
    printf '  per-boot budget:    %s min\n' "${per_boot_budget_minutes}" >&2
    printf '  boot overhead:      %s min (assumed, GUEST_BOOT_OVERHEAD_MINUTES)\n' \
        "${GUEST_BOOT_OVERHEAD_MINUTES}" >&2
    printf '\n' >&2
    printf '  The per-boot budget is the unit timeout plus the firmware/kernel/sddm\n' >&2
    printf '  startup that precedes it, so a non-positive boot overhead would flatten\n' >&2
    printf '  the nest and let the unit claim the whole boot.\n\n' >&2
    exit 1
fi

if ((guest_total_minutes >= boot_deadline_minutes)); then
    assert_banner 'the guest lifecycle cannot finish inside the host boot deadline'
    printf '  TimeoutStartSec:      %s min   (%s)\n' \
        "${unit_timeout_minutes}" "${VERIFY_UNIT}" >&2
    printf '  + boot overhead:      %s min   (assumed)\n' \
        "${GUEST_BOOT_OVERHEAD_MINUTES}" >&2
    printf '  = per-boot budget:    %s min\n' "${per_boot_budget_minutes}" >&2
    printf '  x %s guest boots:      %s min\n' \
        "${guest_boots}" "${guest_total_minutes}" >&2
    printf '  host boot deadline:   %s min (%s s)   (%s)\n' \
        "${boot_deadline_minutes}" "${boot_deadline_seconds}" "${INSTALL_SCRIPT}" >&2
    printf '\n' >&2
    printf '  A slow but healthy guest would be killed by the host poller with no\n' >&2
    printf '  unit-level diagnostic: the serial log would simply stop. Raising\n' >&2
    printf '  TimeoutStartSec REQUIRES raising the boot deadline in test-install.sh,\n' >&2
    printf '  which in turn feeds the %s job budget checked above.\n\n' \
        "${LIFECYCLE_JOB}" >&2
    exit 1
fi

assert_job_fits "${BUILD_JOB}" "${build_total_minutes}" "${build_timeout_minutes}"
assert_job_fits "${LIFECYCLE_JOB}" "${lifecycle_total_minutes}" "${lifecycle_timeout_minutes}"

printf 'ANDROMEDA_CI_TIMEOUT_BUDGET_OK %s=%s/%s %s=%s/%s guest=%s/%s\n' \
    "${BUILD_JOB}" "${build_total_minutes}" "${build_timeout_minutes}" \
    "${LIFECYCLE_JOB}" "${lifecycle_total_minutes}" "${lifecycle_timeout_minutes}" \
    "${guest_total_minutes}" "${boot_deadline_minutes}"
