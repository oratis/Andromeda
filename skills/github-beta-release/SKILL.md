---
name: github-beta-release
description: Cut, verify, republish, or promote an Andromeda beta pre-release on GitHub. Use when asked to release a beta, tag a beta build, publish beta artifacts (CLI binaries or the Developer Preview ISO), fix a failed beta publish, or explain how the beta release pipeline works.
---

# GitHub beta release

Beta releases are GitHub **pre-releases** built by
[.github/workflows/release-beta.yml](../../.github/workflows/release-beta.yml).
One tag produces one immutable set of artifacts; nothing is published outside
that workflow.

## Invariants

1. **Tag format is `v<workspace-version>-beta.<N>`** (e.g. `v0.1.0-beta.1`).
   The workflow fails any tag whose version does not match
   `[workspace.package].version` in `Cargo.toml`. Bump the workspace version
   first when starting a new beta line.
2. **Only the safe interactive ISO ships.** The workflow builds with
   `INSTALLER_DEFAULT=0`. The destructive `*-ci.iso` variant is a CI-only
   input and must never appear in a release (see
   `skills/andromeda-os-engineering/references/testing-and-release.md`).
3. **Every asset is covered by `SHA256SUMS`.** Per-asset `.sha256` files are
   checked and folded into one `SHA256SUMS` at publish time.
4. **Betas are always `--prerelease`.** Promotion to stable is a separate,
   deliberate step — never edit a beta release into a stable one.

## Cut a beta

```bash
git checkout main && git pull --ff-only
cargo test --workspace --locked   # optional local gate; CI re-runs it
git tag v0.1.0-beta.1             # match [workspace.package].version
git push origin v0.1.0-beta.1
```

The tag push triggers the workflow, which:

1. **preflight** — validates the tag format and version match, runs the full
   test suite.
2. **cli** — builds `andromeda` for `x86_64-unknown-linux-gnu`,
   `aarch64-apple-darwin`, and `x86_64-pc-windows-msvc`; packages
   `andromeda-<tag>-<target>.tar.gz` (`.zip` on Windows) with `.sha256`.
3. **iso** — runs `os/scripts/build-iso.sh` (interactive installer default)
   producing `Andromeda-Developer-Preview-x86_64.iso`, its `.sha256`, and the
   build `manifest.json`.
4. **publish** — verifies all checksums, writes `SHA256SUMS`, and creates the
   pre-release with `gh release create --prerelease --verify-tag`. The ISO is
   currently ~4 GiB — over the 2 GiB release-asset limit — so in the normal
   case it ships as the run's `iso` workflow artifact (14-day retention) and
   the release notes link to the run; the `manifest.json` recording the ISO
   sha256 still attaches to the release. If the ISO ever fits under 2 GiB it
   attaches automatically.

## Verify a published beta

```bash
gh release view v0.1.0-beta.1
gh release download v0.1.0-beta.1 --dir /tmp/beta-verify
(cd /tmp/beta-verify && sha256sum --check SHA256SUMS)
```

Also confirm the release is marked **Pre-release** in the GitHub UI and that
the ISO asset (when attached) matches the `iso.sha256` value inside
`Andromeda-Developer-Preview-x86_64.manifest.json`.

## Republish after a failed run

Do **not** delete and re-push the tag; re-run against the existing tag:

```bash
gh workflow run release-beta.yml \
  --field tag=v0.1.0-beta.1 \
  --field build_iso=true
```

Dispatch reuses the tag's commit. `gh release upload --clobber` makes the
publish step idempotent, so a re-run replaces assets on the existing release
instead of failing. Set `build_iso=false` to republish only the CLI binaries
(for example when just the Windows build failed).

## Iterate a beta

A new fix means a new tag: `v0.1.0-beta.2`, `v0.1.0-beta.3`, … Never move an
existing beta tag — consumers may have already downloaded and checksummed its
artifacts.

## Promote to stable

There is deliberately no automated promotion. When a beta is accepted:

1. Tag the same commit as `v<version>` (no `-beta` suffix).
2. Build stable artifacts through a stable release process with its own
   signing/reproducibility evidence — do not re-label beta assets
   (release evidence requirements live in
   `skills/andromeda-os-engineering/references/testing-and-release.md`).

## Troubleshooting

- **preflight fails on version mismatch** — the tag says one version,
  `Cargo.toml` another. Fix the workspace version on `main`, then cut a fresh
  tag; never force-move the old one.
- **ISO job runs out of disk** — the free-disk-space step mirrors
  `os-e2e.yml`; if it still fails, the payload has grown. Check the layer
  budget scripts under `os/scripts/` before raising any limits.
- **publish says the release already exists** — expected on re-runs; assets
  are clobbered in place. If assets look stale, compare `SHA256SUMS` against
  the run's artifacts before touching the release.
