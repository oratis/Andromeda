---
name: andromeda-os-engineering
description: Research, design, implement, diagnose, test, and release the Andromeda AI-native desktop OS. Use when Codex works in oratis/Andromeda on OS architecture, Fedora bootc/KDE images, AI task-control contracts, hardware/HCM and driver support, Windows/macOS migration and compatibility, QEMU or GCP installation E2E, CI failures, product documentation, pull-request gating, or release evidence.
---

# Andromeda OS Engineering

Treat Andromeda as a consumer-OS program with explicit evidence boundaries, not as a generic
Linux customization. Preserve compatibility, reliability, constrained AI agency, and honest
hardware support claims together.

## Start from current facts

1. Locate the repository root and preserve unrelated or uncommitted user changes.
2. Run `python3 skills/andromeda-os-engineering/scripts/audit_repository.py .`.
3. Read [references/project-map.md](references/project-map.md) before changing architecture,
   status, component boundaries, or support claims.
4. Re-read repository source files before acting. Treat reference snapshots as navigation and
   memory, never as authority over newer code or CI evidence.
5. Separate three states in every report: implemented, automatically verified, and planned.

Load only the reference needed for the task:

- Product strategy, OS research, compatibility, or migration: read
  [references/research-and-product.md](references/research-and-product.md).
- Code, CI, ISO, QEMU/GCP, PR, merge, or release work: read
  [references/testing-and-release.md](references/testing-and-release.md).
- Failure diagnosis or reliability changes: read
  [references/incidents-and-guardrails.md](references/incidents-and-guardrails.md).

## Choose the workflow

### Research or plan

1. Search existing `docs/` material before adding a new document.
2. Browse current primary sources for facts that may have changed. Cite the exact upstream page,
   project documentation, standard, or research paper near each claim.
3. Compare open and closed systems by capability and evidence, not brand preference.
4. Convert findings into product decisions, explicit non-goals, risks, and measurable gates.
5. Update the relevant research document, architecture, product plan, and document index together
   when their claims change.

### Implement a control-plane change

1. Trace contracts across `andromeda-core`, `andromeda-policy`, `andromeda-runtime`,
   `andromeda-taskd`, and `andromeda-cli` before changing a shared type or state transition.
2. Keep model output untrusted. Enforce permissions, risk floors, isolation, confirmation, and
   state transitions in deterministic code.
3. Do not describe a policy simulation, API, or state transition as a real executor or sandbox.
4. Add failure-path, concurrency, persistence, and authorization tests with the implementation.
5. Update `docs/development/task-control-plane.md` when an API or security boundary changes.

### Implement an OS or installer change

1. Keep image updates atomic and retain update/rollback semantics.
2. Preserve the safe interactive installer as the product default. Restrict destructive unattended
   installation to clearly named CI artifacts.
3. Treat the installer live filesystem and target disk as different storage budgets.
4. Run the layer-budget, platform-guard, install, and hardware-matrix tests required by the change.
5. Never distribute the `*-ci.iso` artifact as a user-facing installer.

### Change hardware support

1. Keep `probe`, `diagnose`, HCM matching, installer preflight, and certification claims distinct.
2. Treat probe success as inventory only. Require signed, unexpired evidence for Supported or
   Certified claims.
3. Preserve the code-defined tier order: `blocked < community < reference < supported < certified`.
4. Do not generalize QEMU evidence to physical PCs, Intel/T2 Macs, or Apple silicon.
5. Update schema, matcher tests, hardware docs, and certification plan together when HCM semantics
   change.

### Diagnose CI or E2E

1. Bind every conclusion to the exact run head SHA.
2. Inspect the failed step and retained serial/diagnostic artifacts before editing code.
3. Distinguish deterministic regressions from mirrors, runners, cancellation, or concurrent branch
   updates.
4. Reproduce with the narrowest local guard, then run the proportional wider suite.
5. Add a deterministic regression guard when the failure exposed an invariant.

### Publish and merge

1. Confirm the intended diff and clean worktree.
2. Run the local validation matrix from
   [references/testing-and-release.md](references/testing-and-release.md).
3. Push a `codex/` branch and open a focused PR.
4. Inspect repository rules, required reviews, and branch protection. Treat absent server-side
   rules as a reason to enforce this workflow more carefully, not as permission to skip it.
5. Run `python3 skills/andromeda-os-engineering/scripts/check_merge_gates.py <pr>`.
6. Merge only when the exact PR head has all required checks concluded `SUCCESS`; `NEUTRAL`,
   `SKIPPED`, cancelled, stale, or pending required checks do not pass. For dependent PRs, merge
   bottom-up; for independent PRs, audit overlap before sequential merges.
7. After the last merge, verify the latest `main` SHA itself. Wait for its CI and, when triggered,
   Installable OS E2E. Do not reuse evidence from an older tree.
8. Report the final SHA, direct run links, merged/open PR counts, worktree state, and evidence limits.

## Required engineering invariants

- Never promise “all hardware.” Describe tested cohorts and unsupported boundaries precisely.
- Never grant a model ambient administrator authority.
- Never let documentation claim a capability that code and evidence do not provide.
- Never bypass a failed install E2E to merge an OS-sensitive change.
- Never overwrite concurrent owner work; fetch, audit, and incorporate or isolate it.
- Never leave disposable GCP resources running. Prefer the repository wrapper with platform-side
  lifetime deletion and an EXIT cleanup trap.
- Never treat a green PR from one SHA as proof for a later main SHA without tree or run evidence.

## Bundled tools

- `scripts/audit_repository.py`: inspect Git, GitHub PR, and recent run state without mutation.
- `scripts/check_merge_gates.py`: evaluate exact-head PR checks using the repository's path-based
  Installable OS trigger rules.

Run both with `--help` for options. Keep them read-only and deterministic.
