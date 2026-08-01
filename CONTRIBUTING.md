# Contributing to Andromeda

Andromeda is in an early architecture and prototype phase. Contributions should reduce uncertainty while preserving explicit security and hardware-support boundaries.

## Local checks

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
git diff --check
```

Run the platform probe as an additional smoke test:

```bash
cargo run --locked --bin andromeda -- hardware probe
```

## Pull requests

- Keep one architectural concern per PR.
- Explain the user outcome, security boundary, failure path, and validation.
- Add tests for every state transition, policy rule, parser, or compatibility decision.
- Do not silently expand a hardware tier or application compatibility claim.
- Do not add privileged execution, network listening beyond loopback, credential access, firmware writes, disk mutation, or external side effects without a threat model and explicit maintainer review.
- Prefer public, versioned interfaces and upstream contributions over long-lived forks.

## Architecture decisions

Material decisions should include an ADR under [`docs/adr/`](docs/adr/) — copy
[`docs/adr/0000-template.md`](docs/adr/0000-template.md) — with:

1. context and user problem;
2. options considered;
3. selected decision;
4. security, reliability, compatibility, and licensing consequences;
5. an exit or replacement condition.

## Rust

- Rust 1.85 is the current minimum supported version.
- Unsafe Rust is forbidden across the workspace.
- Public fallible functions document their error conditions.
- Models may propose data, but deterministic code owns policy, state transitions, execution, verification, and persistence.

## Licensing

Contributions are accepted under Apache-2.0. New dependencies must have a compatible license and a clear upstream source. Firmware, fonts, codecs, model weights, proprietary SDKs, and data files require artifact-level review rather than assumptions based on the surrounding project.
