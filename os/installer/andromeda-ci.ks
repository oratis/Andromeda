cmdline
eula --agreed
firstboot --disable
keyboard us
lang en_US.UTF-8
network --bootproto=dhcp --device=link --activate
rootpw --lock
timezone UTC --utc

zerombr
clearpart --all --initlabel
reqpart
part / --fstype=ext4 --size=8192 --grow --label=andromeda-root

bootloader --timeout=1 --append="console=tty0 console=ttyS0,115200n8 andromeda.ci=1"
bootc --source-imgref=containers-storage:localhost/andromeda:v1 --target-imgref=ghcr.io/oratis/andromeda:edge --stateroot=andromeda

shutdown
