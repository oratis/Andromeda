# Andromeda Developer Preview image

This directory turns the Rust control plane into an installable x86-64 UEFI
Developer Preview based on Fedora bootc 44 and KDE Plasma.

## Safety boundary

The default ISO boot entry starts the graphical Anaconda installer. The second
entry is destructive automation for CI and must never be selected on a machine
with data: its kickstart runs `zerombr` plus `clearpart --all --initlabel`,
which wipes **every** attached disk, not just the install target. A runtime
guard in `check-platform-compatibility.sh` refuses that path outside a VM, but
the blast radius is the whole machine.

Builds with `INSTALLER_DEFAULT=1` invert that GRUB default so unattended VMs
auto-install; those ISOs are named `*-ci.iso` and must never be distributed
as a developer preview image.

The image is a developer preview, not a claim of universal PC or Mac hardware
support. Hardware support remains gated by signed Hardware Compatibility
Manifests and real-machine certification.

## Build

On an x86-64 Linux host with Podman and privileged containers:

```bash
sudo os/scripts/build-iso.sh
```

The result is `output/Andromeda-Developer-Preview-x86_64.iso` plus a SHA-256
checksum. The build uses the current unified `image-builder` generic ISO
contract and embeds the v1 payload for offline installation. Setting
`INSTALLER_DEFAULT=1` (CI only) makes the destructive automated entry the
GRUB default and renames the output to
`Andromeda-Developer-Preview-x86_64-ci.iso`.

## End-to-end test

Install QEMU and OVMF, then run:

```bash
sudo env INSTALLER_DEFAULT=1 os/scripts/build-iso.sh
sudo os/scripts/test-install.sh
```

The test needs the `*-ci.iso` variant, root privileges, and — on hosts that
are not Debian/Ubuntu — `OVMF_CODE`/`OVMF_VARS_TEMPLATE` overrides for the
firmware paths.

The test boots the ISO with UEFI, automatically installs onto a new 64 GiB
VirtIO disk, removes the ISO, and starts the installed disk. The installed OS
must then:

1. have an Andromeda UEFI NVRAM entry plus a standard fallback loader;
2. run with SELinux enforcing and reach KDE's SDDM display manager;
3. start the loopback-only Andromeda task service;
4. generate a hardware report;
5. stage and boot revision 2 through bootc;
6. stage a rollback and boot revision 1 again.

The build also emits
`Andromeda-Developer-Preview-x86_64.manifest.json`, binding the ISO checksum,
payload digest, `pc_x86_64` platform variant, boot provider, and hardware
enablement profile. Installer preflight rejects Apple hardware and architecture
or payload-identity mismatches; Mac variants require separate guarded images.

In addition to the base lifecycle, the installed CI system enters a Plasma
Wayland session and exercises PipeWire, Flatpak, LibreOffice DOCX/XLSX/PPTX/PDF
conversion, a real Firefox Wayland launch, and persistent user data across update and
rollback. Success is the serial marker `ANDROMEDA_E2E_OK`. GitHub Actions
executes the same flow and uploads the ISO, checksum, and serial evidence.

The boot also produces `hardware-diagnosis.json`. Missing boot-critical storage,
network, graphics, or USB-controller drivers block the E2E run. After the full
lifecycle succeeds, run the pairwise controller matrix:

```bash
sudo os/scripts/test-hardware-matrix.sh
```

It boots independent overlays with Q35/NVMe/e1000e/XHCI, Q35/SATA/e1000e,
and i440fx/IDE/e1000/UHCI profiles. This validates emulated controller paths;
physical hardware remains gated by exact-machine HCM evidence.

On a disposable Google Compute Engine N2 host with nested KVM, run:

```bash
sudo env ANDROMEDA_SOURCE_REVISION="$(git rev-parse HEAD)" \
  os/scripts/test-gcp-nested.sh "$PWD" "$PWD/output"
```

GCP provisioning, evidence retrieval, and guaranteed instance deletion are
handled by the in-repo wrapper `os/scripts/gcp-run-e2e.sh`, which creates a
single labeled instance with `--max-run-duration` and deletes it from an EXIT
trap. See [the daily-driver E2E guide](../docs/development/daily-driver-e2e.md).

## Upstream contracts

- [image-builder generic bootc ISO](https://osbuild.org/docs/developer-guide/projects/image-builder/advanced/bootc/isos/)
- [image-builder container usage](https://osbuild.org/docs/developer-guide/projects/image-builder/installation/)
- [bootc install](https://bootc-dev.github.io/bootc/bootc-install.html)
- [Anaconda bootc Kickstart command](https://pykickstart.readthedocs.io/en/latest/commands.html#bootc)
