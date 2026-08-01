#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly CONTAINERFILE="${REPOSITORY_ROOT}/os/Containerfile"
readonly MINIMUM_PAYLOAD_DNF_LAYERS=20
readonly OUTPUT_DIR="${1:-${ANDROMEDA_OUTPUT_DIR:-${REPOSITORY_ROOT}/output}}"

# Which checks to run (ANDROMEDA_LAYER_BUDGET_MODE):
#   count - static Containerfile DNF-layer floor only; needs no built image, so
#           CI runs it BEFORE the build (and it is the unit-runnable path).
#   size  - largest real OCI layer vs the installer overlay budget; CI runs it
#           AFTER the build against the produced podman-history JSON. This mode
#           FAILS (does not skip) when the history JSON is missing, so the size
#           guard can never silently no-op the way it did while it only ran
#           pre-build.
#   all   - both checks (default). The size check is SKIPPED, not failed, when
#           no history JSON exists, keeping the guard unit-runnable with no
#           image built.
readonly LAYER_BUDGET_MODE="${ANDROMEDA_LAYER_BUDGET_MODE:-all}"

# Upper bound for any single OCI layer bootc stages one-at-a-time into the
# installer's small /var/tmp overlay. Default 3 GiB (3221225472) is a HEADROOM
# value, not the measured ceiling: it is chosen so the first post-build CI run
# PASSES on the known-good image while still catching gross over-consolidation.
# Once os-e2e prints the ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES_OBSERVED line, tighten
# this toward OBSERVED plus a small margin. Override per build via
# ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES.
readonly MAXIMUM_PAYLOAD_LAYER_BYTES="${ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES:-3221225472}"

case "${LAYER_BUDGET_MODE}" in
    count | size | all) ;;
    *)
        printf 'ANDROMEDA_LAYER_BUDGET_MODE must be count, size, or all, got: %s\n' \
            "${LAYER_BUDGET_MODE}" >&2
        exit 1
        ;;
esac

# The Anaconda live environment has a small writable overlay. bootc stages one
# OCI blob at a time in /var/tmp while importing the payload, so combining the
# desktop and firmware transactions into a handful of very large OCI layers
# can exhaust the installer even when the target disk has ample free space.
# The DNF-layer-count floor is a proxy for "no single over-consolidated blob".
check_layer_count() {
    local payload_dnf_layers
    payload_dnf_layers="$(
        awk '
            /^FROM payload AS installer$/ { exit }
            /^RUN dnf -y install \\/ { count++ }
            END { print count + 0 }
        ' "${CONTAINERFILE}"
    )"

    if ((payload_dnf_layers < MINIMUM_PAYLOAD_DNF_LAYERS)); then
        printf 'Payload has %d DNF layers; at least %d are required by the verified installer layer budget.\n' \
            "${payload_dnf_layers}" "${MINIMUM_PAYLOAD_DNF_LAYERS}" >&2
        printf 'Do not consolidate payload transactions without a successful 8 GiB-RAM blank-disk bootc install E2E.\n' >&2
        return 1
    fi

    printf 'ANDROMEDA_CONTAINERFILE_LAYER_BUDGET_OK payload_dnf_layers=%d\n' \
        "${payload_dnf_layers}"
}

# The count floor above cannot catch "20 small layers + 1 huge layer", which
# would still overflow the installer /var/tmp overlay while bootc stages that
# one blob (the real cause of the recent bootc install failure). Once
# build-iso.sh has produced andromeda-v1-history.json, assert the largest single
# layer stays under budget and always publish the observed maximum.
check_layer_size() {
    local history_json max_layer_bytes
    history_json="${ANDROMEDA_LAYER_HISTORY_JSON:-${OUTPUT_DIR}/andromeda-v1-history.json}"

    if [[ ! -f "${history_json}" ]]; then
        if [[ "${LAYER_BUDGET_MODE}" == "size" ]]; then
            printf 'Layer-size check requires %s but it does not exist.\n' \
                "${history_json}" >&2
            printf 'Run this in size mode only after build-iso.sh has produced the payload history JSON.\n' >&2
            return 1
        fi
        printf 'ANDROMEDA_CONTAINERFILE_LAYER_SIZE_SKIP reason=no-history-json path=%s\n' \
            "${history_json}"
        return 0
    fi

    max_layer_bytes="$(
        python3 - "${history_json}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    layers = json.load(handle)


def layer_size(layer):
    # `podman history --format json` serializes each layer's byte count under
    # the lowercase "size" key (see podman's ImageHistoryLayer json tag);
    # `--human=false` guarantees it is the raw integer. Accept "Size" too so the
    # guard stays correct across podman versions.
    value = layer.get("size", layer.get("Size", 0))
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


print(max((layer_size(layer) for layer in layers), default=0))
PY
    )"

    if ! [[ "${max_layer_bytes}" =~ ^[0-9]+$ ]]; then
        printf 'Could not parse layer sizes from %s.\n' "${history_json}" >&2
        return 1
    fi

    # Always publish the observed maximum (pass OR fail) so the real baseline is
    # greppable from CI logs and the threshold can be tightened with data.
    printf 'ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES_OBSERVED=%d threshold=%d\n' \
        "${max_layer_bytes}" "${MAXIMUM_PAYLOAD_LAYER_BYTES}"

    if ((max_layer_bytes > MAXIMUM_PAYLOAD_LAYER_BYTES)); then
        printf 'Largest image layer is %d bytes; the installer overlay budget is %d bytes.\n' \
            "${max_layer_bytes}" "${MAXIMUM_PAYLOAD_LAYER_BYTES}" >&2
        printf 'A single oversized layer can overflow the installer /var/tmp overlay during bootc staging even when the target disk has ample free space.\n' >&2
        printf 'Read the ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES_OBSERVED line above: either split the payload transaction, or if this size is intended raise ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES toward OBSERVED plus a small margin.\n' >&2
        return 1
    fi

    printf 'ANDROMEDA_CONTAINERFILE_LAYER_SIZE_OK max_layer_bytes=%d budget_bytes=%d\n' \
        "${max_layer_bytes}" "${MAXIMUM_PAYLOAD_LAYER_BYTES}"
}

case "${LAYER_BUDGET_MODE}" in
    count)
        check_layer_count
        ;;
    size)
        check_layer_size
        ;;
    all)
        check_layer_count
        check_layer_size
        ;;
esac
