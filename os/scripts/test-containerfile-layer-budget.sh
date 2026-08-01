#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly CONTAINERFILE="${REPOSITORY_ROOT}/os/Containerfile"
readonly MINIMUM_PAYLOAD_DNF_LAYERS=20
readonly OUTPUT_DIR="${1:-${ANDROMEDA_OUTPUT_DIR:-${REPOSITORY_ROOT}/output}}"
# Upper bound for any single OCI layer bootc stages one-at-a-time into the
# installer's small /var/tmp overlay. Default 4 GiB; override for a build whose
# base layers are known-larger via ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES.
readonly MAXIMUM_PAYLOAD_LAYER_BYTES="${ANDROMEDA_MAX_PAYLOAD_LAYER_BYTES:-4294967296}"

# The Anaconda live environment has a small writable overlay. bootc stages one
# OCI blob at a time in /var/tmp while importing the payload, so combining the
# desktop and firmware transactions into a handful of very large OCI layers
# can exhaust the installer even when the target disk has ample free space.
payload_dnf_layers="$({
    awk '
        /^FROM payload AS installer$/ { exit }
        /^RUN dnf -y install \\/ { count++ }
        END { print count + 0 }
    ' "${CONTAINERFILE}"
})"
readonly payload_dnf_layers

if (( payload_dnf_layers < MINIMUM_PAYLOAD_DNF_LAYERS )); then
    printf 'Payload has %d DNF layers; at least %d are required by the verified installer layer budget.\n' \
        "${payload_dnf_layers}" "${MINIMUM_PAYLOAD_DNF_LAYERS}" >&2
    printf 'Do not consolidate payload transactions without a successful 8 GiB-RAM blank-disk bootc install E2E.\n' >&2
    exit 1
fi

# The DNF-layer-count floor above is a proxy for "no single over-consolidated
# blob". It cannot catch "20 small layers + 1 huge layer", which would still
# overflow the installer /var/tmp overlay while bootc stages that one blob (the
# real cause of the recent bootc install failure). When build-iso.sh has already
# produced andromeda-v1-history.json, directly assert the largest single layer
# stays under budget. In the unit-test CI context no image has been built, so
# the size data is unavailable and this check is skipped with a log line rather
# than failing.
history_json="${ANDROMEDA_LAYER_HISTORY_JSON:-${OUTPUT_DIR}/andromeda-v1-history.json}"
if [[ -f "${history_json}" ]]; then
    max_layer_bytes="$(
        python3 - "${history_json}" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    layers = json.load(handle)


def layer_size(layer):
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
        exit 1
    fi
    if (( max_layer_bytes > MAXIMUM_PAYLOAD_LAYER_BYTES )); then
        printf 'Largest image layer is %d bytes; the verified installer overlay budget is %d bytes.\n' \
            "${max_layer_bytes}" "${MAXIMUM_PAYLOAD_LAYER_BYTES}" >&2
        printf 'A single oversized layer can overflow the installer /var/tmp overlay during bootc staging even when the target disk has ample free space.\n' >&2
        exit 1
    fi
    printf 'ANDROMEDA_CONTAINERFILE_LAYER_SIZE_OK max_layer_bytes=%d budget_bytes=%d\n' \
        "${max_layer_bytes}" "${MAXIMUM_PAYLOAD_LAYER_BYTES}"
else
    printf 'ANDROMEDA_CONTAINERFILE_LAYER_SIZE_SKIP reason=no-history-json path=%s\n' \
        "${history_json}"
fi

printf 'ANDROMEDA_CONTAINERFILE_LAYER_BUDGET_OK payload_dnf_layers=%d\n' \
    "${payload_dnf_layers}"
