# Testing, cloud E2E and release workflow

## Contents

- [Local validation ladder](#local-validation-ladder)
- [GitHub checks](#github-checks)
- [GCP nested-KVM workflow](#gcp-nested-kvm-workflow)
- [PR and merge workflow](#pr-and-merge-workflow)
- [Release evidence](#release-evidence)

## Local validation ladder

Run the narrowest relevant tests first, then the required wider gates.

Run the Rust baseline for every Rust, OS image, installer, or shared build change. Documentation-only
changes may omit it when they cannot affect generated artifacts or code:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

For installer/OS changes, also run:

```bash
shellcheck os/scripts/*.sh os/installer/*.sh os/files/usr/libexec/*
os/scripts/test-installer-platform-guard.sh
os/scripts/test-containerfile-layer-budget.sh
```

Run full local QEMU E2E only on a Linux/KVM host with the documented dependencies:

```bash
sudo os/scripts/build-iso.sh "$PWD/output"
os/scripts/test-containerfile-layer-budget.sh "$PWD/output"
sudo os/scripts/test-install.sh "$PWD/output"
sudo os/scripts/test-hardware-matrix.sh "$PWD/output"
```

The second layer-budget invocation is intentional: before a build, the test can only enforce the
Containerfile transaction-count proxy; after a build, `output/andromeda-v1-history.json` lets it
enforce the maximum real single-layer size. If no history file exists, record that the size check was
skipped and rely on neither the proxy nor build success as a substitute for blank-disk install E2E.

Follow `docs/development/installable-preview.md` and
`docs/development/daily-driver-e2e.md` for current prerequisites and artifacts.

## GitHub checks

`.github/workflows/ci.yml` runs on every PR and main push:

- format, clippy and workspace tests;
- installer platform and layer-budget guards;
- Linux, macOS and Windows hardware probes.

`.github/workflows/os-e2e.yml` runs when these paths change:

- `.github/workflows/os-e2e.yml`;
- `Cargo.toml` or `Cargo.lock`;
- `crates/**`;
- `os/**`.

Its required functional sequence is:

1. Validate scripts and guards.
2. Build payloads and the destructive, clearly named CI ISO.
3. Install to a blank UEFI disk and boot without the ISO.
4. Verify first boot, revision update, reboot, rollback and data persistence.
5. Verify the pairwise virtual hardware matrix.
6. Upload serial diagnostics and installer artifacts even on failure.

Do not infer success from a green build step. Require the entire job conclusion `success` on the
exact head SHA.

## GCP nested-KVM workflow

Prefer the auditable repository wrapper:

```bash
ANDROMEDA_GCP_PROJECT=<project-id> os/scripts/gcp-run-e2e.sh
```

The wrapper creates one labeled disposable N2 instance, sets a platform-side maximum run duration
with delete-on-termination, runs nested KVM, downloads `output/gcp-evidence/`, and deletes the
instance in an EXIT trap. If the `gcp-os-e2e` Codex skill is installed, use it for controlled VM
operations and evidence inspection; keep the repository wrapper as the project source of truth.

Before running cloud E2E:

- confirm project, zone, machine type, disk size and cost/lifetime bound;
- confirm authenticated `gcloud` identity without printing secrets;
- archive the exact Git revision, not a dirty worktree;
- retain instance labels and the manual deletion command;
- verify deletion after success, failure or interruption.

GCP virtual evidence does not certify physical hardware, battery/suspend, firmware, real GPU,
wireless, cameras, Macs, games or anti-cheat.

## PR and merge workflow

1. Create a focused `codex/` branch from current `origin/main`.
2. Keep research/docs, contracts, runtime, OS image, and hardware changes separable when useful.
3. Inspect sibling PR file overlap and dependency topology before merging.
4. Inspect branch protection, repository rules and required reviews. Satisfy them when present; when
   absent, keep the exact-head process gates mandatory and report that enforcement is procedural.
5. For a dependent chain, merge leaf/bottom PRs first so each parent receives the verified tree.
6. For independent siblings, wait for each exact-head gate and merge sequentially.
7. Expect GitHub concurrency to cancel superseded main runs; retain only the latest main SHA run.
8. After all merges, fetch `origin/main`, verify open PR count, worktree cleanliness and latest runs.
9. Require a final main CI and, when path-triggered, final main Installable OS success.

Use:

```bash
python3 skills/andromeda-os-engineering/scripts/check_merge_gates.py <pr-number>
python3 skills/andromeda-os-engineering/scripts/audit_repository.py . --github
```

Do not enable or bypass unsafe repository-wide auto-merge merely to satisfy an “automatic merge”
request. Implement check-gated sequencing with auditable merge commands.

Every named required check must conclude `SUCCESS`. Do not accept `NEUTRAL` or `SKIPPED` for a
required gate merely because GitHub represents those as non-failing conclusions.

## Release evidence

Report:

- final main SHA and repository URL;
- direct CI and E2E run URLs for that SHA;
- install/build/matrix step conclusions;
- merged and open PR counts;
- local worktree state;
- installer artifact type and retention;
- virtual-versus-physical evidence boundary;
- any non-blocking workflow deprecation warnings.

Never present the destructive `*-ci.iso` as a consumer installer. A consumer-facing artifact must
use the safe interactive installer default and its own signed/reproducible release process.
