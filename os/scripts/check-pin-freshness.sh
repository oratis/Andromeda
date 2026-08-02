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
# References that are NOT pinned are reported too, with the digest they resolve
# to right now, so `docs/development/supply-chain.md`'s re-pin runbook is a
# copy-paste operation rather than a research project.
#
# EXIT CODES
#   0  no findings, or findings downgraded to warnings (the default)
#   1  --strict and at least one finding
#   2  usage error, or no usable registry client
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
# "the build is broken". --strict promotes findings to failures for the one
# caller that wants a gate.
MAX_AGE_DAYS="${ANDROMEDA_PIN_MAX_AGE_DAYS:-90}"
STRICT=0

# Files scanned for image references. os/Containerfile is the Tier A input this
# script exists for. os/scripts/build-iso.sh is READ ONLY here -- it carries the
# ghcr.io/osbuild/image-builder-cli pin, which is the same risk class, and
# reading it costs nothing while leaving ownership of that file untouched.
SCAN_FILES="os/Containerfile os/scripts/build-iso.sh"

usage() {
    cat <<'EOF'
Usage: check-pin-freshness.sh [--strict] [--max-age-days N] [FILE...]

Reports on container image pins found in FILE... (default: os/Containerfile
and os/scripts/build-iso.sh, relative to the repository root).

  --strict            exit non-zero if any pin is unresolvable or over-age.
                      Without it, findings are warnings and the exit code is 0.
  --max-age-days N    age budget in days (default 90, or
                      $ANDROMEDA_PIN_MAX_AGE_DAYS).
  -h, --help          this message.

Requires skopeo, or curl + jq as a fallback.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --strict)
            STRICT=1
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

readonly MAX_AGE_DAYS STRICT SCAN_FILES

findings=0
checked=0

note() { printf '  %s\n' "$*"; }

finding() {
    findings=$((findings + 1))
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

MANIFEST_ACCEPT='application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json'
readonly MANIFEST_ACCEPT

# registry_host REF -> the host to talk HTTP to.
# docker.io is a redirect shim; the real API lives on registry-1.docker.io.
registry_host() {
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
    token_host=$1
    token_path=$2
    token_ref=${3:-latest}
    challenge=$(curl -sSI --max-time 30 \
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
    curl -sS --max-time 30 --get \
        --data-urlencode "service=${service}" \
        --data-urlencode "scope=repository:${token_path}:pull" \
        "${realm}" 2>/dev/null \
        | jq -r '.token // .access_token // empty' 2>/dev/null || true
}

# curl_manifest HOST PATH REFERENCE [HEADERS_OUT] -> manifest body on stdout.
curl_manifest() {
    manifest_host=$1
    manifest_path=$2
    manifest_ref=$3
    manifest_headers=${4:-/dev/null}
    manifest_token=$(registry_token "${manifest_host}" "${manifest_path}" "${manifest_ref}")
    manifest_url="https://${manifest_host}/v2/${manifest_path}/manifests/${manifest_ref}"
    if [ -n "${manifest_token}" ]; then
        curl -sS --max-time 60 --fail --dump-header "${manifest_headers}" \
            -H "Authorization: Bearer ${manifest_token}" \
            -H "Accept: ${MANIFEST_ACCEPT}" \
            "${manifest_url}" 2>/dev/null
    else
        curl -sS --max-time 60 --fail --dump-header "${manifest_headers}" \
            -H "Accept: ${MANIFEST_ACCEPT}" \
            "${manifest_url}" 2>/dev/null
    fi
}

# pin_resolvable REF DIGEST -> 0 when the registry still serves that digest.
# This is the check that would have caught the fedora-bootc breakage.
pin_resolvable() {
    if [ "${BACKEND}" = "skopeo" ]; then
        skopeo inspect --raw "docker://$1@$2" >/dev/null 2>&1
        return
    fi
    curl_manifest "$(registry_host "$1")" "$(registry_path "$1")" "$2" >/dev/null
}

# pin_created REF DIGEST -> the image's build timestamp (RFC 3339), or nothing.
# Reaching it means walking index -> linux/amd64 manifest -> config blob, which
# skopeo does for us via --override-os/--override-arch.
pin_created() {
    if [ "${BACKEND}" = "skopeo" ]; then
        skopeo inspect --override-os linux --override-arch amd64 \
            --format '{{.Created}}' "docker://$1@$2" 2>/dev/null || true
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
        curl -sSL --max-time 60 --fail \
            -H "Authorization: Bearer ${config_token}" \
            "${config_url}" 2>/dev/null \
            | jq -r '.created // empty' 2>/dev/null || true
    else
        curl -sSL --max-time 60 --fail "${config_url}" 2>/dev/null \
            | jq -r '.created // empty' 2>/dev/null || true
    fi
}

# tag_digest REF TAG -> the digest that tag points at today. Feeds the re-pin
# runbook: the value printed here is the value to paste into the Containerfile.
# The manifest-list digest is what we want (it keeps the reference
# multi-arch), which is exactly sha256 over the raw manifest bytes.
tag_digest() {
    if [ "${BACKEND}" = "skopeo" ]; then
        raw=$(skopeo inspect --raw "docker://$1:$2" 2>/dev/null) || return 0
        [ -n "${raw}" ] || return 0
        if command -v sha256sum >/dev/null 2>&1; then
            printf 'sha256:%s' "$(printf '%s' "${raw}" | sha256sum | cut -d' ' -f1)"
        elif command -v shasum >/dev/null 2>&1; then
            printf 'sha256:%s' "$(printf '%s' "${raw}" | shasum -a 256 | cut -d' ' -f1)"
        fi
        return 0
    fi

    tag_headers=$(mktemp)
    if curl_manifest "$(registry_host "$1")" "$(registry_path "$1")" "$2" "${tag_headers}" >/dev/null; then
        tr -d '\r' <"${tag_headers}" \
            | awk 'tolower($1) == "docker-content-digest:" { print $2; exit }'
    fi
    rm -f "${tag_headers}"
}

# iso_to_epoch RFC3339 -> seconds since epoch. GNU date, BSD date and python3
# all disagree here, and CI (Linux) and the maintainer's box (darwin) are not
# the same platform, so try each rather than assuming.
iso_to_epoch() {
    stamp="${1%%.*}"
    case "${stamp}" in
        *Z) ;;
        *) stamp="${stamp}Z" ;;
    esac
    date -u -d "${stamp}" +%s 2>/dev/null && return 0
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "${stamp}" +%s 2>/dev/null && return 0
    if command -v python3 >/dev/null 2>&1; then
        python3 -c 'import calendar,sys,time; print(calendar.timegm(time.strptime(sys.argv[1], "%Y-%m-%dT%H:%M:%SZ")))' \
            "${stamp}" 2>/dev/null && return 0
    fi
    return 1
}

age_days() {
    epoch=$(iso_to_epoch "$1") || return 1
    now=$(date -u +%s)
    printf '%s' "$(( (now - epoch) / 86400 ))"
}

# --------------------------------------------------------------------------
# Reference reporting
# --------------------------------------------------------------------------

report_pinned() {
    ref=$1
    digest=$2
    origin=$3
    checked=$((checked + 1))
    printf '\n%s\n' "${ref}@${digest}"
    note "declared in ${origin}, pinned by digest"

    if ! pin_resolvable "${ref}" "${digest}"; then
        finding "UNRESOLVABLE: the registry no longer serves this digest."
        note "This is the fedora-bootc failure mode (run 30702410393): every"
        note "build using this reference is broken until the pin is refreshed."
        note "Fix: resolve the tag to its current digest and update the pin --"
        note "see the re-pin runbook in docs/development/supply-chain.md."
        return
    fi
    note "resolvable: yes"

    created=$(pin_created "${ref}" "${digest}")
    if [ -z "${created}" ]; then
        note "age: unknown (registry did not expose a build timestamp)"
        return
    fi
    if ! days=$(age_days "${created}"); then
        note "age: unparseable timestamp ${created}"
        return
    fi
    if [ "${days}" -gt "${MAX_AGE_DAYS}" ]; then
        finding "STALE: built ${created} (${days} days ago, budget ${MAX_AGE_DAYS})."
        note "Still resolvable, so nothing is broken today -- but an old pin is"
        note "both the kind registries prune and ${days} days of missed upstream"
        note "fixes. Renovate opens the refresh PR once it is installed."
    else
        note "age: ${days} days (built ${created}), within the ${MAX_AGE_DAYS} day budget"
    fi
}

report_unpinned() {
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
        note "current digest: could not resolve (registry unreachable?)"
        return
    fi
    note "current digest: ${digest}"

    created=$(pin_created "${ref}" "${digest}")
    if [ -n "${created}" ]; then
        if days=$(age_days "${created}"); then
            note "tag content built ${created} (${days} days ago)"
        fi
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

if [ "${STRICT}" -eq 1 ]; then
    mode_label='strict (findings fail)'
else
    mode_label='advisory (findings warn)'
fi

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

printf '\n----\n'
printf '%s reference(s) checked, %s finding(s)\n' "${checked}" "${findings}"

if [ "${findings}" -eq 0 ]; then
    printf 'ANDROMEDA_PIN_FRESHNESS_OK\n'
    exit 0
fi

if [ "${STRICT}" -eq 1 ]; then
    printf 'ANDROMEDA_PIN_FRESHNESS_FAILED (strict)\n' >&2
    exit 1
fi

printf 'ANDROMEDA_PIN_FRESHNESS_WARN (advisory mode, not failing)\n'
exit 0
