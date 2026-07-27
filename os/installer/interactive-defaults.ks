bootc --source-imgref=containers-storage:localhost/andromeda:v1 --target-imgref=ghcr.io/oratis/andromeda:edge

%post --nochroot --erroronfail --log=/tmp/andromeda-uefi-fallback.log
/usr/libexec/andromeda-install-uefi-fallback /mnt/sysimage /mnt/sysroot
%end
