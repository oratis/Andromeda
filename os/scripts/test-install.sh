#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
ANDROMEDA_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly ANDROMEDA_SCRIPT_DIR
# Resolved from BASH_SOURCE so it works from CI, from `sudo os/scripts/...`, and
# from the GCP path where the repository is untarred into ~/andromeda.
# shellcheck source=os/scripts/lib/assert.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/assert.sh"
# shellcheck source=os/scripts/lib/markers.sh
# shellcheck disable=SC1091
. "${ANDROMEDA_SCRIPT_DIR}/lib/markers.sh"
readonly OUTPUT_DIR="${1:-${REPOSITORY_ROOT}/output}"
readonly ISO_PATH="${OUTPUT_DIR}/Andromeda-Developer-Preview-x86_64-ci.iso"

if [[ "${EUID}" -ne 0 ]]; then
    printf 'test-install.sh needs root for modprobe, qemu-nbd, and mount; run it with sudo.\n' >&2
    exit 1
fi
readonly DISK_PATH="${OUTPUT_DIR}/andromeda-test.qcow2"
readonly INSTALL_LOG="${OUTPUT_DIR}/install-serial.log"
readonly BOOT_LOG="${OUTPUT_DIR}/boot-serial.log"
readonly DIAGNOSTICS_DIR="${OUTPUT_DIR}/diagnostics"
# Every marker assertion below greps these NUL/CR/ANSI-stripped copies, never
# the raw serial logs (lib/markers.sh explains why). They live under
# diagnostics/host so the os-e2e evidence artifact carries exactly the bytes the
# gates actually saw.
readonly INSTALL_LOG_NORMALIZED="${DIAGNOSTICS_DIR}/host/install-serial.normalized.log"
readonly BOOT_LOG_NORMALIZED="${DIAGNOSTICS_DIR}/host/boot-serial.normalized.log"
readonly OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
readonly OVMF_VARS_TEMPLATE="${OVMF_VARS_TEMPLATE:-/usr/share/OVMF/OVMF_VARS_4M.fd}"
readonly OVMF_VARS="${OUTPUT_DIR}/OVMF_VARS.fd"

http_pid=""
qemu_pid=""
nbd_device=""
esp_mount=""
root_mount=""

# shellcheck disable=SC2317,SC2329 # Invoked indirectly by the EXIT trap.
cleanup() {
    if [[ -n "${qemu_pid}" ]]; then
        kill "${qemu_pid}" 2>/dev/null || true
        wait "${qemu_pid}" 2>/dev/null || true
    fi
    if [[ -n "${http_pid}" ]]; then
        kill "${http_pid}" 2>/dev/null || true
        wait "${http_pid}" 2>/dev/null || true
    fi
    if [[ -n "${root_mount}" ]] && mountpoint --quiet "${root_mount}"; then
        umount "${root_mount}" || true
    fi
    if [[ -n "${esp_mount}" ]] && mountpoint --quiet "${esp_mount}"; then
        umount "${esp_mount}" || true
    fi
    if [[ -n "${nbd_device}" ]]; then
        qemu-nbd --disconnect "${nbd_device}" 2>/dev/null || true
    fi
    if [[ -n "${root_mount}" ]]; then
        rmdir "${root_mount}" 2>/dev/null || true
    fi
    if [[ -n "${esp_mount}" ]]; then
        rmdir "${esp_mount}" 2>/dev/null || true
    fi
    if [[ -f "${OVMF_VARS}" && -d "${DIAGNOSTICS_DIR}/nvram" ]]; then
        cp -f "${OVMF_VARS}" \
            "${DIAGNOSTICS_DIR}/nvram/OVMF_VARS.final.fd" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if [[ ! -f "${ISO_PATH}" ]]; then
    printf 'Missing CI installer ISO: %s\n' "${ISO_PATH}" >&2
    printf 'Build it with INSTALLER_DEFAULT=1 os/scripts/build-iso.sh; this lifecycle test needs the destructive CI entry as the GRUB default.\n' >&2
    exit 1
fi
require_file "${OUTPUT_DIR}/andromeda-v2.tar" \
    'the rev2 update payload (andromeda-v2.tar) built by build-iso.sh is required; the guest fetches it over slirp and verifies it against the fw_cfg SHA-256'
require_file "${OVMF_CODE}" \
    'the OVMF firmware image is required for the UEFI boot path; install the ovmf package or set OVMF_CODE'
require_file "${OVMF_VARS_TEMPLATE}" \
    'the OVMF variable-store template is required to give the guest fresh NVRAM; install the ovmf package or set OVMF_VARS_TEMPLATE'
update_sha256="$(sha256sum "${OUTPUT_DIR}/andromeda-v2.tar" | cut -d' ' -f1)"
readonly update_sha256

rm -rf "${DIAGNOSTICS_DIR}"
mkdir -p \
    "${DIAGNOSTICS_DIR}/installer" \
    "${DIAGNOSTICS_DIR}/host" \
    "${DIAGNOSTICS_DIR}/nvram" \
    "${DIAGNOSTICS_DIR}/root"
# Record the host toolchain BEFORE the long install, so the evidence bundle can
# say what produced it. Firmware and emulator versions silently change UEFI and
# device behaviour, and UEFI boot is precisely what is under test -- an artifact
# that cannot name its own OVMF/QEMU is not reproducible evidence. The GCP path
# already writes host-environment.txt; this is the CI equivalent. Written early
# and best-effort so a missing optional tool never fails the run.
{
    printf 'collected           %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'uname               %s\n' "$(uname -a)"
    printf 'qemu-system-x86_64  %s\n' \
        "$(qemu-system-x86_64 --version 2>/dev/null | head -n 1)"
    printf 'qemu-img            %s\n' \
        "$(qemu-img --version 2>/dev/null | head -n 1)"
    printf 'qemu-nbd            %s\n' \
        "$(qemu-nbd --version 2>/dev/null | head -n 1)"
    for optional in podman skopeo shellcheck; do
        printf '%-19s %s\n' "${optional}" \
            "$(command -v "${optional}" >/dev/null &&
                "${optional}" --version 2>/dev/null | head -n 1 ||
                printf 'absent')"
    done
    printf 'OVMF_CODE           %s\n' "${OVMF_CODE}"
    printf 'OVMF_CODE sha256    %s\n' \
        "$(sha256sum "${OVMF_CODE}" 2>/dev/null | cut -d' ' -f1)"
    printf 'OVMF_VARS_TEMPLATE  %s\n' "${OVMF_VARS_TEMPLATE}"
    printf 'OVMF_VARS sha256    %s\n' \
        "$(sha256sum "${OVMF_VARS_TEMPLATE}" 2>/dev/null | cut -d' ' -f1)"
    printf 'installer ISO       %s\n' "${ISO_PATH}"
    printf 'ISO sha256          %s\n' \
        "$(sha256sum "${ISO_PATH}" 2>/dev/null | cut -d' ' -f1)"
    printf 'update payload sha256 %s\n' "${update_sha256}"
} >"${DIAGNOSTICS_DIR}/host/versions.txt" 2>&1 || true

rm -f "${DISK_PATH}" "${INSTALL_LOG}" "${BOOT_LOG}" "${OVMF_VARS}"
qemu-img create -f qcow2 "${DISK_PATH}" 64G
cp "${OVMF_VARS_TEMPLATE}" "${OVMF_VARS}"

accel=tcg
cpu=max
if [[ -c /dev/kvm ]]; then
    accel=kvm
    cpu=host
elif [[ "${ANDROMEDA_ALLOW_TCG:-0}" != "1" ]]; then
    # Mirror test-gcp-nested.sh's `test -c /dev/kvm` precheck. Without KVM the
    # TCG software fallback runs ~10x slower: the 45m install timeout plus the
    # 2700s boot deadline can exceed the os-e2e lifecycle job
    # `timeout-minutes: 140`, so a missing /dev/kvm would surface as a
    # confusing timeout instead of a clear failure. Fail fast; set
    # ANDROMEDA_ALLOW_TCG=1 to force TCG for local debug.
    printf 'test-install.sh: /dev/kvm is unavailable.\n' >&2
    printf 'The TCG software fallback can exceed the os-e2e lifecycle job timeout (140m) and surface as a confusing timeout rather than a clear failure.\n' >&2
    printf 'Set ANDROMEDA_ALLOW_TCG=1 to force the slow TCG path for local debugging.\n' >&2
    exit 1
else
    printf 'WARNING: /dev/kvm is unavailable; ANDROMEDA_ALLOW_TCG=1 is set, using slow TCG emulation. Expect a very slow run that may exceed CI budgets.\n' >&2
fi

common_qemu=(
    -machine "q35,accel=${accel}"
    -cpu "${cpu}"
    -smp 4
    -m 8192
    -nodefaults
    -no-user-config
    -device virtio-vga
    -device virtio-rng-pci
    -audiodev "driver=none,id=audio0"
    -device ich9-intel-hda
    -device "hda-duplex,audiodev=audio0"
    -device "virtio-net-pci,netdev=net0"
    -netdev "user,id=net0"
    -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}"
    -drive "if=pflash,format=raw,file=${OVMF_VARS}"
    -drive "if=virtio,format=qcow2,file=${DISK_PATH}"
    -display none
    -monitor none
)

install_status=0
timeout 45m qemu-system-x86_64 \
    "${common_qemu[@]}" \
    -drive "if=ide,media=cdrom,readonly=on,file=${ISO_PATH}" \
    -serial "file:${INSTALL_LOG}" || install_status="$?"

printf '%s\n' "${install_status}" \
    >"${DIAGNOSTICS_DIR}/host/install-exit-status.txt"
qemu-img info "${DISK_PATH}" \
    >"${DIAGNOSTICS_DIR}/host/qemu-img-info.txt" 2>&1 || true
image_check_status=0
qemu-img check "${DISK_PATH}" \
    >"${DIAGNOSTICS_DIR}/host/qemu-img-check.txt" 2>&1 \
    || image_check_status="$?"
printf '%s\n' "${image_check_status}" \
    >"${DIAGNOSTICS_DIR}/host/qemu-img-check-status.txt"
cp -f "${OVMF_VARS}" \
    "${DIAGNOSTICS_DIR}/nvram/OVMF_VARS.after-install.fd"

modprobe nbd max_part=16
for candidate in /dev/nbd{0..15}; do
    if [[ ! -e "/sys/block/${candidate##*/}/pid" ]]; then
        nbd_device="${candidate}"
        break
    fi
done
require 'a free /dev/nbdN device is needed to inspect the installed disk; all of /dev/nbd0../dev/nbd15 are already connected (another test-install.sh run on this host?)' \
    test -n "${nbd_device}"

qemu-nbd --read-only --connect="${nbd_device}" "${DISK_PATH}"
blockdev --rereadpt "${nbd_device}"
partprobe "${nbd_device}"
udevadm trigger --settle "${nbd_device}"
udevadm settle

partition_probe_ok=0
partition_probe_deadline="$((SECONDS + 30))"
while (( SECONDS < partition_probe_deadline )); do
    blkid "${nbd_device}"* >/dev/null 2>&1 || true
    if lsblk -prno NAME,PARTTYPE "${nbd_device}" \
        | grep -qi 'c12a7328-f81f-11d2-ba4b-00a0c93ec93b' \
        && lsblk -prno LABEL "${nbd_device}" \
        | grep -qx 'andromeda-root'; then
        partition_probe_ok=1
        break
    fi
    sleep 1
done

lsblk --paths --output \
    NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,PARTTYPE,UUID "${nbd_device}" \
    | tee "${DIAGNOSTICS_DIR}/host/lsblk.txt"
blkid "${nbd_device}"* \
    >"${DIAGNOSTICS_DIR}/host/blkid.txt" 2>&1 || true
sfdisk --dump "${nbd_device}" \
    >"${DIAGNOSTICS_DIR}/host/partition-table.sfdisk" 2>&1 || true

# Harvest the disk-side evidence UNCONDITIONALLY, before any strict exit check.
# On a bootc install failure the anaconda %onerror hook has already written its
# diagnostics (program.log, the Payloads-module journal that carries bootc's own
# stderr, etc.) to EFI/Andromeda/diagnostics on the ESP; collecting them here --
# rather than after the exit-code gates below -- is what lets the failure path
# upload the same disk-side evidence as the success path, turning an opaque
# "bootc ... exited with status 1" into a root-causable file. Everything in this
# block is best-effort: a missing/unmountable partition must not mask the real
# failure cause reported by the exit-code checks that follow.
esp_partition="$(
    lsblk -prno NAME,PARTTYPE "${nbd_device}" \
        | awk 'tolower($2) == "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" {
            print $1
        }'
)"
root_partition="$(
    lsblk -prno NAME,LABEL "${nbd_device}" \
        | awk '$2 == "andromeda-root" { print $1 }'
)"

if [[ "$(wc -w <<<"${esp_partition}")" -eq 1 ]]; then
    esp_mount="$(mktemp -d "${OUTPUT_DIR}/esp.XXXXXX")"
    if mount -o ro "${esp_partition}" "${esp_mount}"; then
        find "${esp_mount}" -maxdepth 4 -type f -printf '%P\n' \
            | sort | tee "${OUTPUT_DIR}/esp-tree.txt" || true
        if [[ -d "${esp_mount}/EFI/Andromeda/diagnostics" ]]; then
            cp -a "${esp_mount}/EFI/Andromeda/diagnostics/." \
                "${DIAGNOSTICS_DIR}/installer/" || true
        fi
    else
        printf 'WARNING: could not mount ESP %s for diagnostics.\n' \
            "${esp_partition}" >&2
        rmdir "${esp_mount}" 2>/dev/null || true
        esp_mount=""
    fi
fi

if [[ "$(wc -w <<<"${root_partition}")" -eq 1 ]]; then
    root_mount="$(mktemp -d "${OUTPUT_DIR}/root.XXXXXX")"
    if mount -o ro,noload "${root_partition}" "${root_mount}"; then
        find "${root_mount}" -type f \
            \( -path '*/var/log/anaconda/*' \
            -o -name anaconda-ks.cfg \
            -o -name original-ks.cfg \
            -o -name andromeda-uefi-fallback.log \) \
            -print | sort >"${DIAGNOSTICS_DIR}/root/log-paths.txt" || true
        : >"${DIAGNOSTICS_DIR}/root/log-excerpts.txt"
        while IFS= read -r installed_log; do
            printf '\n===== %s =====\n' \
                "${installed_log#"${root_mount}"}" \
                >>"${DIAGNOSTICS_DIR}/root/log-excerpts.txt"
            tail -n 1000 "${installed_log}" \
                >>"${DIAGNOSTICS_DIR}/root/log-excerpts.txt" 2>&1 || true
        done <"${DIAGNOSTICS_DIR}/root/log-paths.txt"
    else
        printf 'WARNING: could not mount root %s for diagnostics.\n' \
            "${root_partition}" >&2
        rmdir "${root_mount}" 2>/dev/null || true
        root_mount=""
    fi
fi

# Disk-side evidence is now captured; report the primary failure cause. On any
# exit here the EXIT trap unmounts the ESP/root harvested above.
if [[ "${install_status}" -ne 0 ]]; then
    printf 'Installer exited with status %s.\n' "${install_status}" >&2
    tail -100 "${INSTALL_LOG}" >&2 || true
    exit "${install_status}"
fi
if [[ "${image_check_status}" -ne 0 ]]; then
    printf 'Installed disk image failed qemu-img check with status %s.\n' \
        "${image_check_status}" >&2
    exit "${image_check_status}"
fi

# ---------------------------------------------------------------------------
# Installer failure gate: a QEMU exit status of 0 does NOT mean the install
# succeeded.
#
# Anaconda's %onerror hook (os/installer/andromeda-ci.ks:33-35) collects
# diagnostics and returns; the kickstart's own `shutdown` (:27) then still runs,
# so the VM powers off cleanly and `install_status` above is 0. run 30686771448
# failed exactly this way: the ESP carried EFI/Andromeda/diagnostics/*, which is
# written ONLY by %onerror, yet every exit-status gate passed and the script
# fell through to the marker grep further below. That grep matched nothing, and
# `set -o pipefail` plus `set -e` turned it into a bare `exit code 1` with not
# one word of explanation in the CI log. See docs/reviews/e2e-pipeline-review.md
# P0 #1.
#
# This gate MUST run before the partition-probe gate below: a `%pre
# --erroronfail` preflight failure aborts Anaconda before partitioning, so the
# ESP GUID and andromeda-root label never appear, partition_probe_ok stays 0,
# and the probe gate would otherwise mask the real cause behind a misleading
# "Timed out waiting for the ESP GUID..." message -- leaving the
# PREFLIGHT_FAILED branch here unreachable.
#
# So: check the installer's own failure markers explicitly, and positively
# require its success marker instead of only asserting that some greps matched.
# Both checks read the normalized log -- a marker split by serial cursor-control
# residue must not be able to hide an installer failure.
normalize_serial_log "${INSTALL_LOG}" "${INSTALL_LOG_NORMALIZED}"

install_failure_reason=""
if LC_ALL=C grep --text --quiet 'ANDROMEDA_INSTALLER_PREFLIGHT_FAILED' \
    "${INSTALL_LOG_NORMALIZED}"; then
    install_failure_reason='the installer preflight (%pre) rejected this host or the embedded payload'
elif LC_ALL=C grep --text --quiet 'ANDROMEDA_INSTALLER_DIAGNOSTICS_START' \
    "${INSTALL_LOG_NORMALIZED}"; then
    install_failure_reason='Anaconda ran its %onerror hook, so the installation aborted'
elif [[ -n "$(find "${DIAGNOSTICS_DIR}/installer" -type f 2>/dev/null)" ]]; then
    # Third, serial-independent trigger: EFI/Andromeda/diagnostics on the ESP
    # is written ONLY by the %onerror hook, so harvested installer diagnostics
    # prove the install aborted even when neither marker reached the serial
    # log (e.g. /dev/ttyS0 was not a character device in the installer
    # environment, so the markers were never emitted there).
    install_failure_reason='the ESP carries EFI/Andromeda/diagnostics, which only the %onerror hook writes, so the installation aborted (no serial failure marker was captured; /dev/ttyS0 may not have been usable in the installer environment)'
fi

if [[ -n "${install_failure_reason}" ]]; then
    {
        printf '\n'
        printf '================================================================\n'
        printf 'INSTALLER FAILED -- this is NOT a test-harness failure.\n'
        printf '================================================================\n'
        printf '  reason: %s.\n' "${install_failure_reason}"
        printf '  QEMU exited %s, but the exit code is not an install result:\n' \
            "${install_status}"
        printf '  the kickstart still runs its shutdown directive after %%onerror,\n'
        printf '  so a failed install also powers the VM off cleanly.\n'
        printf '\n'
        printf '  Installer diagnostics harvested from the ESP:\n'
    } >&2
    # The directory is pre-created empty, so test for CONTENT, not existence.
    installer_diagnostics="$(
        find "${DIAGNOSTICS_DIR}/installer" -type f 2>/dev/null | sort
    )" || installer_diagnostics=""
    if [[ -n "${installer_diagnostics}" ]]; then
        printf '%s\n' "${installer_diagnostics}" | sed 's/^/    /' >&2
        printf '  (anaconda.log, program.log and anaconda-payloads-journal.log\n' >&2
        printf '   carry the bootc payload stderr; start there.)\n' >&2
    else
        printf '    none -- the ESP held no EFI/Andromeda/diagnostics, or it\n' >&2
        printf '    could not be mounted (see the WARNINGs above).\n' >&2
    fi
    printf '\n' >&2
    dump_log_region "${INSTALL_LOG_NORMALIZED}" \
        'ANDROMEDA_INSTALLER_(PREFLIGHT_FAILED|DIAGNOSTICS_START)' 60 120
    printf '\n  serial markers seen during the install:\n' >&2
    LC_ALL=C grep --text --extended-regexp \
        'ANDROMEDA_INSTALLER_[A-Z_]*' "${INSTALL_LOG_NORMALIZED}" \
        | sed 's/^/    /' >&2 || true
    printf '\n  full normalized install log: %s\n\n' \
        "${INSTALL_LOG_NORMALIZED}" >&2
    exit 1
fi

require_marker "${INSTALL_LOG_NORMALIZED}" 'ANDROMEDA_INSTALLER_EFI_OK' \
    'the UEFI fallback stage never completed: os/installer/install-uefi-fallback.sh did not emit ANDROMEDA_INSTALLER_EFI_OK, so the kickstart %post that installs EFI/BOOT/BOOTX64.EFI and creates the Andromeda NVRAM entry did not finish'

if [[ "${partition_probe_ok}" -ne 1 ]]; then
    printf 'Timed out waiting for the ESP GUID and andromeda-root label on %s.\n' \
        "${nbd_device}" >&2
    exit 1
fi

# Success path: the partitions were found and must be mounted for the strict
# assertions below (partition_probe_ok=1 guarantees both were present).
require 'the ESP was found on the installed disk but could not be mounted, so the EFI binary assertions below cannot run (see the earlier WARNING for the mount error)' \
    test -n "${esp_mount}"
require 'the andromeda-root partition was found but could not be mounted read-only, so the installed-system assertions below cannot run (see the earlier WARNING for the mount error)' \
    test -n "${root_mount}"

grep --text --extended-regexp \
    'ANDROMEDA_INSTALLER_(EFI_(START|OK)|KARGS_OK)' "${INSTALL_LOG_NORMALIZED}" \
    | tee "${OUTPUT_DIR}/install-post.log"
require_marker "${INSTALL_LOG_NORMALIZED}" 'ANDROMEDA_INSTALLER_KARGS_OK mode=ci' \
    'the installer did not confirm the CI kernel arguments; install-uefi-fallback.sh emits this only after selinux=1/enforcing=1 are merged and the root=/boot= arguments survive in the target boot entries'

require_file "${esp_mount}/EFI/fedora/shimx64.efi" \
    'the ESP is missing the shim vendor binary, so Secure Boot chain loading cannot start'
require_file "${esp_mount}/EFI/fedora/grubx64.efi" \
    'the ESP is missing the GRUB vendor binary that shim chain loads'
require_file "${esp_mount}/EFI/BOOT/BOOTX64.EFI" \
    'the ESP is missing the removable-media fallback loader; firmware that ignores NVRAM boot entries would drop to the UEFI Shell'
require_file "${esp_mount}/EFI/BOOT/grubx64.efi" \
    'the ESP is missing the GRUB copy next to the removable-media fallback loader'

umount "${root_mount}"
umount "${esp_mount}"
qemu-nbd --disconnect "${nbd_device}"
nbd_device=""
rmdir "${root_mount}" "${esp_mount}"
root_mount=""
esp_mount=""

strings --encoding=l "${OVMF_VARS}" \
    >"${DIAGNOSTICS_DIR}/nvram/strings-after-install.txt"
grep Andromeda "${DIAGNOSTICS_DIR}/nvram/strings-after-install.txt" \
    | tee "${OUTPUT_DIR}/ovmf-vars.txt"
grep -F '\EFI\fedora\shimx64.efi' \
    "${DIAGNOSTICS_DIR}/nvram/strings-after-install.txt" \
    | tee -a "${OUTPUT_DIR}/ovmf-vars.txt"

# The guest reaches the host over slirp via 10.0.2.2, which forwards to the
# host loopback; never expose the output directory on real interfaces.
#
# Port 0 lets the kernel assign a free port instead of hardcoding 8080. A fixed
# port is fine on an ephemeral GitHub runner but collides on any host that runs
# two lifecycle tests at once or reuses one machine (the GCP path keeps a single
# instance alive across runs), and the failure mode -- "Address already in use"
# racing an unrelated server -- is not obviously about ports. The chosen port is
# handed to the guest over the same fw_cfg channel that already carries the
# update SHA-256.
#
# `-u` is load-bearing: without it Python buffers the "Serving HTTP on ... port
# N" banner when stdout is a file, so the port cannot be read back here.
python3 -u -m http.server 0 \
    --bind 127.0.0.1 \
    --directory "${OUTPUT_DIR}" \
    >"${OUTPUT_DIR}/update-server.log" 2>&1 &
http_pid="$!"

update_port=""
for _ in $(seq 1 50); do
    update_port="$(
        sed -n 's/.*port \([0-9][0-9]*\).*/\1/p' \
            "${OUTPUT_DIR}/update-server.log" 2>/dev/null | head -n 1
    )"
    [[ -n "${update_port}" ]] && break
    sleep 0.1
done
require "the update server announced its listening port within 5s" \
    test -n "${update_port}"
readonly update_port
printf 'update server listening on 127.0.0.1:%s (guest reaches it at 10.0.2.2:%s)\n' \
    "${update_port}" "${update_port}"

qemu-system-x86_64 \
    "${common_qemu[@]}" \
    -fw_cfg "name=opt/andromeda/update-sha256,string=${update_sha256}" \
    -fw_cfg "name=opt/andromeda/update-port,string=${update_port}" \
    -boot order=c \
    -serial "file:${BOOT_LOG}" &
qemu_pid="$!"

# TIMEOUT BUDGET INVARIANT (docs/reviews/e2e-pipeline-review.md P0 #4,
# re-derived for the P1 #7 job split).
# The budgets must nest, or a genuinely slow run surfaces as an unattributable
# cancellation instead of a named timeout:
#
#   unit timeout            andromeda-ci-verify.service TimeoutStartSec = 12 min
#     <  per-boot budget    12 min unit + ~2 min firmware/kernel/sddm/plasma
#                           = 14 min
#   3 x per-boot budget     3 boots (first-boot, update, rollback) = 42 min
#     <  boot deadline      2700 s = 45 min   <-- the deadline below
#   sum of stage budgets    install 45 min + boot 45 min
#                           + matrix 3 x 10 min (a HOST-side cap that
#                             deliberately sits below the 12 min verify unit
#                             timeout; test-hardware-matrix.sh explains why)
#                           + free-disk/apt/download/upload ~15 min = ~135 min
#     <  job timeout        os-e2e.yml `lifecycle` job timeout-minutes: 140
#
# The image build no longer appears in this sum: since the split it runs in
# the separate `build` job, whose own budget derivation lives in os-e2e.yml.
#
# Changing any budget here means re-checking the two sites above and below it.
deadline="$((SECONDS + 2700))"
while (( SECONDS < deadline )); do
    # Grep the NUL/CR/ANSI-stripped copy, never the raw serial log: a marker
    # split mid-token by cursor-control residue would otherwise cause a false
    # 2700 s timeout. normalize_serial_log lives in lib/markers.sh and is shared
    # with test-hardware-matrix.sh so this fix cannot again land in only one of
    # the two pollers.
    normalize_serial_log "${BOOT_LOG}" "${BOOT_LOG_NORMALIZED}"
    if LC_ALL=C grep --text -qE 'ANDROMEDA_.*_FAILED' \
        "${BOOT_LOG_NORMALIZED}"; then
        printf 'Installed system emitted a failure marker.\n' >&2
        LC_ALL=C grep -aE 'ANDROMEDA_.*_(FAILED|OK)' \
            "${BOOT_LOG_NORMALIZED}" >&2
        exit 1
    fi
    if LC_ALL=C grep --text -q ANDROMEDA_E2E_OK "${BOOT_LOG_NORMALIZED}"; then
        LC_ALL=C grep -aE \
            'ANDROMEDA_(SELINUX_LABELS|DAILY_DRIVER|FIRST_BOOT|UPDATE|ROLLBACK|E2E)' \
            "${BOOT_LOG_NORMALIZED}"
        # The validator is fed the already-normalized log; it repeats the
        # stripping because the transform is idempotent and it must stay correct
        # if it is ever pointed at a raw log.
        python3 - "${BOOT_LOG_NORMALIZED}" <<'PY'
import pathlib
import re
import sys

log = pathlib.Path(sys.argv[1]).read_bytes().replace(b"\0", b"").decode(errors="replace")
log = log.replace("\r", "")
log = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", log)
markers = [
    "ANDROMEDA_SELINUX_LABELS_OK",
    "ANDROMEDA_DAILY_DRIVER_OK phase=first-boot revision=1",
    "ANDROMEDA_FIRST_BOOT_OK revision=1",
    "ANDROMEDA_UPDATE_STAGED_OK revision=2",
    "ANDROMEDA_DAILY_DRIVER_OK phase=updating revision=2",
    "ANDROMEDA_UPDATE_BOOT_OK revision=2",
    "ANDROMEDA_ROLLBACK_STAGED_OK revision=1",
    "ANDROMEDA_DAILY_DRIVER_OK phase=rolling-back revision=1",
    "ANDROMEDA_ROLLBACK_BOOT_OK revision=1",
    "ANDROMEDA_E2E_OK",
]
failures = re.findall(r"ANDROMEDA_[A-Z0-9_]*FAILED[^\n]*", log)
if failures:
    raise SystemExit(f"Failure marker present: {failures}")
counts = [log.count(marker) for marker in markers]
if counts != [1] * len(markers):
    raise SystemExit(f"Daily-driver markers must occur exactly once: {counts}")
positions = [log.find(marker) for marker in markers]
if positions != sorted(positions):
    raise SystemExit(f"Daily-driver marker sequence invalid: {positions}")
PY
        exit 0
    fi
    # The UEFI Shell prompt is written by firmware that also emits cursor
    # positioning, so this detector reads the normalized copy as well.
    if LC_ALL=C grep --text -q 'Shell>' "${BOOT_LOG_NORMALIZED}" 2>/dev/null; then
        printf 'UEFI firmware could not find an installed bootloader.\n' >&2
        tail -200 "${BOOT_LOG_NORMALIZED}" >&2
        exit 1
    fi
    if ! kill -0 "${qemu_pid}" 2>/dev/null; then
        wait "${qemu_pid}"
        printf 'QEMU exited before the end-to-end marker was emitted.\n' >&2
        tail -200 "${BOOT_LOG_NORMALIZED}" >&2
        exit 1
    fi
    sleep 5
done

printf 'Timed out waiting for ANDROMEDA_E2E_OK after %s s.\n' 2700 >&2
printf 'Budget check: 3 guest boots x (TimeoutStartSec 12min + ~2min boot) = 42min must fit in this deadline; see the invariant comment above.\n' >&2
tail -200 "${BOOT_LOG_NORMALIZED}" >&2
exit 1
