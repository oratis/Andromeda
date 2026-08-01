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

## Current security invariants

- Untrusted plans cannot choose their own risk floor.
- Deny policy overrides capability grants.
- A capability is resource-scoped, expires independently, and contains no secret value.
- L2 actions require a strong-isolation decision; L3 actions require a brokered decision and confirmation.
- Task state cannot move directly from running to succeeded without verification.
- Task writes use atomic replacement, cross-process locking, and optimistic revision checks.
- Hardware reports omit serial numbers and do not themselves grant a support tier.

The current runtime evaluates policies but does not execute tools. Executor, credential broker, signed policy/HCM, append-only audit, sandbox attestation, and remote authentication are not implemented yet and must not be implied by integrations.
