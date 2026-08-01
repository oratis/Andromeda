# ADR 0000: <short decision title>

- **Status:** Proposed <!-- Proposed | Accepted | Superseded by ADR-XXXX | Deprecated -->
- **Date:** YYYY-MM-DD
- **Deciders:** <names or roles>

## Context

What is the problem, and who has it? Capture the user problem, the forces at
play (security, reliability, hardware support, licensing, timeline), and the
options considered with their trade-offs. State the assumptions and constraints
that make this decision necessary now.

## Decision

The option that was selected, stated in one or two clear sentences. Be specific
enough that a reader can tell what will and will not be built.

## Consequences

The results of the decision, both positive and negative. Cover, where relevant:

- **Security** — new trust boundaries, privileged surfaces, threat-model deltas.
- **Reliability** — failure paths, recovery, and rollback behaviour.
- **Compatibility** — hardware tiers, application/format claims, and any
  `SupportTier` implications (do not silently expand a tier).
- **Licensing** — dependency licenses, redistribution, and provenance.

## Exit / replacement condition

The condition under which this decision should be revisited or replaced (for
example, a new upstream capability, a failed assumption, or a superseding ADR).
