bootc --source-imgref=containers-storage:localhost/andromeda:v1 --target-imgref=ghcr.io/oratis/andromeda:edge
selinux --enforcing

%post --nochroot --erroronfail --log=/tmp/andromeda-uefi-fallback.log
/usr/libexec/andromeda-install-uefi-fallback /mnt/sysimage /mnt/sysroot interactive
%end
