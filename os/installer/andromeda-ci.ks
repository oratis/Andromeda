cmdline
eula --agreed
firstboot --disable
keyboard us
lang en_US.UTF-8
network --bootproto=dhcp --device=link --activate
rootpw --lock
selinux --enforcing
timezone UTC --utc
user --name=andromeda --lock --gecos="Andromeda E2E"

zerombr
clearpart --all --initlabel
reqpart --add-boot
part / --fstype=ext4 --size=8192 --grow --label=andromeda-root

bootloader --timeout=1 --append="console=tty0 console=ttyS0,115200n8 andromeda.ci=1"
# SECURITY: --target-imgref is a mutable, unsigned tag. Remote `bootc
# switch`/upgrade from this ref is NOT signature-enforced yet: no signing key
# exists, and enabling a strict fail-closed policy now would break the unsigned
# :edge update flow os-e2e exercises. A not-yet-enforced policy template and the
# cosign release/activation runbook live at os/signing/policy.json.example and
# docs/development/installable-preview.md ("bootc image signing runbook"). Until
# a key exists, a compromised registry or retagged image would be booted on the
# next update. See security-review.md finding #4.
bootc --source-imgref=containers-storage:localhost/andromeda:v1 --target-imgref=ghcr.io/oratis/andromeda:edge --stateroot=andromeda
shutdown

%pre --erroronfail --log=/tmp/andromeda-installer-preflight.log
/usr/libexec/andromeda-installer-preflight ci
%end

%onerror --log=/tmp/andromeda-installer-onerror.log
/usr/libexec/andromeda-collect-anaconda-diagnostics
%end

%post --nochroot --erroronfail --log=/tmp/andromeda-uefi-fallback.log
/usr/libexec/andromeda-install-uefi-fallback /mnt/sysimage /mnt/sysroot ci
%end
