#!/usr/bin/env bash
set -euo pipefail

# Derives the os-e2e timeout budget and asserts every layer of it nests.
#
# The invariant (docs/reviews/e2e-pipeline-review.md P0 #4, re-derived for P1 #7
# when the job was split) is:
#
#   sum of per-boot budgets < host boot deadline
#   sum of a job's stage budgets < that job's timeout-minutes
#   timeout-minutes exceeds its job's worst case by at most a bounded headroom
#   the `= N min ... < N min (here)` arithmetic in os-e2e.yml's TIMEOUT BUDGET
#     INVARIANT comments equals the totals derived here
#
# (An earlier version also advertised `unit timeout < per-boot budget` as an
# invariant. It is not one: the per-boot budget is DEFINED as the unit timeout
# plus the assumed GUEST_BOOT_OVERHEAD_MINUTES, so the comparison reduces to
# "that readonly constant is positive". The check is still performed below,
# but only as a tripwire against someone editing the overhead to zero.)
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

# The fits check alone is one-sided: any regression could be "fixed" by
# bumping timeout-minutes, which silently converts a named stage timeout back
# into a 2-hour unattributable cancellation window. So a budget may exceed its
# derived worst case by at most this much; growing a stage means growing its
# derivation comment and its assumption constant, not just the outer number.
readonly MAX_HEADROOM_MINUTES=15
# Absolute ceiling on any parsed timeout-minutes, modelled or not. GitHub's
# default is 360; nothing in this pipeline should ever be allowed to run for
# more than 4 hours before someone re-derives the budget from scratch.
readonly MAX_JOB_TIMEOUT_MINUTES=240

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

    # 10# forces decimal: a parsed value with a leading zero (say `08`) would
    # otherwise be read as octal and either error or silently shrink.
    case "${unit}" in
        "" | s | sec | secs | seconds) printf '%s\n' "$((10#${value}))" ;;
        m | min | mins | minutes) printf '%s\n' "$((10#${value} * 60))" ;;
        h | hr | hrs | hours) printf '%s\n' "$((10#${value} * 3600))" ;;
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
    printf '%s\n' "$(((10#${1} + 59) / 60))"
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
    # Both patterns tolerate a trailing `# comment`, which is valid YAML on any
    # key line. This is not cosmetic for the job-id pattern: a job id the
    # parser fails to recognise is not registered, and its timeout-minutes
    # would then land in the PREVIOUS job's slot -- checking the wrong job
    # against the wrong budget while everything stays green. The write-once
    # guard on the slot below backstops any recognition gap that remains.
    local job_id_pattern='^  ([A-Za-z0-9_-]+):[[:space:]]*(#.*)?$'
    local job_timeout_pattern='^    timeout-minutes:[[:space:]]*([0-9]+)[[:space:]]*(#.*)?$'
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
            # Write-once. A second timeout-minutes landing in the same slot
            # means a job-id line BETWEEN the two was not recognised by
            # job_id_pattern (an unanticipated spelling: quoted key, unusual
            # character, changed indentation). Overwriting would silently
            # reassign that unrecognised job's budget to this one -- the
            # exact drift this guard exists to prevent -- so refuse instead.
            if [[ -n "${JOB_TIMEOUTS[${index}]}" ]]; then
                assert_banner "two timeout-minutes lines matched for job \`${job}\` in ${WORKFLOW}"
                printf '  already recorded: %s\n' "${JOB_TIMEOUTS[${index}]}" >&2
                printf '  second line:      %s\n' "${line}" >&2
                printf '  Most likely a job-id line between them was NOT recognised by this\n' >&2
                printf '  parser, so the later job was about to overwrite the budget slot of\n' >&2
                printf '  the earlier one. Teach job_id_pattern the new spelling; do not\n' >&2
                printf '  weaken this refusal.\n\n' >&2
                exit 1
            fi
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
# Parse: the derivation comments in os-e2e.yml.
#
# Each job's `TIMEOUT BUDGET INVARIANT, <job> half` comment block ends in a
# machine-shaped summary line:
#
#     #   = 71 min                                    < 75 min (here)
#
# Those comments are the human-facing copy of the arithmetic this script
# computes, and they are prose to the runner: nothing enforced them, so a
# comment edit could silently disagree with both the constants above and the
# real timeout-minutes (exactly what happened when the P1 #7 split re-derived
# them). Parsing the `= N min` total and `< N min (here)` bound out of each
# block and asserting they equal the derived worst case and the parsed
# timeout-minutes turns such an edit into a failure here.
# ---------------------------------------------------------------------------

COMMENT_TOTAL_BUILD=""
COMMENT_BOUND_BUILD=""
COMMENT_TOTAL_LIFECYCLE=""
COMMENT_BOUND_LIFECYCLE=""

parse_budget_comments() {
    local header_pattern='TIMEOUT BUDGET INVARIANT, ([a-z]+) half'
    local summary_pattern='^[[:space:]]*#[[:space:]]*=[[:space:]]*([0-9]+) min[[:space:]]+<[[:space:]]*([0-9]+) min \(here\)[[:space:]]*$'
    local line
    local half=""

    while IFS= read -r line || [[ -n "${line}" ]]; do
        if [[ ${line} =~ ${header_pattern} ]]; then
            half="${BASH_REMATCH[1]}"
            continue
        fi
        [[ ${line} =~ ${summary_pattern} ]] || continue
        case "${half}" in
            "${BUILD_JOB}")
                if [[ -n "${COMMENT_TOTAL_BUILD}" ]]; then
                    assert_banner "two budget summary lines in the ${BUILD_JOB} invariant comment of ${WORKFLOW}"
                    printf '  line: %s\n\n' "${line}" >&2
                    exit 1
                fi
                COMMENT_TOTAL_BUILD="${BASH_REMATCH[1]}"
                COMMENT_BOUND_BUILD="${BASH_REMATCH[2]}"
                ;;
            "${LIFECYCLE_JOB}")
                if [[ -n "${COMMENT_TOTAL_LIFECYCLE}" ]]; then
                    assert_banner "two budget summary lines in the ${LIFECYCLE_JOB} invariant comment of ${WORKFLOW}"
                    printf '  line: %s\n\n' "${line}" >&2
                    exit 1
                fi
                COMMENT_TOTAL_LIFECYCLE="${BASH_REMATCH[1]}"
                COMMENT_BOUND_LIFECYCLE="${BASH_REMATCH[2]}"
                ;;
            *)
                assert_banner "a budget summary line outside any recognised invariant comment in ${WORKFLOW}"
                printf '  line: %s\n' "${line}" >&2
                printf '  It is not under a "TIMEOUT BUDGET INVARIANT, %s|%s half" header,\n' \
                    "${BUILD_JOB}" "${LIFECYCLE_JOB}" >&2
                printf '  so this guard cannot tell which job it claims to describe.\n\n' >&2
                exit 1
                ;;
        esac
    done < "${WORKFLOW}"

    if [[ -z "${COMMENT_TOTAL_BUILD}" || -z "${COMMENT_TOTAL_LIFECYCLE}" ]]; then
        assert_banner "could not find both \`= N min ... < N min (here)\` summary lines in ${WORKFLOW}"
        printf '  build half found:     %s\n' "${COMMENT_TOTAL_BUILD:-no}" >&2
        printf '  lifecycle half found: %s\n' "${COMMENT_TOTAL_LIFECYCLE:-no}" >&2
        printf '  Each TIMEOUT BUDGET INVARIANT comment must keep its summary line in\n' >&2
        printf '  that shape: it is what lets this guard verify the comment arithmetic\n' >&2
        printf '  instead of letting it rot into fiction.\n\n' >&2
        exit 1
    fi
}

# assert_comment_agrees <job> <what> <comment-value> <derived-value>
assert_comment_agrees() {
    local job="$1"
    local what="$2"
    local comment_value="$3"
    local derived_value="$4"

    if ((10#${comment_value} == 10#${derived_value})); then
        return 0
    fi

    assert_banner "the ${job} invariant comment in os-e2e.yml disagrees with the derived budget"
    printf '  job:               %s\n' "${job}" >&2
    printf '  term:              %s\n' "${what}" >&2
    printf '  comment claims:    %s min   (%s)\n' "${comment_value}" "${WORKFLOW}" >&2
    printf '  this guard finds:  %s min\n' "${derived_value}" >&2
    printf '\n' >&2
    printf '  The TIMEOUT BUDGET INVARIANT comments and this guard restate the same\n' >&2
    printf '  arithmetic on purpose, so they must be edited together. Update the\n' >&2
    printf '  comment block (or the assumption constant that diverged from it);\n' >&2
    printf '  never leave the two telling different stories.\n\n' >&2
    exit 1
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
parse_budget_comments

build_timeout_minutes="$(job_timeout "${BUILD_JOB}")"
lifecycle_timeout_minutes="$(job_timeout "${LIFECYCLE_JOB}")"

# Historical drift, kept dead. Before the P1 #7 split both harness scripts
# cited the monolithic job's budget as `timeout-minutes: 180` / `(180m)`; the
# workflow had long since moved on, and those strings survived two renumbers
# unnoticed (fixed in the PR #28 review pass). Plain grep, not a parse: the
# strings must simply never reappear. (The TCG arithmetic `3 x 3600s =
# 180 min` in test-hardware-matrix.sh is a different, still-correct 180.)
if grep -Fn -e 'timeout-minutes: 180' -e '(180m)' \
    "${INSTALL_SCRIPT}" "${MATRIX_SCRIPT}" >&2; then
    assert_banner 'a harness script cites the retired 180-minute monolithic job budget'
    printf '  The offending lines are printed above. Since the job split there is\n' >&2
    printf '  no 180-minute budget anywhere; cite the per-job timeout-minutes from\n' >&2
    printf '  os-e2e.yml instead.\n\n' >&2
    exit 1
fi

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

# The lifecycle boot phase polls until this deadline. The 30 s
# `partition_probe_deadline` in the same file also sits at column zero; what
# excludes it is the `^deadline=` prefix itself, which matches only the bare
# variable name and not the `*_deadline` suffixed one.
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

# count_matches alone is bounded only from below: a marker deleted from the
# validator list (3 -> 2) would shrink the derived guest total and pass. The
# harness advertises its own boot count in the budget message it prints on
# timeout, so parse that literal and require the two to agree -- each now
# checks the other, and neither can drift alone.
parse_unique \
    'could not read the advertised guest-boot count from test-install.sh' \
    "${INSTALL_SCRIPT}" \
    'Budget check: ([0-9]+) guest boots '
advertised_guest_boots="${PARSED[1]}"
if ((10#${guest_boots} != 10#${advertised_guest_boots})); then
    assert_banner 'test-install.sh advertises a different guest-boot count than it enforces'
    printf '  DAILY_DRIVER markers counted:  %s\n' "${guest_boots}" >&2
    printf '  "Budget check:" message says:  %s\n' "${advertised_guest_boots}" >&2
    printf '  (both in %s)\n' "${INSTALL_SCRIPT}" >&2
    printf '  A lifecycle boot was added or removed without updating the budget\n' >&2
    printf '  message -- or vice versa. Update both, then re-derive the lifecycle\n' >&2
    printf '  budget in os-e2e.yml.\n\n' >&2
    exit 1
fi

# KVM per-profile budget. Anchored at column zero: the TCG fallback assignment
# is indented inside the no-KVM branch and is not what CI runs.
# The `(#.*)?` tail tolerates an end-of-line comment being folded onto the
# assignment; without it the guard hard-fails on a formatting change that
# alters no value (fail-closed, but needlessly brittle).
parse_unique \
    'could not read the per-profile timeout from test-hardware-matrix.sh' \
    "${MATRIX_SCRIPT}" \
    '^profile_timeout_seconds=([0-9]+)[[:space:]]*(#.*)?$'
profile_timeout_seconds="${PARSED[1]}"

# The profile list is the run_profile call sites, not a restated count.
matrix_profiles="$(
    count_matches \
        'could not count the hardware-matrix profiles in test-hardware-matrix.sh' \
        "${MATRIX_SCRIPT}" \
        '^run_profile[[:space:]]+[A-Za-z0-9_-]+'
)"

# Same mutual check as the guest boots: the call-site count is only bounded
# from below, but the harness's ANDROMEDA_HARDWARE_MATRIX_OK line advertises
# `scenarios=N` (and test-gcp-nested.sh greps for that exact line), so a
# commented-out run_profile now disagrees with the advertised count instead
# of silently shrinking the matrix stage.
parse_unique \
    'could not read the advertised scenario count from test-hardware-matrix.sh' \
    "${MATRIX_SCRIPT}" \
    "^printf 'ANDROMEDA_HARDWARE_MATRIX_OK scenarios=([0-9]+)"
advertised_matrix_profiles="${PARSED[1]}"
if ((10#${matrix_profiles} != 10#${advertised_matrix_profiles})); then
    assert_banner 'test-hardware-matrix.sh advertises a different profile count than it runs'
    printf '  run_profile call sites:            %s\n' "${matrix_profiles}" >&2
    printf '  ANDROMEDA_HARDWARE_MATRIX_OK says: scenarios=%s\n' \
        "${advertised_matrix_profiles}" >&2
    printf '  (both in %s)\n' "${MATRIX_SCRIPT}" >&2
    printf '  A profile was added or removed without updating the success marker\n' >&2
    printf '  -- which test-gcp-nested.sh matches verbatim -- or vice versa.\n' >&2
    printf '  Update both, then re-derive the lifecycle budget in os-e2e.yml.\n\n' >&2
    exit 1
fi
matrix_minutes="$(ceil_minutes "$((profile_timeout_seconds * matrix_profiles))")"

parse_unique \
    'could not read TimeoutStartSec from andromeda-ci-verify.service' \
    "${VERIFY_UNIT}" \
    '^TimeoutStartSec=([0-9]+)[[:space:]]*([a-z]*)[[:space:]]*(#.*)?$'
unit_timeout_seconds="$(
    to_seconds "${PARSED[1]}" "${PARSED[2]}" \
        'TimeoutStartSec in andromeda-ci-verify.service'
)"
# TimeoutStartSec=0 DISABLES the start timeout in systemd. Arithmetically it
# would flow through here as a 0-minute innermost budget and trivially nest
# inside everything, while the real guest had no unit timeout at all.
if ((unit_timeout_seconds <= 0)); then
    assert_banner 'TimeoutStartSec=0 in andromeda-ci-verify.service disables the unit timeout'
    printf '  file: %s\n' "${VERIFY_UNIT}" >&2
    printf '  systemd treats 0 as "no start timeout", not as a zero-length one,\n' >&2
    printf '  so the innermost layer of the nest would not exist and every failure\n' >&2
    printf '  inside the guest would surface as the host boot deadline instead.\n\n' >&2
    exit 1
fi
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
printf 'WORKFLOW COMMENT CROSS-CHECK (TIMEOUT BUDGET INVARIANT blocks)\n'
printf '  %-10s comment says = %3s min < %3s min   derived = %3s min < %3s min\n' \
    "${BUILD_JOB}" "${COMMENT_TOTAL_BUILD}" "${COMMENT_BOUND_BUILD}" \
    "${build_total_minutes}" "${build_timeout_minutes}"
printf '  %-10s comment says = %3s min < %3s min   derived = %3s min < %3s min\n' \
    "${LIFECYCLE_JOB}" "${COMMENT_TOTAL_LIFECYCLE}" "${COMMENT_BOUND_LIFECYCLE}" \
    "${lifecycle_total_minutes}" "${lifecycle_timeout_minutes}"
printf '\n'

# assert_job_fits <job-id> <worst case minutes> <timeout-minutes>
#
# Strictly less than, never equal: a job that exactly fills its budget has no
# room for the scheduling jitter every term here rounds away. Bounded from
# above too: a budget more than MAX_HEADROOM_MINUTES past its worst case means
# someone raised timeout-minutes without raising the stage terms, which
# reopens the unattributable-cancellation window this whole file closes.
assert_job_fits() {
    local job="$1"
    local worst_case="$2"
    local budget="$3"

    if ((budget > MAX_JOB_TIMEOUT_MINUTES)); then
        assert_banner "os-e2e job \`${job}\` timeout-minutes exceeds the absolute ceiling"
        printf '  timeout-minutes:  %s   (%s)\n' "${budget}" "${WORKFLOW}" >&2
        printf '  ceiling:          %s   (MAX_JOB_TIMEOUT_MINUTES)\n' \
            "${MAX_JOB_TIMEOUT_MINUTES}" >&2
        printf '  Nothing in this pipeline should run that long. If it genuinely\n' >&2
        printf '  must, re-derive the whole budget and raise the ceiling in the same\n' >&2
        printf '  reviewed change.\n\n' >&2
        exit 1
    fi

    if ((worst_case < budget)); then
        if ((budget - worst_case > MAX_HEADROOM_MINUTES)); then
            assert_banner "os-e2e job \`${job}\` timeout-minutes is padded far past its derivation"
            printf '  timeout-minutes:      %s   (%s)\n' "${budget}" "${WORKFLOW}" >&2
            printf '  derived worst case:   %s min\n' "${worst_case}" >&2
            printf '  headroom:             %s min (max %s, MAX_HEADROOM_MINUTES)\n' \
                "$((budget - worst_case))" "${MAX_HEADROOM_MINUTES}" >&2
            printf '\n' >&2
            printf '  "Just bump timeout-minutes" is not a fix: it converts every named\n' >&2
            printf '  stage timeout back into dead air the runner waits through before an\n' >&2
            printf '  unattributable kill. Grow the stage term (and its assumption\n' >&2
            printf '  constant and the os-e2e.yml comment) that actually got slower, and\n' >&2
            printf '  keep the outer budget snug against the sum.\n\n' >&2
            exit 1
        fi
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

# The workflow's own derivation comments restate this arithmetic for humans;
# hold them to it, so a comment edit diverging from the constants above (or
# from the real timeout-minutes) fails here instead of quietly lying.
assert_comment_agrees "${BUILD_JOB}" 'worst-case total (the "= N min" line)' \
    "${COMMENT_TOTAL_BUILD}" "${build_total_minutes}"
assert_comment_agrees "${BUILD_JOB}" 'budget bound (the "< N min (here)" term)' \
    "${COMMENT_BOUND_BUILD}" "${build_timeout_minutes}"
assert_comment_agrees "${LIFECYCLE_JOB}" 'worst-case total (the "= N min" line)' \
    "${COMMENT_TOTAL_LIFECYCLE}" "${lifecycle_total_minutes}"
assert_comment_agrees "${LIFECYCLE_JOB}" 'budget bound (the "< N min (here)" term)' \
    "${COMMENT_BOUND_LIFECYCLE}" "${lifecycle_timeout_minutes}"

printf 'ANDROMEDA_CI_TIMEOUT_BUDGET_OK %s=%s/%s %s=%s/%s guest=%s/%s\n' \
    "${BUILD_JOB}" "${build_total_minutes}" "${build_timeout_minutes}" \
    "${LIFECYCLE_JOB}" "${lifecycle_total_minutes}" "${lifecycle_timeout_minutes}" \
    "${guest_total_minutes}" "${boot_deadline_minutes}"
