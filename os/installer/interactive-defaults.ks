# SECURITY: --target-imgref is a mutable, unsigned tag. Before trusting remote
# `bootc switch`/upgrade from this ref, a bootc signing policy
# (sigstore/containers-policy.json pinning a trusted signing key) is a REQUIRED
# follow-up; without it a compromised registry or retagged image is booted on
# the next update. See security-review.md finding #4.
bootc --source-imgref=containers-storage:localhost/andromeda:v1 --target-imgref=ghcr.io/oratis/andromeda:edge
selinux --enforcing

%pre --erroronfail --log=/tmp/andromeda-installer-preflight.log
/usr/libexec/andromeda-installer-preflight interactive
%end

# Leave disk-side evidence on the ESP when an interactive install fails, so a
# human can pull the drive and read the anaconda/bootc diagnostics afterwards.
%onerror --log=/tmp/andromeda-installer-onerror.log
/usr/libexec/andromeda-collect-anaconda-diagnostics
%end

%post --nochroot --erroronfail --log=/tmp/andromeda-uefi-fallback.log
/usr/libexec/andromeda-install-uefi-fallback /mnt/sysimage /mnt/sysroot interactive
%end
