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

# require_marker_fixed <logfile> <fixed-string> <human description>
#
# Fixed-string variant of require_marker: matches with --fixed-strings, so use
# it whenever the expected marker is a literal (especially one interpolating a
# runtime value such as scenario=${scenario}) -- with --extended-regexp an
# unescaped interpolated value could silently change what is asserted.
require_marker_fixed() {
    local logfile="$1"
    local pattern="$2"
    local description="${3:-marker ${pattern}}"

    if [[ -f "${logfile}" ]] \
        && LC_ALL=C grep --text --quiet --fixed-strings \
            -- "${pattern}" "${logfile}"; then
        return 0
    fi

    assert_banner "${description}"
    printf '  expected fixed string: %s\n' "${pattern}" >&2
    printf '  in log file:           %s\n' "${logfile}" >&2
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
