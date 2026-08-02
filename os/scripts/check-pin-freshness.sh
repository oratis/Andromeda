#!/usr/bin/env bash
# Supply-chain Tier A safety net: report on the freshness of container image
# pins before they rot.
#
# WHY THIS EXISTS
# ---------------
# `quay.io/fedora/fedora-bootc:44` used to be pinned by @sha256. Fedora
# garbage-collects superseded fedora-bootc digests from quay.io within days, so
# the pin silently became `manifest unknown` and EVERY build broke at once
# (Actions run 30702410393); commit d27cefa had to un-pin it as emergency
# first aid. Nothing in CI saw it coming.
#
# This script is that missing early-warning system. For every image reference it
# finds it answers two questions:
#
#   1. RESOLVABLE?  Is the pinned digest still fetchable from the registry?
#                   A "no" here is the fedora-bootc failure, caught before the
#                   next build instead of during it.
#   2. HOW OLD?     How long ago was the pinned image built? Age is both a
#                   rot-risk proxy (old digests are the ones registries prune)
#                   and a missing-security-updates signal.
#
# Answers come in FOUR classes, and the distinctions are load-bearing:
#
#   UNRESOLVABLE   the registry positively reports the digest gone (HTTP
#                  404/410, skopeo "manifest unknown"). The pin is broken NOW.
#   STALE          still resolvable, but older than the age budget. Advisory:
#                  rot risk plus missed upstream fixes; nothing broken today.
#   INDETERMINATE  the registry answered with something other than a clean
#                  yes/no: throttling (429), outage (5xx), auth trouble, a
#                  timeout, or no build timestamp exposed. NOT proof of
#                  deletion -- and NOT silently swallowed either, because a
#                  silently dead signal is exactly how the skopeo timestamp
#                  regression shipped green (commit 1921496).
#   OK             resolvable and within the age budget.
#
# References that are NOT pinned are reported too, with the digest they resolve
# to right now, so `docs/development/supply-chain.md`'s re-pin runbook is a
# copy-paste operation rather than a research project.
#
# EXIT CODES
#   0  pass: no findings, or findings not gated (no --fail-on = advisory mode)
#   1  gated findings include at least one UNRESOLVABLE pin
#   3  gated findings present, none of them UNRESOLVABLE (STALE and/or
#      INDETERMINATE)
#   2  usage error, no usable registry client, or ZERO references scanned --
#      a run that verified nothing must not report success
#
# See docs/development/supply-chain.md for the tiering strategy this implements.

set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT

# Default age budget. Deliberately generous: the point of the age signal is to
# catch pins drifting out of the window where registries still keep them and
# where upstream security updates have been picked up -- not to nag on every
# run. Registries differ wildly (Docker Hub keeps digests indefinitely, quay
# prunes fedora-bootc within days), so treat a warning as "look at it", not
# "the build is broken". --fail-on promotes classes of findings to failures
# for the callers that want a gate.
MAX_AGE_DAYS="${ANDROMEDA_PIN_MAX_AGE_DAYS:-90}"

# Which finding classes fail the run. Empty = advisory: findings are warnings
# and the exit code is 0. `--fail-on unresolvable` is for callers who mean
# "block me only when a pin is broken right now" (the re-pin runbook gate);
# `--fail-on any` (compatibility alias: --strict) also fails on STALE and
# INDETERMINATE. The classes are deliberately NOT conflated: the rust pin
# being 501 days old (STALE, advisory) must not be able to mask -- or be
# mistaken for -- a digest the registry no longer serves (UNRESOLVABLE).
FAIL_ON=""

# Files scanned for image references. os/Containerfile is the Tier A input this
# script exists for. os/scripts/build-iso.sh is READ ONLY here -- it carries the
# ghcr.io/osbuild/image-builder-cli pin, which is the same risk class, and
# reading it costs nothing while leaving ownership of that file untouched.
SCAN_FILES="os/Containerfile os/scripts/build-iso.sh"

usage() {
    cat <<'EOF'
Usage: check-pin-freshness.sh [--fail-on CLASS] [--max-age-days N] [FILE...]

Reports on container image pins found in FILE... (default: os/Containerfile
and os/scripts/build-iso.sh, relative to the repository root).

  --fail-on CLASS     promote finding classes to failures. CLASS is one of:
                        unresolvable  fail only when the registry positively
                                      no longer serves a pinned digest (the
                                      build-breaking class)
                        stale         additionally fail on over-age pins
                        any           additionally fail on INDETERMINATE
                                      findings (registry throttled or
                                      unreachable, no build timestamp)
                      Without --fail-on, findings are warnings and exit is 0.
  --strict            compatibility alias for --fail-on any.
  --max-age-days N    age budget in days (default 90, or
                      $ANDROMEDA_PIN_MAX_AGE_DAYS).
  -h, --help          this message.

Exit codes:
  0  pass, or findings present but not gated by --fail-on
  1  gated findings include at least one UNRESOLVABLE pin
  3  gated findings present, none unresolvable (STALE and/or INDETERMINATE)
  2  usage error, no usable registry client, or zero references scanned

Requires skopeo, or curl + jq as a fallback.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --strict)
            FAIL_ON=any
            shift
            ;;
        --fail-on)
            if [ "$#" -lt 2 ]; then
                printf 'check-pin-freshness: --fail-on needs a value\n' >&2
                exit 2
            fi
            FAIL_ON="$2"
            shift 2
            ;;
        --fail-on=*)
            FAIL_ON="${1#--fail-on=}"
            shift
            ;;
        --max-age-days)
            if [ "$#" -lt 2 ]; then
                printf 'check-pin-freshness: --max-age-days needs a value\n' >&2
                exit 2
            fi
            MAX_AGE_DAYS="$2"
            shift 2
            ;;
        --max-age-days=*)
            MAX_AGE_DAYS="${1#--max-age-days=}"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            printf 'check-pin-freshness: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
        *)
            break
            ;;
    esac
done

if [ "$#" -gt 0 ]; then
    SCAN_FILES="$*"
fi

case "${MAX_AGE_DAYS}" in
    ''|*[!0-9]*)
        printf 'check-pin-freshness: --max-age-days must be a whole number, got: %s\n' \
            "${MAX_AGE_DAYS}" >&2
        exit 2
        ;;
esac

case "${FAIL_ON}" in
    ''|unresolvable|stale|any) ;;
    *)
        printf 'check-pin-freshness: --fail-on must be unresolvable, stale or any, got: %s\n' \
            "${FAIL_ON}" >&2
        exit 2
        ;;
esac

readonly MAX_AGE_DAYS FAIL_ON SCAN_FILES

unresolvable=0
stale=0
indeterminate=0
checked=0

note() { printf '  %s\n' "$*"; }

finding_unresolvable() {
    unresolvable=$((unresolvable + 1))
    printf '  !! %s\n' "$*"
}

finding_stale() {
    stale=$((stale + 1))
    printf '  !! %s\n' "$*"
}

finding_indeterminate() {
    indeterminate=$((indeterminate + 1))
    printf '  !! %s\n' "$*"
}

# --------------------------------------------------------------------------
# Registry backend
# --------------------------------------------------------------------------
# skopeo is the preferred client: it already knows every registry's auth quirks
# and is present on the os-e2e runner. When it is missing (a plain ubuntu-latest
# CI runner, or a macOS dev box) fall back to the Docker Registry v2 HTTP API
# over curl, which needs no privileged container tooling at all. Degrading this
# way keeps the check runnable in the cheap CI job instead of forcing it into
# the 48-minute one.

BACKEND=""
if command -v skopeo >/dev/null 2>&1; then
    BACKEND="skopeo"
elif command -v curl >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
    BACKEND="curl"
else
    printf 'check-pin-freshness: needs skopeo, or curl + jq.\n' >&2
    printf '  Debian/Ubuntu: sudo apt-get install -y skopeo   (or: curl jq)\n' >&2
    printf '  macOS:         brew install skopeo              (or: brew install jq)\n' >&2
    exit 2
fi
readonly BACKEND

# Every temp file lives under one mktemp -d that the EXIT trap removes --
# including on set -e aborts, which is exactly when a stray header dump would
# otherwise be left behind.
SCRATCH_DIR="$(mktemp -d)"
readonly SCRATCH_DIR
trap 'rm -rf "${SCRATCH_DIR}"' EXIT

MANIFEST_ACCEPT='application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json'
readonly MANIFEST_ACCEPT

# Bounded, retrying curl. Transient registry hiccups (the 429/5xx class) get a
# few retries before they are reported as INDETERMINATE, and no single lookup
# may hang the whole check.
CURL_RETRY_ARGS=(--retry 3 --retry-all-errors --max-time 60 --connect-timeout 15)
readonly CURL_RETRY_ARGS

# registry_host REF -> the host to talk HTTP to.
# docker.io is a redirect shim; the real API lives on registry-1.docker.io.
registry_host() {
    local ref_host
    ref_host="${1%%/*}"
    case "${ref_host}" in
        docker.io) printf 'registry-1.docker.io' ;;
        *) printf '%s' "${ref_host}" ;;
    esac
}

# registry_path REF -> the repository path component.
registry_path() {
    printf '%s' "${1#*/}"
}

# strip_tag REF[:TAG] -> REF. A colon only introduces a tag when it appears in
# the last path segment; `localhost:5000/img` is a registry port, not a tag.
strip_tag() {
    case "${1##*/}" in
        *:*) printf '%s' "${1%:*}" ;;
        *) printf '%s' "$1" ;;
    esac
}

# registry_token HOST PATH REFERENCE -> a bearer token, by replaying the
# registry's own WWW-Authenticate challenge. Generic on purpose: quay.io,
# ghcr.io and Docker Hub all use different realms, and hardcoding them is how
# this check would quietly stop working on the next registry someone adds.
#
# An empty result is a legitimate answer, not an error: quay.io serves public
# repositories anonymously and never issues a challenge. Callers must then omit
# the Authorization header entirely -- sending an empty `Bearer ` is a malformed
# credential and gets rejected.
registry_token() {
    local token_host token_path token_ref challenge realm service
    token_host=$1
    token_path=$2
    token_ref=${3:-latest}
    challenge=$(curl -sSI "${CURL_RETRY_ARGS[@]}" \
        "https://${token_host}/v2/${token_path}/manifests/${token_ref}" 2>/dev/null \
        | tr -d '\r' \
        | awk 'tolower($1) == "www-authenticate:" { $1 = ""; print substr($0, 2); exit }') || true
    case "${challenge}" in
        Bearer*|bearer*) ;;
        *) return 0 ;;
    esac
    realm=$(printf '%s' "${challenge}" | sed -n 's/.*realm="\([^"]*\)".*/\1/p')
    service=$(printf '%s' "${challenge}" | sed -n 's/.*service="\([^"]*\)".*/\1/p')
    [ -n "${realm}" ] || return 0
    curl -sS "${CURL_RETRY_ARGS[@]}" --get \
        --data-urlencode "service=${service}" \
        --data-urlencode "scope=repository:${token_path}:pull" \
        "${realm}" 2>/dev/null \
        | jq -r '.token // .access_token // empty' 2>/dev/null || true
}

# curl_manifest HOST PATH REFERENCE [HEADERS_OUT] -> manifest body on stdout.
curl_manifest() {
    local manifest_host manifest_path manifest_ref manifest_headers manifest_token manifest_url
    manifest_host=$1
    manifest_path=$2
    manifest_ref=$3
    manifest_headers=${4:-/dev/null}
    manifest_token=$(registry_token "${manifest_host}" "${manifest_path}" "${manifest_ref}")
    manifest_url="https://${manifest_host}/v2/${manifest_path}/manifests/${manifest_ref}"
    if [ -n "${manifest_token}" ]; then
        curl -sS "${CURL_RETRY_ARGS[@]}" --fail --dump-header "${manifest_headers}" \
            -H "Authorization: Bearer ${manifest_token}" \
            -H "Accept: ${MANIFEST_ACCEPT}" \
            "${manifest_url}" 2>/dev/null
    else
        curl -sS "${CURL_RETRY_ARGS[@]}" --fail --dump-header "${manifest_headers}" \
            -H "Accept: ${MANIFEST_ACCEPT}" \
            "${manifest_url}" 2>/dev/null
    fi
}

# pin_resolvable REF DIGEST -> three-way verdict, because "the fetch failed" is
# NOT the same claim as "the registry deleted the digest". On a Docker Hub 429
# the old binary yes/no printed "the registry no longer serves this digest",
# which was simply false.
#
#   return 0                  digest is served (2xx)
#   return 1, detail stdout   registry positively reports it gone (404/410,
#                             skopeo "manifest unknown")
#   return 2, detail stdout   anything else: throttling, outage, auth trouble,
#                             timeout -> caller reports INDETERMINATE
pin_resolvable() {
    local skopeo_err err_file host path token url code
    if [ "${BACKEND}" = "skopeo" ]; then
        err_file=$(mktemp "${SCRATCH_DIR}/skopeo-err.XXXXXX")
        if skopeo inspect --raw "docker://$1@$2" >/dev/null 2>"${err_file}"; then
            rm -f "${err_file}"
            return 0
        fi
        skopeo_err=$(head -n 1 "${err_file}")
        rm -f "${err_file}"
        if printf '%s' "${skopeo_err}" | grep -qiE 'manifest[ _]unknown|name[ _]unknown|not found|404'; then
            printf 'skopeo: %s' "${skopeo_err}"
            return 1
        fi
        printf 'skopeo failed without a not-found error: %s' "${skopeo_err}"
        return 2
    fi

    host=$(registry_host "$1")
    path=$(registry_path "$1")
    token=$(registry_token "${host}" "${path}" "$2")
    url="https://${host}/v2/${path}/manifests/$2"
    if [ -n "${token}" ]; then
        code=$(curl -sS -o /dev/null -w '%{http_code}' "${CURL_RETRY_ARGS[@]}" \
            -H "Authorization: Bearer ${token}" \
            -H "Accept: ${MANIFEST_ACCEPT}" \
            "${url}" 2>/dev/null) || true
    else
        code=$(curl -sS -o /dev/null -w '%{http_code}' "${CURL_RETRY_ARGS[@]}" \
            -H "Accept: ${MANIFEST_ACCEPT}" \
            "${url}" 2>/dev/null) || true
    fi
    case "${code}" in
        2??)
            return 0
            ;;
        404|410)
            printf 'registry answered HTTP %s for the manifest' "${code}"
            return 1
            ;;
        ''|000)
            printf 'no HTTP response (DNS failure, connect failure, or timeout)'
            return 2
            ;;
        *)
            printf 'registry answered HTTP %s -- throttling/outage/auth, not proof of deletion' "${code}"
            return 2
            ;;
    esac
}

# pin_created REF DIGEST -> the image's build timestamp (RFC 3339), or nothing.
# Reaching it means walking index -> linux/amd64 manifest -> config blob, which
# skopeo does for us via --override-os/--override-arch. An empty result is the
# caller's cue to report INDETERMINATE -- never to stay silent.
pin_created() {
    local stamp created_host created_path created_manifest child config_digest config_token config_url
    if [ "${BACKEND}" = "skopeo" ]; then
        stamp=$(skopeo inspect --override-os linux --override-arch amd64 \
            --format '{{.Created}}' "docker://$1@$2" 2>/dev/null || true)
        # Go renders a zero time.Time as 0001-01-01...; that means "no
        # timestamp recorded", not a two-thousand-year-old image.
        case "${stamp}" in
            0001-01-01*) return 0 ;;
        esac
        printf '%s' "${stamp}"
        return 0
    fi

    created_host=$(registry_host "$1")
    created_path=$(registry_path "$1")
    created_manifest=$(curl_manifest "${created_host}" "${created_path}" "$2") || return 0

    child=$(printf '%s' "${created_manifest}" \
        | jq -r '[.manifests[]? | select(.platform.os == "linux" and .platform.architecture == "amd64")][0].digest // empty' 2>/dev/null || true)
    if [ -n "${child}" ]; then
        created_manifest=$(curl_manifest "${created_host}" "${created_path}" "${child}") || return 0
    fi

    config_digest=$(printf '%s' "${created_manifest}" \
        | jq -r '.config.digest // empty' 2>/dev/null || true)
    [ -n "${config_digest}" ] || return 0

    config_token=$(registry_token "${created_host}" "${created_path}" "$2")
    config_url="https://${created_host}/v2/${created_path}/blobs/${config_digest}"
    if [ -n "${config_token}" ]; then
        curl -sSL "${CURL_RETRY_ARGS[@]}" --fail \
            -H "Authorization: Bearer ${config_token}" \
            "${config_url}" 2>/dev/null \
            | jq -r '.created // empty' 2>/dev/null || true
    else
        curl -sSL "${CURL_RETRY_ARGS[@]}" --fail "${config_url}" 2>/dev/null \
            | jq -r '.created // empty' 2>/dev/null || true
    fi
}

# tag_digest REF TAG -> the digest that tag points at today. Feeds the re-pin
# runbook: the value printed here is the value to paste into the Containerfile.
# The manifest-list digest is what we want (it keeps the reference multi-arch).
#
# On the skopeo path, prefer skopeo's own digest computation (`--format
# '{{.Digest}}'`). The old fallback hashed `$(skopeo inspect --raw ...)`
# client-side -- but command substitution strips trailing newlines, so a
# manifest ending in one would hash to the WRONG digest. The fallback now
# writes the raw bytes to a file and hashes the file, which preserves every
# byte.
tag_digest() {
    local skopeo_digest raw_file tag_headers
    if [ "${BACKEND}" = "skopeo" ]; then
        skopeo_digest=$(skopeo inspect --format '{{.Digest}}' "docker://$1:$2" 2>/dev/null || true)
        case "${skopeo_digest}" in
            sha256:*)
                printf '%s' "${skopeo_digest}"
                return 0
                ;;
        esac
        raw_file=$(mktemp "${SCRATCH_DIR}/manifest.XXXXXX")
        if skopeo inspect --raw "docker://$1:$2" >"${raw_file}" 2>/dev/null \
            && [ -s "${raw_file}" ]; then
            if command -v sha256sum >/dev/null 2>&1; then
                printf 'sha256:%s' "$(sha256sum <"${raw_file}" | cut -d' ' -f1)"
            elif command -v shasum >/dev/null 2>&1; then
                printf 'sha256:%s' "$(shasum -a 256 <"${raw_file}" | cut -d' ' -f1)"
            fi
        fi
        rm -f "${raw_file}"
        return 0
    fi

    tag_headers=$(mktemp "${SCRATCH_DIR}/headers.XXXXXX")
    if curl_manifest "$(registry_host "$1")" "$(registry_path "$1")" "$2" "${tag_headers}" >/dev/null; then
        tr -d '\r' <"${tag_headers}" \
            | awk 'tolower($1) == "docker-content-digest:" { print $2; exit }'
    fi
    rm -f "${tag_headers}"
}

# iso_to_epoch TIMESTAMP -> seconds since epoch.
#
# Two normalisations are needed before any date(1) will touch it:
#   * the registry API returns RFC 3339:  2025-03-18T20:40:17Z
#   * skopeo's Go template renders a time.Time with Go's default layout:
#     2025-03-18 20:40:17 +0000 UTC
# The second form silently defeated parsing on CI runners (which DO ship
# skopeo), so every age check there degraded to "unparseable" and the stale-pin
# finding was lost. Collapse both to YYYY-MM-DDTHH:MM:SSZ; the age budget is
# measured in days, so dropping sub-second precision costs nothing.
#
# Then: GNU date, BSD date and python3 all disagree on parsing flags, and CI
# (Linux) and the maintainer's box (darwin) are not the same platform, so try
# each rather than assuming.
iso_to_epoch() {
    local stamp
    stamp=$(printf '%s' "$1" \
        | sed -e 's/ /T/' -e 's/^\(....-..-..T..:..:..\).*/\1Z/')
    date -u -d "${stamp}" +%s 2>/dev/null && return 0
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "${stamp}" +%s 2>/dev/null && return 0
    if command -v python3 >/dev/null 2>&1; then
        python3 -c 'import calendar,sys,time; print(calendar.timegm(time.strptime(sys.argv[1], "%Y-%m-%dT%H:%M:%SZ")))' \
            "${stamp}" 2>/dev/null && return 0
    fi
    return 1
}

age_days() {
    local epoch now
    epoch=$(iso_to_epoch "$1") || return 1
    now=$(date -u +%s)
    printf '%s' "$(( (now - epoch) / 86400 ))"
}

# --------------------------------------------------------------------------
# Reference reporting
# --------------------------------------------------------------------------

report_pinned() {
    local ref digest origin resolve_detail resolve_status created days
    ref=$1
    digest=$2
    origin=$3
    checked=$((checked + 1))
    printf '\n%s\n' "${ref}@${digest}"
    note "declared in ${origin}, pinned by digest"

    resolve_status=0
    resolve_detail=$(pin_resolvable "${ref}" "${digest}") || resolve_status=$?
    if [ "${resolve_status}" -eq 1 ]; then
        finding_unresolvable "UNRESOLVABLE: the registry no longer serves this digest (${resolve_detail})."
        note "This is the fedora-bootc failure mode (run 30702410393): every"
        note "build using this reference is broken until the pin is refreshed."
        note "Fix: resolve the tag to its current digest and update the pin --"
        note "see the re-pin runbook in docs/development/supply-chain.md."
        return
    fi
    if [ "${resolve_status}" -ne 0 ]; then
        finding_indeterminate "INDETERMINATE: could not verify the pin: ${resolve_detail}."
        note "This is NOT the fedora-bootc failure mode: the registry did not"
        note "positively report the digest gone, it just failed to answer"
        note "cleanly. Re-run before treating this pin as broken."
        return
    fi
    note "resolvable: yes"

    created=$(pin_created "${ref}" "${digest}")
    if [ -z "${created}" ]; then
        finding_indeterminate "INDETERMINATE: age unknown (registry did not expose a build timestamp)."
        note "The age signal is dead for this reference, so staleness cannot be"
        note "assessed. A silently dead age signal is how the skopeo timestamp"
        note "regression shipped unnoticed (commit 1921496) -- hence a finding."
        return
    fi
    if ! days=$(age_days "${created}"); then
        finding_indeterminate "INDETERMINATE: unparseable build timestamp: ${created}."
        return
    fi
    if [ "${days}" -gt "${MAX_AGE_DAYS}" ]; then
        finding_stale "STALE: built ${created} (${days} days ago, budget ${MAX_AGE_DAYS})."
        note "Still resolvable, so nothing is broken today -- but an old pin is"
        note "both the kind registries prune and ${days} days of missed upstream"
        note "fixes. Renovate opens the refresh PR once it is installed."
    else
        note "age: ${days} days (built ${created}), within the ${MAX_AGE_DAYS} day budget"
    fi
}

report_unpinned() {
    local ref tag origin digest created days
    ref=$1
    tag=$2
    origin=$3
    checked=$((checked + 1))
    printf '\n%s:%s\n' "${ref}" "${tag}"
    note "declared in ${origin}, TAG-TRACKED (no digest pin)"
    note "Not a failure: a rolling tag cannot go unresolvable. It trades"
    note "reproducibility for availability -- the same commit builds different"
    note "content tomorrow. Tier A wants this pinned once digest-refresh"
    note "automation is live; see docs/development/supply-chain.md."

    digest=$(tag_digest "${ref}" "${tag}")
    if [ -z "${digest}" ]; then
        finding_indeterminate "INDETERMINATE: could not resolve the tag's current digest (registry unreachable or throttled)."
        note "The re-pin runbook needs this value; re-run when the registry"
        note "answers."
        return
    fi
    note "current digest: ${digest}"

    created=$(pin_created "${ref}" "${digest}")
    if [ -n "${created}" ]; then
        if days=$(age_days "${created}"); then
            note "tag content built ${created} (${days} days ago)"
        fi
    else
        note "tag content age: unavailable (no build timestamp exposed)"
    fi
    note "to re-pin, the reference becomes:"
    note "  ${ref}:${tag}@${digest}"
}

# --------------------------------------------------------------------------
# Extraction
# --------------------------------------------------------------------------
# Containerfile FROM lines are parsed structurally; every other file is swept
# for digest-bearing references, which is enough to cover pins that live in
# shell variables (build-iso.sh) without this script needing to know their shape.

DIGEST_RE='[0-9a-f]\{64\}'
# A reference must START with an alphanumeric. Without that anchor the leading
# `-` of a shell default like ${IMAGE_BUILDER_IMAGE:-ghcr.io/...} gets swallowed
# into the registry host and every lookup fails with a bogus "unresolvable".
HOST_RE='[A-Za-z0-9][A-Za-z0-9._-]\{0,\}\(\.[A-Za-z0-9._-]\{1,\}\)\{1,\}\(:[0-9]\{1,\}\)\{0,1\}'
PATH_RE='/[A-Za-z0-9._/-]\{1,\}\(:[A-Za-z0-9._-]\{1,\}\)\{0,1\}'

scan_containerfile() {
    local file origin image ref_and_tag base
    file=$1
    origin=$2
    while IFS= read -r image; do
        [ -n "${image}" ] || continue
        # A bare word with no registry host and no digest is a build-stage
        # alias (`FROM payload AS installer`), not something to resolve.
        case "${image}" in
            *@sha256:*) ;;
            */*) ;;
            *)
                continue
                ;;
        esac
        case "${image}" in
            *@sha256:*)
                ref_and_tag="${image%%@*}"
                report_pinned "$(strip_tag "${ref_and_tag}")" "${image#*@}" "${origin}"
                ;;
            *)
                base=$(strip_tag "${image}")
                if [ "${base}" = "${image}" ]; then
                    report_unpinned "${image}" latest "${origin}"
                else
                    report_unpinned "${base}" "${image##*:}" "${origin}"
                fi
                ;;
        esac
    done <<EOF
$(awk 'toupper($1) == "FROM" {
           for (i = 2; i <= NF; i++) {
               if (substr($i, 1, 2) != "--") { print $i; break }
           }
       }' "${file}")
EOF
}

scan_for_digests() {
    local file origin image ref_and_tag
    file=$1
    origin=$2
    while IFS= read -r image; do
        [ -n "${image}" ] || continue
        ref_and_tag="${image%%@*}"
        report_pinned "$(strip_tag "${ref_and_tag}")" "${image#*@}" "${origin}"
    done <<EOF
$(grep -o "${HOST_RE}${PATH_RE}@sha256:${DIGEST_RE}" "${file}" | sort -u)
EOF
}

if [ -n "${FAIL_ON}" ]; then
    mode_label="gate (--fail-on ${FAIL_ON})"
else
    mode_label='advisory (findings warn)'
fi
readonly mode_label

printf 'Container image pin freshness\n'
printf '  backend:      %s\n' "${BACKEND}"
printf '  age budget:   %s days\n' "${MAX_AGE_DAYS}"
printf '  mode:         %s\n' "${mode_label}"

# SCAN_FILES is a deliberately space-separated list; word splitting is the point.
# shellcheck disable=SC2086
for relative in ${SCAN_FILES}; do
    path="${REPOSITORY_ROOT}/${relative}"
    if [ ! -f "${path}" ]; then
        path="${relative}"
    fi
    if [ ! -f "${path}" ]; then
        printf '\n%s: not found, skipping\n' "${relative}"
        continue
    fi
    case "${relative}" in
        *Containerfile|*Dockerfile)
            scan_containerfile "${path}" "${relative}"
            ;;
        *)
            scan_for_digests "${path}" "${relative}"
            ;;
    esac
done

total=$((unresolvable + stale + indeterminate))

printf '\n----\n'
printf '%s reference(s) checked, %s finding(s): %s unresolvable, %s stale, %s indeterminate\n' \
    "${checked}" "${total}" "${unresolvable}" "${stale}" "${indeterminate}"

# Machine-readable summary: one line, stable format, greppable from CI logs
# without parsing the prose above it.
printf 'ANDROMEDA_PIN_FRESHNESS unresolvable=%s stale=%s indeterminate=%s checked=%s\n' \
    "${unresolvable}" "${stale}" "${indeterminate}" "${checked}"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
        printf '### Container image pin freshness\n\n'
        printf -- '- references checked: %s\n' "${checked}"
        printf -- '- UNRESOLVABLE (build-breaking): %s\n' "${unresolvable}"
        printf -- '- STALE (over the %s-day age budget): %s\n' "${MAX_AGE_DAYS}" "${stale}"
        printf -- '- INDETERMINATE (registry did not answer cleanly): %s\n' "${indeterminate}"
        printf -- '- mode: %s, backend: %s\n\n' "${mode_label}" "${BACKEND}"
    } >>"${GITHUB_STEP_SUMMARY}"
fi

# A run that checked nothing must not report success: either the scan list is
# wrong or the extraction regressed, and both mean the safety net is not
# actually deployed. This is an infrastructure failure (2), not a finding.
if [ "${checked}" -eq 0 ]; then
    printf 'check-pin-freshness: zero image references found in scan list: %s\n' \
        "${SCAN_FILES}" >&2
    printf 'A check that verified nothing cannot vouch for anything. Fix the\n' >&2
    printf 'scan list (or the extraction patterns) before trusting this check.\n' >&2
    exit 2
fi

# Gating. `--fail-on stale` gates the superset {unresolvable, stale}: a
# staleness gate that waves through a pin the registry already deleted would
# be nonsense. Exit codes stay distinct so callers can tell "broken now" (1)
# from "advisory findings promoted" (3) without parsing output.
fail_exit=0
case "${FAIL_ON}" in
    unresolvable)
        if [ "${unresolvable}" -gt 0 ]; then
            fail_exit=1
        fi
        ;;
    stale)
        if [ "${unresolvable}" -gt 0 ]; then
            fail_exit=1
        elif [ "${stale}" -gt 0 ]; then
            fail_exit=3
        fi
        ;;
    any)
        if [ "${unresolvable}" -gt 0 ]; then
            fail_exit=1
        elif [ "${total}" -gt 0 ]; then
            fail_exit=3
        fi
        ;;
esac

if [ "${fail_exit}" -ne 0 ]; then
    printf 'ANDROMEDA_PIN_FRESHNESS_FAILED (--fail-on %s)\n' "${FAIL_ON}" >&2
    exit "${fail_exit}"
fi

if [ "${total}" -eq 0 ]; then
    printf 'ANDROMEDA_PIN_FRESHNESS_OK\n'
else
    printf 'ANDROMEDA_PIN_FRESHNESS_WARN (findings present but not gated by --fail-on)\n'
fi
exit 0
