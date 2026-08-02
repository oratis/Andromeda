# Security Policy

## Project maturity

Andromeda is a v0 engineering prototype and is not a production operating system. The current task service is non-privileged, has no authentication, and binds to loopback by default. Do not expose it to untrusted networks or use it to protect production secrets.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub Security Advisories for `oratis/Andromeda`. Include:

- affected commit and platform;
- threat model and required attacker access;
- minimal reproduction;
- expected and actual policy decision;
- impact on confidentiality, integrity, availability, recovery, or hardware safety.

Do not open a public issue for an unpatched vulnerability involving privilege boundaries, credential exposure, filesystem escape, task-policy bypass, unsafe update/recovery behavior, firmware, or destructive hardware actions.

## Threat model

[`docs/andromeda-threat-model.md`](./docs/andromeda-threat-model.md) is the v0 threat model: assets, subjects, attacker classes, trust boundaries, and — in its section 6 — the known-unfixed attack surface, ordered by exploitability. Read it before touching any privileged path; section 7 lists what a PR crossing a privilege boundary must supply.

Two boundaries are called out here because integrations get them wrong:

- **Malicious root and physical administrators are explicitly out of scope for v0.** Task records are not a tamper-evident ledger.
- **`andromeda hardware check` authenticates a manifest only when `--trusted-keys` is passed.** Without a keyring the manifest's declared tier is self-asserted and a forged file reaches `certified`; gating `--require-tier supported|certified` on such a check is refused outright.

## Current security invariants

- A plan cannot declare a risk level below the floor of the action kind it selected. It does still choose that kind, so classification itself is not yet independently checked (threat model section 4.1).
- Deny policy overrides capability grants, and path targets are lexically normalised before matching, so `..` traversal cannot dodge a deny root.
- A capability is resource-scoped, expires independently, and contains no secret value.
- L2 actions require a strong-isolation decision and L3 actions a brokered one, but the isolation level is **asserted by the caller, not attested by an execution environment** — no sandbox or microVM exists yet.
- An L3 external side effect cannot reach `running` unless the transition explicitly carries confirmation; the confirmation defaults to absent and is recorded on the state-change event with its actor. It is caller-asserted, not broker-attested.
- A task cannot reach `succeeded` unless every planned action has a recorded outcome that succeeded or was skipped and carries at least one piece of evidence. The evidence is recorded by the executing party; there is no independent verifier yet.
- Task writes use atomic replacement, cross-process locking, and optimistic revision checks.
- `taskd` refuses to bind to a non-loopback address at startup unless explicitly overridden. The `Host` header check defends browsers against DNS rebinding only and is not authentication.
- Hardware reports omit serial numbers and do not themselves grant a support tier.

The current runtime evaluates policies but does not execute tools. Executor, sandbox/microVM attestation, credential broker, independent verifier, confirmation broker, signed policy/HCM, append-only audit, local caller authentication, and remote authentication are not implemented yet and must not be implied by integrations. In particular, `taskd` has **no authentication of any kind**: any local process reaching loopback can drive the full API and mint its own capabilities.
