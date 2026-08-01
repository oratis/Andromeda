# Research and product memory

## User problem framing

The motivating trade-off is concrete:

- Windows remains valuable for PC games, Microsoft Office, peripheral/application breadth, and
  long-tail file-format handling.
- macOS remains valuable for low-maintenance reliability, coherent hardware/software integration,
  Unix development, and strong AI/developer tooling.
- Windows maintenance can violate user expectations: a feature update may create `Windows.old`,
  consume the system volume, and make cleanup decisions entangled with rollback or application
  state. Andromeda must expose ownership, budgets, consequences, and recovery before cleanup.

Do not reduce this to “clone Windows plus macOS styling.” Preserve workload compatibility while
changing the reliability and authority model.

## Research protocol

1. Search `docs/research/` and the architecture document before starting.
2. Build a capability taxonomy: kernel/driver model, application compatibility, updates/recovery,
   security/isolation, desktop UX, AI integration, migration, licensing and governance.
3. Cover relevant commercial and open-source systems, but research by architecture families rather
   than producing an unbounded brand list.
4. Browse current primary sources for changing facts: upstream project docs, vendor support pages,
   standards, kernel/driver documentation and research papers.
5. Record license, maintenance health, hardware scope, integration cost, exit strategy and security
   boundary for every adoption candidate.
6. Separate fact, inference and recommendation. Attach dates to volatile support claims.
7. Convert findings into an adoption decision: adopt, adapt, isolate, watch or reject.

The existing research routes are:

- `docs/os-landscape-and-andromeda-architecture.md`
- `docs/research/open-source-adoption-matrix.md`
- `docs/research/windows-gaming-office-formats.md`
- `docs/research/reliability-update-ai-agent.md`
- `docs/research/desktop-platform-and-distribution.md`
- `docs/research/hardware-drivers-and-migration.md`

## Product principles to preserve

1. Compatibility is valuable, but compatibility layers must not mutate the system core.
2. Reliable defaults outrank arbitrary mutability; expose an explicit developer mode.
3. An update is an image/state transition, not an uncontrolled in-place construction project.
4. AI output is a proposal. Deterministic components own permission and system truth.
5. High-impact actions have visible commit points; external irreversible effects require final
   confirmation.
6. Every automation has a visible principal, reason, resource budget and stop control.
7. System rollback does not silently roll back or delete user data.
8. Prefer semantic APIs over simulated clicks.
9. Do not promise compatibility or hardware support without a passing test artifact.
10. Prefer upstream solutions and retain replacement paths for models, clouds and stores.

## Compatibility routing

Use a portfolio rather than one universal compatibility layer:

- Native/Flatpak for supported Linux desktop apps.
- Web/PWA for service-first workflows.
- Wine/Proton for verified Windows applications and games.
- Isolated Windows Workspace for Office fidelity, specialized software and incompatible anti-cheat
  or drivers.
- Format routing with fidelity classification, preview, conversion and original-file preservation.
- Migration manifests with checksums, import scope, rollback and a retained source-system path.

Never claim Microsoft Office fidelity from LibreOffice smoke tests alone. Never claim gaming support
from Steam/Proton installation alone; test the title, GPU path, DRM/anti-cheat and performance cohort.

## Hardware and Mac strategy

- Treat selected x86-64 PCs as the first physical certification candidates.
- Treat non-T2 Intel Mac by exact model cohort; T2 requires a distinct experimental path.
- Treat M1/M2 through an Asahi-based platform branch and independent evidence.
- Keep newer Apple-silicon models in Watch until upstream machine support and install path exist.
- Do not turn upstream development activity into a delivery commitment.

## AI-native system boundary

Codex and Claude Code are interaction and execution-loop inspirations: understand intent, plan, use
tools, inspect results and iterate. The OS version must add boundaries they do not inherently supply:

- versioned machine-readable plans;
- risk floors that the model cannot lower;
- scoped, expiring capabilities bound to the task/action subject;
- per-action isolation;
- independent verification and evidence;
- durable events, rollback/compensation and user-visible resource accounting.

Until a real executor, credential broker and verifier exist, describe the current control plane as
contract/policy/persistence infrastructure, not an autonomous privileged OS agent.
