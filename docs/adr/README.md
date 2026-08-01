# Architecture Decision Records (ADRs)

This directory holds Andromeda's Architecture Decision Records. An ADR captures a
single material design decision, the context that forced it, and the
consequences the project accepts as a result. The practice is being established;
this directory currently contains only the template.

## When to write one

Write an ADR for any decision that is expensive to reverse or that changes a
system boundary — for example: a new privileged surface or trust boundary, a
change to the deterministic policy/state model, a hardware-support or
`SupportTier` semantics change, an update/rollback contract, or a dependency with
non-trivial licensing or provenance implications. Routine or easily reversible
changes do not need one. See [`CONTRIBUTING.md`](../../CONTRIBUTING.md) for the
required content of an ADR.

## Process

1. Copy [`0000-template.md`](./0000-template.md) to
   `NNNN-short-decision-title.md`, using the next unused four-digit number.
2. Fill in Context, Decision, and Consequences. Start the ADR at status
   **Proposed**.
3. Open a pull request with the ADR alongside the change it justifies. Keep one
   architectural concern per PR.
4. When the decision is agreed, set the status to **Accepted**. Never edit the
   substance of an accepted ADR; instead add a new ADR and mark the old one
   **Superseded by ADR-NNNN**.

## Index

| ADR | Title | Status |
|---|---|---|
| [0000](./0000-template.md) | Template (not a decision) | — |
