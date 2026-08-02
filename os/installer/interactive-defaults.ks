# SECURITY: --target-imgref is a mutable, unsigned tag. Remote `bootc
# switch`/upgrade from this ref is NOT signature-enforced yet: no signing key
# exists, and enabling a strict fail-closed policy now would break the unsigned
# :edge update flow os-e2e exercises. A not-yet-enforced policy template and the
# cosign release/activation runbook live at os/signing/policy.json.example and
# docs/development/installable-preview.md ("bootc image signing runbook"). Until
# a key exists, a compromised registry or retagged image would be booted on the
# next update. See security-review.md finding #4.
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
