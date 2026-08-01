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
# SECURITY: --target-imgref is a mutable, unsigned tag. Before trusting remote
# `bootc switch`/upgrade from this ref, a bootc signing policy
# (sigstore/containers-policy.json pinning a trusted signing key) is a REQUIRED
# follow-up; without it a compromised registry or retagged image is booted on
# the next update. See security-review.md finding #4.
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
