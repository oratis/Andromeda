# Andromeda project map

## Source-of-truth order

Use this precedence when facts disagree:

1. Current code, schemas, and workflow definitions.
2. Exact-head automated evidence and retained artifacts.
3. Development documentation.
4. Product plans and research documents.
5. This skill's dated memory snapshot.

Do not silently reconcile a disagreement. Fix the stale layer or report it explicitly.

## Product definition

Andromeda targets existing PC and selected Mac hardware as an AI-native personal desktop OS. Its
three product pillars are:

- Compatibility: preserve games, Office workflows, files, web/native apps, and isolated legacy
  environments where needed.
- Reliability: use image-based atomic updates, rollback, explicit disk budgets, attributable
  background work, and separation of system and user state.
- Agency: let an AI plan and use semantic tools while deterministic policy, capability brokers,
  isolation, verification, and confirmation retain authority.

The project intentionally does not build a new kernel, promise every Windows application/game,
productize macOS on non-Apple hardware, or give an AI default administrator access.

## Implemented component map

| Path | Responsibility |
|---|---|
| `crates/andromeda-core` | Intent/action plans, risk, capabilities, evidence, recovery semantics, task state machine |
| `crates/andromeda-policy` | Deny-first deterministic authorization, scopes, expiry, isolation and confirmation |
| `crates/andromeda-runtime` | Durable task records, locking, DAG validation, events and optimistic concurrency |
| `crates/andromeda-taskd` | Loopback-only development HTTP control plane |
| `crates/andromeda-cli` | Task operations, policy evaluation and hardware commands |
| `crates/andromeda-hardware` | Cross-platform privacy-aware probe, diagnosis, HCM matching and verification |
| `os/Containerfile` | Fedora bootc payload and installer image composition |
| `os/installer` | Kickstarts, compatibility preflight, diagnostics and safe/CI installer profiles |
| `os/scripts` | ISO build, QEMU lifecycle, matrix, GCP and regression harnesses |
| `schemas` | Machine-readable contracts, including HCM |
| `docs/research` | Current OS/ecosystem research and adoption analysis |
| `docs/development` | Build, API, hardware, certification and E2E operations |

## Stable architectural decisions

- Build the primary x86-64 system on Fedora bootc with KDE Plasma/Wayland.
- Keep `/usr` image-managed and preserve multiple bootable deployments.
- Keep product APIs independent from a single low-level update implementation where possible.
- Route incompatible workloads through explicit compatibility domains such as Flatpak, Wine/Proton,
  web, OCI, or an isolated Windows workspace rather than contaminating the base system.
- Use platform-specific boot providers: PC UEFI, Intel Mac Apple EFI, Apple-silicon boot policy and
  Asahi stack. Equivalent recovery semantics do not imply identical firmware mechanisms.
- Make hardware support cohort- and evidence-based through HCM, not a generic Linux compatibility
  assertion.
- Keep taskd loopback-only until authentication, identity, brokered execution, and multi-tenant
  boundaries exist.

## Evidence boundaries

Current automated virtual evidence can prove x86-64 QEMU/OVMF installation, Plasma startup,
selected daily workflows, bootc update/rollback, user-data persistence, and a bounded virtual device
matrix. It cannot certify physical GPU, Wi-Fi, Bluetooth, camera, suspend, thermal, firmware,
Secure Boot/TPM, gaming performance/anti-cheat, Intel/T2 Mac, or Apple silicon behavior.

Use these support terms precisely:

- `blocked`: selector/requirements/evidence do not permit use.
- `community`: inventory or community experience without product evidence.
- `reference`: virtual L0-L2 reference evidence; lower than physical Supported.
- `supported`: exact cohort with required, current evidence and ownership.
- `certified`: highest tested and signed product commitment.

“OEM Reference Design” is a product-line label, not `SupportTier::Reference`.

## Current snapshot

As of 2026-08-01, the repository had an installable x86-64 virtual Daily Driver Candidate, Rust task
control plane, HCM v2/hardware diagnosis, QEMU lifecycle and pairwise virtual hardware E2E. PRs
#1-#14 were merged and `main` at `ded31f19639ac1b032b879c1e113c05e1d21d15e` passed CI and
Installable OS E2E.

This snapshot is navigation, not a permanent release claim. Re-run `audit_repository.py`, inspect
the current README, and verify current exact-head Actions runs before reporting status.
