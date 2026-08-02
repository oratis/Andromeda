#!/usr/bin/env bash
# Shared serial-log normalization for the os/scripts E2E harnesses.
#
# Why this exists: guest markers are emitted to a QEMU serial port that also
# carries agetty/plasma cursor-control traffic. A marker can therefore arrive
# interleaved with NUL padding, bare CRs, and ANSI CSI escapes, splitting an
# otherwise exact string mid-token. Grepping the RAW serial log then reports "no
# match" for a marker that is physically present -- a false negative that costs a
# full 600s/2700s timeout before anyone finds out.
#
# This has already bitten this repository once (docs/development/
# daily-driver-e2e.md, historical false negative) and the fix was only applied to
# test-install.sh; test-hardware-matrix.sh kept grepping raw serial output while
# asserting the single longest exact-match string in the whole pipeline. See
# docs/reviews/e2e-pipeline-review.md P0 #3. One copy, used by both harnesses.
#
# Source it from a sibling script with:
#     ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#     # shellcheck source=os/scripts/lib/markers.sh
#     # shellcheck disable=SC1091
#     . "${ANDROMEDA_SCRIPT_DIR}/lib/markers.sh"

# Idempotent: sourcing twice must not clobber anything.
if [[ -n "${ANDROMEDA_MARKERS_SH_LOADED:-}" ]]; then
    return 0
fi
readonly ANDROMEDA_MARKERS_SH_LOADED=1

# A literal ESC. Built with printf rather than written as \x1b inside the sed
# program so the expression stays portable across GNU and BSD sed.
ANDROMEDA_ESC="$(printf '\033')"
readonly ANDROMEDA_ESC

# normalize_serial_stream
#
# Filter: reads a raw serial log on stdin, writes a grep-safe copy to stdout.
# Removes NUL and CR (agetty line discipline residue) and ANSI CSI escape
# sequences (ESC [ <params> <intermediates> <final>). LC_ALL=C keeps tr/sed byte
# oriented so invalid UTF-8 in the serial stream cannot abort the pipeline.
normalize_serial_stream() {
    LC_ALL=C tr -d '\000\r' \
        | LC_ALL=C sed -E "s#${ANDROMEDA_ESC}\\[[0-?]*[ -/]*[@-~]##g" \
        | repair_spliced_kernel_lines
}

# repair_spliced_kernel_lines
#
# The guest and the kernel share one UART, so a printk can land in the MIDDLE of
# a userspace write and cut a marker in half:
#
#     ANDROMEDA_SELINU[    8.687420] snd_hda_codec_generic hdaudioC0D0: Line=0x5
#     X_LABELS_OK
#
# Stripping control characters cannot repair this -- the marker is split by
# another writer's text, not by escape codes -- so a lifecycle that actually
# completed still fails the "each marker exactly once" check. Observed on Actions
# run 30740353815: the guest emitted all ten markers including ANDROMEDA_E2E_OK,
# and the run failed because ANDROMEDA_SELINUX_LABELS_OK matched zero times.
#
# Rejoin only the splice: a kernel timestamp that begins mid-line, plus the rest
# of that physical line and its newline. A kernel message at the START of a line
# is ordinary console output and is kept -- this does not strip kernel logging in
# general, only the fragment that interrupted somebody else's line. On the run
# above, all 42 removed fragments were embedded in non-kernel lines. The raw
# serial log is retained next to the normalized copy, so no evidence is lost.
repair_spliced_kernel_lines() {
    LC_ALL=C perl -0777 -pe 's/(?<!\n)\[[ ]*\d+\.\d+\][^\n]*\n//g'
}

# normalize_serial_log <input> [output]
#
# Writes a normalized copy of <input> to <output>, or to stdout when <output> is
# omitted. A missing/unreadable input yields empty output and a zero status:
# callers poll these logs while QEMU is still creating them, and a transient
# read error must not be mistaken for a test failure.
normalize_serial_log() {
    local input="$1"
    local output="${2:-}"

    if [[ -n "${output}" ]]; then
        # Test readability BEFORE redirecting: `<"${input}" ... 2>/dev/null`
        # opens the input redirection before stderr is redirected, so a
        # missing log would leak "No such file or directory" to real stderr.
        if [[ -r "${input}" ]]; then
            normalize_serial_stream <"${input}" >"${output}" \
                || : >"${output}"
        else
            : >"${output}"
        fi
        return 0
    fi
    normalize_serial_stream <"${input}" 2>/dev/null || true
}

# dump_log_region <logfile> <extended-regexp> [before] [after]
#
# Prints the region of <logfile> around the FIRST line matching <pattern> to
# stderr, with line numbers. Used by the installer failure gate so the CI log's
# first screen already shows the failure context instead of a pointer to a 2 MiB
# artifact download.
dump_log_region() {
    local logfile="$1"
    local pattern="$2"
    local before="${3:-40}"
    local after="${4:-40}"
    local hit

    [[ -f "${logfile}" ]] || return 0
    hit="$(
        LC_ALL=C grep --text --line-number --max-count=1 --extended-regexp \
            -- "${pattern}" "${logfile}" 2>/dev/null | cut -d: -f1
    )" || hit=""
    [[ -n "${hit}" ]] || return 0

    # The leading two spaces are not cosmetic: a printf format string starting
    # with '-' is parsed as options by the bash builtin and fails at runtime --
    # on the failure path, where it is least affordable.
    printf '  --- %s lines %s-%s (around /%s/) ---\n' \
        "${logfile}" \
        "$(( hit > before ? hit - before : 1 ))" \
        "$(( hit + after ))" \
        "${pattern}" >&2
    LC_ALL=C awk -v start="$(( hit > before ? hit - before : 1 ))" \
        -v end="$(( hit + after ))" \
        'NR >= start && NR <= end { printf "%6d  %s\n", NR, $0 }
         NR > end { exit }' \
        "${logfile}" >&2 || true
    printf '  --- end of region ---\n' >&2
}
