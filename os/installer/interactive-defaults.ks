bootc --source-imgref=containers-storage:localhost/andromeda:v1 --target-imgref=ghcr.io/oratis/andromeda:edge
selinux --enforcing

%pre --erroronfail --log=/tmp/andromeda-installer-preflight.log
/usr/libexec/andromeda-installer-preflight interactive
%end

%post --nochroot --erroronfail --log=/tmp/andromeda-uefi-fallback.log
/usr/libexec/andromeda-install-uefi-fallback /mnt/sysimage /mnt/sysroot interactive
%end
