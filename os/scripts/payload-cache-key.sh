#!/usr/bin/env bash
# Content-addressed cache identity for the Andromeda payload image.
#
# Why this exists: 66-68% of the os-e2e wall clock is the image build, and the
# measured breakdown (docs/reviews/e2e-pipeline-review.md, run 30694919736) puts
# 11.1 min of that in the 23 payload dnf layers plus 2.9 min in the installer
# stage. `Using cache` appears 73 times in a build log but every one of those
# hits is INTRA-run (v1 -> v2, v1 -> installer): the runner is fresh, so the
# cross-run hit rate is exactly zero and those ~14 min are re-spent on every
# push. This script names the cache so the layers can survive across runs.
#
# It emits three things:
#
#   content hash  sha256 over every tracked file the payload/installer stages
#                 actually consume (see PAYLOAD_INPUT_PATHS). Purely
#                 informational for the layer cache -- podman derives its own
#                 per-layer keys -- but it is the identity printed in the job
#                 summary, so "should this run have hit cache?" is answerable
#                 from the log alone, and it is the key a future exact-match
#                 image cache would use.
#   week stamp    ISO year + ISO week, e.g. 2026w31. THE TIME BOUND. See below.
#   cache repo    ${base}/${week} -- what podman --cache-from/--cache-to gets.
#
# WHY THE WEEK STAMP IS MANDATORY, AND WHY IT IS IN THE *REPOSITORY*:
#
# The payload installs ~300 dnf packages from a rolling Fedora repo. A cache
# that never expires would pin the image to whatever package set happened to be
# current the first time it was populated, so security updates would silently
# stop reaching the built image -- the cache would quietly convert a
# security-relevant input into a frozen one. Rotating the repository every ISO
# week forces a completely cold rebuild at least every 7 days, which reinstates
# a full `dnf install` against current Fedora. build-iso.sh additionally passes
# `--cache-ttl` as a second, finer-grained bound; the week stamp is the belt to
# that suspenders, because it is a key change and therefore cannot be defeated
# by a cache entry whose timestamp gets refreshed on every run.
#
# The stamp lives in the repository rather than the tag because
# `podman build --cache-from/--cache-to` takes a REPOSITORY: buildah trims any
# tag you hand it and applies its own per-layer cache-key tags. A tag is
# therefore documentary only; only the repository is an enforced key.
#
# WHY THE CONTENT HASH IS *NOT* THE LAYER-CACHE KEY:
#
# Keying the layer cache on the full content hash would guarantee a miss on
# every PR that touches crates/** or os/files/** -- which is nearly every PR
# that triggers this workflow -- reducing the cache to a no-op. The expensive
# layers (the 23 dnf transactions) sit BEFORE the `COPY --from=rust-builder` and
# `COPY os/files/` instructions in os/Containerfile, so podman's own per-layer
# keys already reuse them across a code-only change. A coarse, week-scoped
# repository is what preserves those partial hits; the content hash is the
# human-readable identity layered on top.
#
# Usage:
#   payload-cache-key.sh                 # NAME=value lines, one per field
#   payload-cache-key.sh --print a,b     # just those values, space separated
#   payload-cache-key.sh --manifest      # the per-file digest list being hashed
#
# Fields: content-hash short week tag base-repo repo ref
#
# Environment:
#   ANDROMEDA_PAYLOAD_CACHE_REPO  base registry repo; empty disables repo/ref
#   ANDROMEDA_PAYLOAD_CACHE_WEEK  override the week stamp (tests/determinism)
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT

# Everything the payload and installer stages of os/Containerfile consume.
# Derived by reading the Containerfile, not by guessing:
#   rust-builder  COPY Cargo.toml Cargo.lock rust-toolchain.toml ./ + COPY crates
#   payload       the Containerfile text itself (base image ref + 23 dnf lists)
#                 and COPY os/files/ /
#   installer     COPY os/installer/{iso.yaml,*.ks,*.sh}
# os/scripts/build-iso.sh is included because it supplies the --build-arg values
# (PLATFORM_VARIANT, BOOT_PROVIDER, HEP_ID, INSTALLER_DEFAULT) that end up baked
# into image labels and /usr/lib/andromeda/platform.json, so it is as much a
# build input as the Containerfile is.
readonly PAYLOAD_INPUT_PATHS=(
    Cargo.lock
    Cargo.toml
    rust-toolchain.toml
    crates
    os/Containerfile
    os/files
    os/installer
    os/scripts/build-iso.sh
)

readonly PAYLOAD_CACHE_BASE_REPO="${ANDROMEDA_PAYLOAD_CACHE_REPO:-}"

# sha256 of stdin, hex only. sha256sum on the Fedora/Ubuntu paths, shasum on the
# darwin development host, so the determinism check can be run where the
# maintainer actually works.
sha256_stdin() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum | cut -d' ' -f1
    elif command -v shasum > /dev/null 2>&1; then
        shasum -a 256 | cut -d' ' -f1
    else
        printf 'payload-cache-key.sh needs sha256sum or shasum on PATH.\n' >&2
        exit 1
    fi
}

# List the input files, repo-root relative, NUL separated, in a byte order that
# does not depend on the locale or on the filesystem's directory order.
#
# `git ls-files` is preferred because it excludes build output (crates/*/target
# is gitignored and must never enter the hash) and matches "the tracked inputs"
# exactly. The find fallback exists for the GCP path, which untars the repository
# into ~/andromeda with no .git directory.
list_input_files() {
    if git -C "${REPOSITORY_ROOT}" rev-parse --is-inside-work-tree \
        > /dev/null 2>&1; then
        git -C "${REPOSITORY_ROOT}" ls-files -z -- "${PAYLOAD_INPUT_PATHS[@]}"
    else
        (
            cd "${REPOSITORY_ROOT}"
            find "${PAYLOAD_INPUT_PATHS[@]}" \
                \( -name target -o -name .git \) -prune -o \
                -type f -print0
        )
    fi | LC_ALL=C sort -z
}

# One line per input file: `<mode> <sha256> <path>`.
#
# The mode is recorded because the executable bit is load-bearing for
# os/files/usr/libexec/* -- a chmod with no content change still produces a
# different image and must therefore produce a different hash.
payload_manifest() {
    local path mode digest
    while IFS= read -r -d '' path; do
        if [[ -x "${REPOSITORY_ROOT}/${path}" ]]; then
            mode=755
        else
            mode=644
        fi
        digest="$(sha256_stdin < "${REPOSITORY_ROOT}/${path}")"
        printf '%s %s %s\n' "${mode}" "${digest}" "${path}"
    done < <(list_input_files)
}

# ISO year + ISO week, e.g. 2026w31. %G/%V rather than %Y/%U so the stamp does
# not jump backwards across a new year boundary.
cache_week() {
    if [[ -n "${ANDROMEDA_PAYLOAD_CACHE_WEEK:-}" ]]; then
        printf '%s' "${ANDROMEDA_PAYLOAD_CACHE_WEEK}"
        return 0
    fi
    date -u +%Gw%V
}

manifest="$(payload_manifest)"
if [[ -z "${manifest}" ]]; then
    printf 'payload-cache-key.sh found no payload inputs under %s.\n' \
        "${REPOSITORY_ROOT}" >&2
    exit 1
fi

content_hash="$(printf '%s\n' "${manifest}" | sha256_stdin)"
readonly content_hash
content_short="${content_hash:0:16}"
readonly content_short
week="$(cache_week)"
readonly week
cache_tag="${week}-${content_short}"
readonly cache_tag

# The week-scoped repository is the enforced key handed to podman; cache_ref is
# the same thing with the content hash attached for humans and logs.
if [[ -n "${PAYLOAD_CACHE_BASE_REPO}" ]]; then
    cache_repo="${PAYLOAD_CACHE_BASE_REPO}/${week}"
    cache_ref="${cache_repo}:${content_short}"
else
    cache_repo=""
    cache_ref=""
fi
readonly cache_repo cache_ref

field_value() {
    case "$1" in
        content-hash) printf '%s' "${content_hash}" ;;
        short) printf '%s' "${content_short}" ;;
        week) printf '%s' "${week}" ;;
        tag) printf '%s' "${cache_tag}" ;;
        base-repo) printf '%s' "${PAYLOAD_CACHE_BASE_REPO}" ;;
        repo) printf '%s' "${cache_repo}" ;;
        ref) printf '%s' "${cache_ref}" ;;
        *)
            printf 'Unknown field: %s (want one of: content-hash short week tag base-repo repo ref)\n' \
                "$1" >&2
            exit 2
            ;;
    esac
}

case "${1:-}" in
    --manifest)
        printf '%s\n' "${manifest}"
        ;;
    --print)
        if [[ -z "${2:-}" ]]; then
            printf '%s\n' '--print needs a comma-separated field list.' >&2
            exit 2
        fi
        separator=""
        IFS=',' read -r -a requested_fields <<< "$2"
        for field in "${requested_fields[@]}"; do
            printf '%s' "${separator}"
            field_value "${field}"
            separator=" "
        done
        printf '\n'
        ;;
    "")
        printf 'ANDROMEDA_PAYLOAD_CACHE_CONTENT_HASH=%s\n' "${content_hash}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_SHORT=%s\n' "${content_short}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_WEEK=%s\n' "${week}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_TAG=%s\n' "${cache_tag}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_BASE_REPO=%s\n' "${PAYLOAD_CACHE_BASE_REPO}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_RESOLVED_REPO=%s\n' "${cache_repo}"
        printf 'ANDROMEDA_PAYLOAD_CACHE_RESOLVED_REF=%s\n' "${cache_ref}"
        ;;
    *)
        printf 'Usage: payload-cache-key.sh [--manifest | --print <fields>]\n' >&2
        exit 2
        ;;
esac
