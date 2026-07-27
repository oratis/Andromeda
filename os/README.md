# Andromeda Developer Preview image

This directory turns the Rust control plane into an installable x86-64 UEFI
Developer Preview based on Fedora bootc 43 and KDE Plasma.

## Safety boundary

The normal ISO boot entry starts the graphical Anaconda installer. The second
entry is destructive automation for CI and must never be selected on a machine
with data: it wipes the first installation disk.

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
contract and embeds the v1 payload for offline installation.

## End-to-end test

Install QEMU and OVMF, then run:

```bash
sudo os/scripts/test-install.sh
```

The test boots the ISO with UEFI, automatically installs onto a new 32 GiB
VirtIO disk, removes the ISO, and starts the installed disk. The installed OS
must then:

1. have an Andromeda UEFI NVRAM entry plus a standard fallback loader;
2. run with SELinux enforcing and reach KDE's SDDM display manager;
3. start the loopback-only Andromeda task service;
4. generate a hardware report;
5. stage and boot revision 2 through bootc;
6. stage a rollback and boot revision 1 again.

Success is the serial marker `ANDROMEDA_E2E_OK`. GitHub Actions executes the
same flow and uploads the ISO, checksum, and serial evidence.

## Upstream contracts

- [image-builder generic bootc ISO](https://osbuild.org/docs/developer-guide/projects/image-builder/advanced/bootc/isos/)
- [image-builder container usage](https://osbuild.org/docs/developer-guide/projects/image-builder/installation/)
- [bootc install](https://bootc-dev.github.io/bootc/bootc-install.html)
- [Anaconda bootc Kickstart command](https://pykickstart.readthedocs.io/en/latest/commands.html#bootc)
