#!/usr/bin/env bash
set -euo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPOSITORY_ROOT
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
test -f "${OUTPUT_DIR}/andromeda-v2.tar"
test -f "${OVMF_CODE}"
test -f "${OVMF_VARS_TEMPLATE}"
update_sha256="$(sha256sum "${OUTPUT_DIR}/andromeda-v2.tar" | cut -d' ' -f1)"
readonly update_sha256

rm -rf "${DIAGNOSTICS_DIR}"
mkdir -p \
    "${DIAGNOSTICS_DIR}/installer" \
    "${DIAGNOSTICS_DIR}/host" \
    "${DIAGNOSTICS_DIR}/nvram" \
    "${DIAGNOSTICS_DIR}/root"
rm -f "${DISK_PATH}" "${INSTALL_LOG}" "${BOOT_LOG}" "${OVMF_VARS}"
qemu-img create -f qcow2 "${DISK_PATH}" 64G
cp "${OVMF_VARS_TEMPLATE}" "${OVMF_VARS}"

accel=tcg
cpu=max
if [[ -c /dev/kvm ]]; then
    accel=kvm
    cpu=host
elif [[ "${ANDROMEDA_ALLOW_TCG:-0}" != "1" ]]; then
    # Mirror test-gcp-nested.sh:41's `test -c /dev/kvm` precheck. Without KVM the
    # TCG software fallback runs ~10x slower: the 45m install timeout plus the
    # 2700s boot deadline can exceed the os-e2e job `timeout-minutes: 150`, so a
    # missing /dev/kvm would surface as a confusing timeout instead of a clear
    # failure. Fail fast; set ANDROMEDA_ALLOW_TCG=1 to force TCG for local debug.
    printf 'test-install.sh: /dev/kvm is unavailable.\n' >&2
    printf 'The TCG software fallback can exceed the os-e2e job timeout (150m) and surface as a confusing timeout rather than a clear failure.\n' >&2
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
if [[ "${partition_probe_ok}" -ne 1 ]]; then
    printf 'Timed out waiting for the ESP GUID and andromeda-root label on %s.\n' \
        "${nbd_device}" >&2
    exit 1
fi

# Success path: the partitions were found and must be mounted for the strict
# assertions below (partition_probe_ok=1 guarantees both were present).
test -n "${esp_mount}"
test -n "${root_mount}"

grep --text --extended-regexp \
    'ANDROMEDA_INSTALLER_(EFI_(START|OK)|KARGS_OK)' "${INSTALL_LOG}" \
    | tee "${OUTPUT_DIR}/install-post.log"
grep -q 'ANDROMEDA_INSTALLER_KARGS_OK mode=ci' "${INSTALL_LOG}"

test -f "${esp_mount}/EFI/fedora/shimx64.efi"
test -f "${esp_mount}/EFI/fedora/grubx64.efi"
test -f "${esp_mount}/EFI/BOOT/BOOTX64.EFI"
test -f "${esp_mount}/EFI/BOOT/grubx64.efi"

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
python3 -m http.server 8080 \
    --bind 127.0.0.1 \
    --directory "${OUTPUT_DIR}" \
    >"${OUTPUT_DIR}/update-server.log" 2>&1 &
http_pid="$!"

qemu-system-x86_64 \
    "${common_qemu[@]}" \
    -fw_cfg "name=opt/andromeda/update-sha256,string=${update_sha256}" \
    -boot order=c \
    -serial "file:${BOOT_LOG}" &
qemu_pid="$!"

deadline="$((SECONDS + 2700))"
while (( SECONDS < deadline )); do
    # Strip serial cursor-control residue (NUL, CR, ANSI CSI escapes) before the
    # trigger greps so a marker split mid-token by control chars can't cause a
    # false 2700s timeout. This mirrors the Python sequence validator below and
    # test-gcp-nested.sh:78-85's LC_ALL=C extraction on a normalized copy.
    boot_log_stripped="$(
        LC_ALL=C tr -d '\000\r' <"${BOOT_LOG}" 2>/dev/null \
            | LC_ALL=C sed -E 's#\x1b\[[0-?]*[ -/]*[@-~]##g'
    )" || boot_log_stripped=""
    if LC_ALL=C grep -qE 'ANDROMEDA_.*_FAILED' <<<"${boot_log_stripped}"; then
        printf 'Installed system emitted a failure marker.\n' >&2
        LC_ALL=C grep -aE 'ANDROMEDA_.*_(FAILED|OK)' <<<"${boot_log_stripped}" >&2
        exit 1
    fi
    if LC_ALL=C grep -q ANDROMEDA_E2E_OK <<<"${boot_log_stripped}"; then
        LC_ALL=C grep -aE \
            'ANDROMEDA_(SELINUX_LABELS|DAILY_DRIVER|FIRST_BOOT|UPDATE|ROLLBACK|E2E)' \
            <<<"${boot_log_stripped}"
        python3 - "${BOOT_LOG}" <<'PY'
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
    if grep -q 'Shell>' "${BOOT_LOG}" 2>/dev/null; then
        printf 'UEFI firmware could not find an installed bootloader.\n' >&2
        tail -200 "${BOOT_LOG}" >&2
        exit 1
    fi
    if ! kill -0 "${qemu_pid}" 2>/dev/null; then
        wait "${qemu_pid}"
        printf 'QEMU exited before the end-to-end marker was emitted.\n' >&2
        tail -200 "${BOOT_LOG}" >&2
        exit 1
    fi
    sleep 5
done

printf 'Timed out waiting for ANDROMEDA_E2E_OK.\n' >&2
tail -200 "${BOOT_LOG}" >&2
exit 1
