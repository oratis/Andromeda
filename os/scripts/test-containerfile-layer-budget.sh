#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
readonly CONTAINERFILE="${REPOSITORY_ROOT}/os/Containerfile"
readonly MINIMUM_PAYLOAD_DNF_LAYERS=20

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

printf 'ANDROMEDA_CONTAINERFILE_LAYER_BUDGET_OK payload_dnf_layers=%d\n' \
    "${payload_dnf_layers}"
