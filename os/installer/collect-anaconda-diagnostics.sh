#!/usr/bin/bash
set -uo pipefail

readonly SERIAL_DEVICE="/dev/ttyS0"
readonly ESP_MOUNT="/mnt/sysimage/boot/efi"
readonly ESP_DIAGNOSTICS="${ESP_MOUNT}/EFI/Andromeda/diagnostics"

diagnostic_logs=(
    /tmp/anaconda.log
    /tmp/program.log
    /tmp/storage.log
    /tmp/packaging.log
    /tmp/syslog
    /tmp/andromeda-installer-preflight.log
    /tmp/andromeda-uefi-fallback.log
)

emit_file() {
    local diagnostic_file="$1"

    [[ -f "${diagnostic_file}" ]] || return 0
    printf '\n===== %s =====\n' "${diagnostic_file}" >"${SERIAL_DEVICE}"
    tail -n 2000 "${diagnostic_file}" >"${SERIAL_DEVICE}"
}

if [[ -c "${SERIAL_DEVICE}" ]]; then
    printf 'ANDROMEDA_INSTALLER_DIAGNOSTICS_START\n' >"${SERIAL_DEVICE}"
    for diagnostic_log in "${diagnostic_logs[@]}"; do
        emit_file "${diagnostic_log}"
    done
    printf '\n===== journalctl =====\n' >"${SERIAL_DEVICE}"
    journalctl --boot --no-pager --lines=2000 >"${SERIAL_DEVICE}" 2>&1 || true
    printf 'ANDROMEDA_INSTALLER_DIAGNOSTICS_END\n' >"${SERIAL_DEVICE}"
fi

if mountpoint --quiet "${ESP_MOUNT}"; then
    mkdir -p "${ESP_DIAGNOSTICS}"
    for diagnostic_log in "${diagnostic_logs[@]}"; do
        if [[ -f "${diagnostic_log}" ]]; then
            cp -f "${diagnostic_log}" "${ESP_DIAGNOSTICS}/"
        fi
    done
    journalctl --boot --no-pager \
        >"${ESP_DIAGNOSTICS}/journal.log" 2>&1 || true
    lsblk --output NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,PARTTYPE,UUID \
        >"${ESP_DIAGNOSTICS}/lsblk.txt" 2>&1 || true
    findmnt --real \
        >"${ESP_DIAGNOSTICS}/findmnt.txt" 2>&1 || true
    efibootmgr --verbose \
        >"${ESP_DIAGNOSTICS}/efibootmgr.txt" 2>&1 || true
    sync "${ESP_DIAGNOSTICS}"
fi
