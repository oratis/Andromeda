# Incidents and engineering guardrails

## Installer target free space did not prevent live-overlay exhaustion

### Symptom

`bootc install` failed with a write error under `/var/tmp` and “no space left on device” even though
the target disk had tens of gigabytes free.

### Root cause

The Anaconda live environment had a small writable root overlay. bootc staged one OCI blob at a
time in live `/var/tmp` before importing it to the target. Consolidating many package transactions
into a few multi-gigabyte layers exceeded the live overlay; target-disk free space was irrelevant.

### Durable guard

Keep package payload layers bounded. `os/scripts/test-containerfile-layer-budget.sh` currently
enforces both a minimum transaction/layer count and, when image history exists, a maximum single
layer size. Run blank-disk install E2E before changing these budgets. Do not “optimize” layer count
without measuring the installer staging filesystem.

## Concurrent commits invalidated an otherwise useful run

### Symptom

A long E2E run built successfully but was cancelled or no longer represented the current branch
because another process pushed new commits.

### Guard

Record the run `headSha`, monitor the remote branch head, and treat a different SHA as new work.
Fetch and audit owner commits; never force-push over them. After merges, validate latest main rather
than reusing an earlier PR run unless tree identity is explicitly proven and no final run is needed.

## External package metadata can fail transiently

### Symptom

Fedora repository zchunk or checksum metadata failed while source and tests were unchanged.

### Guard

Inspect the precise logs and compare a retry on the same SHA. Do not edit product code for a mirror
or network failure. Add bounded retry only when it preserves supply-chain verification and does not
mask deterministic errors.

## Serial evidence extraction can produce false negatives

### Symptom

The guest emitted all success markers, but the outer collector failed because serial lines used CRLF
or contained ANSI/getty control sequences on the same physical line.

### Guard

Normalize the byte stream and extract only the explicit `ANDROMEDA_` printable marker fields. Keep
the raw serial log, normalized markers and collector version. A repaired extractor validated against
old logs is not equivalent to a fresh end-to-end pass on the repaired code.

## `Windows.old` is a product-requirement warning

The motivating Windows failure mode is not merely a cleanup bug: a feature update created a large
rollback directory, the system volume filled, and manual deletion appeared to remove or break parts
of the application environment.

Translate this into requirements:

- reserve and display update/rollback disk budgets before download;
- attribute every large object to system version, app, cache, user data or recovery;
- show exactly what cleanup removes and which rollback guarantee it expires;
- keep applications and user data outside disposable system deployments;
- make cleanup transactional and evidence-producing;
- preserve at least one known-good boot path until the user explicitly accepts its removal.

## Documentation drift is a release defect

Examples include confusing the product label “OEM Reference Design” with the low HCM
`SupportTier::Reference`, overstating Host-header DNS-rebinding checks as authentication, or
describing virtual probes as physical certification.

Keep code/schema enums authoritative, lock duplicated contracts with tests, and update README,
development docs and product plans in the same PR when a boundary changes.

## Non-blocking workflow warnings

GitHub can emit action-runtime deprecation warnings while all steps succeed. Record them as follow-up
maintenance, but do not relabel a successful run as failed. Conversely, never ignore a failed
functional step because artifact upload succeeded under `if: always()`.
