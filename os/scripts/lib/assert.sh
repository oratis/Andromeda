#!/usr/bin/env bash
# Shared assertion helpers for the os/scripts E2E harnesses.
#
# Why this exists: a bare `test -f "${OVMF_CODE}"` or `grep -q MARKER log` under
# `set -e` fails with an EMPTY message. CI then shows nothing but `exit code 1`
# and an operator has to bisect the script by hand to learn which precondition
# blew up. See docs/reviews/e2e-pipeline-review.md P0 #2 (and the run
# 30686771448 post-mortem in P0 #1, where a real installer failure surfaced as a
# bare `exit code 1`).
#
# Every helper here prints WHAT was expected, WHAT ran, and WHERE to look, then
# exits non-zero. Callers keep `set -e`; these helpers exit on their own so a
# missed `||` cannot silently swallow a failed assertion.
#
# Source it from a sibling script with:
#     ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#     # shellcheck source=os/scripts/lib/assert.sh
#     # shellcheck disable=SC1091
#     . "${ANDROMEDA_SCRIPT_DIR}/lib/assert.sh"
# which resolves correctly from CI, from `sudo os/scripts/...`, and from a
# checkout unpacked anywhere (the GCP path untars the repo into ~/andromeda).

# Idempotent: sourcing twice must not clobber anything.
if [[ -n "${ANDROMEDA_ASSERT_SH_LOADED:-}" ]]; then
    return 0
fi
readonly ANDROMEDA_ASSERT_SH_LOADED=1

# Number of trailing log lines printed as context by require_marker.
: "${ANDROMEDA_ASSERT_LOG_TAIL:=60}"

# Print a uniform, greppable failure banner on stderr.
#   assert_banner <description>
assert_banner() {
    printf '\n' >&2
    printf '================================================================\n' >&2
    printf 'ASSERT FAILED: %s\n' "$1" >&2
    printf '================================================================\n' >&2
}

# require <human description> <command...>
#
# Runs <command...>; on a non-zero exit it prints the description, the exact
# command (shell-quoted, so the actual expanded values are visible) and the exit
# status, then exits with that status.
require() {
    local description="$1"
    local status=0
    shift

    if (( $# == 0 )); then
        assert_banner "${description}"
        printf '  require() was called without a command to run.\n' >&2
        exit 2
    fi

    "$@" || status="$?"
    if (( status == 0 )); then
        return 0
    fi

    assert_banner "${description}"
    printf '  command: ' >&2
    printf '%q ' "$@" >&2
    printf '\n  exit status: %s\n\n' "${status}" >&2
    exit "${status}"
}

# require_file <path> <human description>
#
# Asserts that <path> exists and is a regular file. On failure it also lists the
# parent directory, which is almost always the next thing an operator wants
# ("the ISO is missing -- so what DID the build produce?").
require_file() {
    local path="$1"
    local description="${2:-required file}"
    local parent

    if [[ -f "${path}" ]]; then
        return 0
    fi

    assert_banner "${description}"
    printf '  expected an existing file: %s\n' "${path}" >&2
    parent="$(dirname "${path}")"
    if [[ -d "${parent}" ]]; then
        printf '  contents of %s:\n' "${parent}" >&2
        # shellcheck disable=SC2012 # Human-readable diagnostic listing only;
        # nothing parses this and `ls -la` reads better than find -printf here.
        ls -la "${parent}" 2>&1 | sed 's/^/    /' >&2 || true
    else
        printf '  parent directory does not exist either: %s\n' "${parent}" >&2
    fi
    printf '\n' >&2
    exit 1
}

# require_marker <logfile> <extended-regexp> <human description>
#
# Asserts that <logfile> contains <extended-regexp>. On failure it prints the
# pattern that was expected plus a tail of the log, so the first screen of CI
# output already carries the evidence instead of pointing at an artifact
# download. Uses --text/LC_ALL=C because serial logs are binary-ish.
#
# NOTE: pass an ALREADY-NORMALIZED log (see lib/markers.sh normalize_serial_log)
# when the log came off a guest serial port; raw agetty output can split a
# marker with CR/ANSI residue and produce a false negative.
require_marker() {
    local logfile="$1"
    local pattern="$2"
    local description="${3:-marker ${pattern}}"

    if [[ -f "${logfile}" ]] \
        && LC_ALL=C grep --text --quiet --extended-regexp \
            -- "${pattern}" "${logfile}"; then
        return 0
    fi

    assert_banner "${description}"
    printf '  expected pattern: %s\n' "${pattern}" >&2
    printf '  in log file:      %s\n' "${logfile}" >&2
    if [[ -f "${logfile}" ]]; then
        printf '  last %s lines of %s:\n' \
            "${ANDROMEDA_ASSERT_LOG_TAIL}" "${logfile}" >&2
        tail -n "${ANDROMEDA_ASSERT_LOG_TAIL}" "${logfile}" 2>&1 \
            | sed 's/^/    /' >&2 || true
    else
        printf '  the log file does not exist.\n' >&2
    fi
    printf '\n' >&2
    exit 1
}

# require_marker_sequence <logfile> <human description> <marker>...
#
# The strictest marker check in the pipeline: every <marker> must occur in
# <logfile> EXACTLY ONCE, and their occurrences must appear in the order given.
#
# Markers are matched as FIXED strings, never as regular expressions. That is
# deliberate rather than incidental: a caller interpolates runtime values into
# these (test-hardware-matrix.sh asserts scenario=${scenario}), and under
# --extended-regexp an unescaped interpolated value would silently change what
# is being asserted. Use require_marker when a pattern is genuinely wanted.
#
# Why exactly once AND ordered, rather than mere presence: presence cannot tell
# a guest that completed its lifecycle apart from one that crash-looped through
# the same phase twice, and counting alone cannot tell a real first-boot ->
# update -> rollback progression apart from markers arriving in a nonsensical
# order. docs/reviews/e2e-pipeline-review.md evaluation 7 records that the three
# harnesses each checked a DIFFERENT one of these properties; this is the single
# implementation they now share, so the strictness of one cannot drift from the
# strictness of another.
#
# The marker LIST stays at the call site. It describes what that specific
# harness boots, and test-ci-timeout-budget.sh derives the guest boot count by
# counting the ANDROMEDA_DAILY_DRIVER_OK entries in test-install.sh's list --
# hoisting the list in here would move that bound away from the script that
# enforces it.
#
# Note the argument order: the description precedes the markers because the
# marker list is variadic. Like require_marker, this expects an ALREADY
# NORMALIZED log (lib/markers.sh normalize_serial_log); a marker split by serial
# cursor-control residue or by a kernel printk sharing the UART would otherwise
# count as zero.
require_marker_sequence() {
    local logfile="$1"
    local description="$2"
    shift 2

    local marker
    local count
    local offset
    local raw
    local index
    local previous=-1
    local counts_ok=1
    local order_ok=1
    local -a markers=("$@")
    local -a counts=()
    local -a offsets=()

    if (( ${#markers[@]} == 0 )); then
        assert_banner "${description}"
        printf '  require_marker_sequence() was called without any markers.\n' >&2
        exit 2
    fi

    if [[ ! -f "${logfile}" ]]; then
        assert_banner "${description}"
        printf '  the log file does not exist: %s\n\n' "${logfile}" >&2
        exit 1
    fi

    for marker in "${markers[@]}"; do
        # grep -c counts LINES, not occurrences: two markers collapsed onto one
        # physical line by CR-stripping would count as 1 and a duplicate would
        # go unnoticed. -o then counting the emitted matches is what makes
        # "exactly once" mean exactly once.
        count="$(
            LC_ALL=C grep --text --only-matching --fixed-strings \
                -- "${marker}" "${logfile}" 2>/dev/null | wc -l
        )" || count=0
        # Strip the padding BSD wc emits so the value is usable in arithmetic.
        count="${count//[^0-9]/}"
        counts+=("${count:-0}")

        # First-occurrence byte offset, for the ordering check. Captured whole
        # and split here rather than piped through `head`, which would close
        # grep's stdout early and make a SIGPIPE indistinguishable from "no
        # match" -- discarding a perfectly good offset.
        raw="$(
            LC_ALL=C grep --text --byte-offset --only-matching --fixed-strings \
                --max-count=1 -- "${marker}" "${logfile}" 2>/dev/null
        )" || raw=""
        offset="${raw%%$'\n'*}"
        offset="${offset%%:*}"
        [[ "${offset}" =~ ^[0-9]+$ ]] || offset=-1
        offsets+=("${offset}")
    done

    for index in "${!markers[@]}"; do
        if (( 10#${counts[${index}]} != 1 )); then
            counts_ok=0
            continue
        fi
        if (( offsets[index] < previous )); then
            order_ok=0
        fi
        previous="${offsets[${index}]}"
    done

    if (( counts_ok == 1 && order_ok == 1 )); then
        return 0
    fi

    assert_banner "${description}"
    if (( counts_ok == 0 )); then
        printf '  every marker below must occur exactly once.\n' >&2
    else
        printf '  the markers below are present exactly once each but OUT OF ORDER.\n' >&2
    fi
    printf '  in log file: %s\n\n' "${logfile}" >&2
    printf '    %-5s %-9s %s\n' 'COUNT' 'AT BYTE' 'MARKER' >&2
    local shown_offset
    local verdict
    for index in "${!markers[@]}"; do
        shown_offset="${offsets[${index}]}"
        if (( offsets[index] < 0 )); then
            shown_offset='-'
        fi
        verdict=''
        if (( 10#${counts[${index}]} == 0 )); then
            verdict='   <== never seen'
        elif (( 10#${counts[${index}]} > 1 )); then
            verdict='   <== duplicated'
        fi
        printf '    %-5s %-9s %s%s\n' \
            "${counts[${index}]}" "${shown_offset}" \
            "${markers[${index}]}" "${verdict}" >&2
    done
    printf '\n  last %s lines of %s:\n' \
        "${ANDROMEDA_ASSERT_LOG_TAIL}" "${logfile}" >&2
    tail -n "${ANDROMEDA_ASSERT_LOG_TAIL}" "${logfile}" 2>&1 \
        | sed 's/^/    /' >&2 || true
    printf '\n' >&2
    exit 1
}
