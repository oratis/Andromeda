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

# The bootc payload runs under Anaconda 44's Payloads DBus module; its stderr
# lands in that module's journal rather than reliably in program.log. Capture it
# first-class so a "bootc ... exited with status 1" is root-causable.
readonly PAYLOADS_UNIT_GLOB='org.fedoraproject.Anaconda.Modules.Payloads*'

emit_file() {
    local diagnostic_file="$1"
    # max_lines <= 0 emits the file in full. program.log is emitted in full
    # because the failing bootc payload phase can be well beyond the last lines;
    # the CI serial sink is a host file, so full emission is cheap.
    local max_lines="${2:-2000}"

    [[ -f "${diagnostic_file}" ]] || return 0
    printf '\n===== %s =====\n' "${diagnostic_file}" >"${SERIAL_DEVICE}"
    if (( max_lines <= 0 )); then
        cat "${diagnostic_file}" >"${SERIAL_DEVICE}"
    else
        tail -n "${max_lines}" "${diagnostic_file}" >"${SERIAL_DEVICE}"
    fi
}

if [[ -c "${SERIAL_DEVICE}" ]]; then
    printf 'ANDROMEDA_INSTALLER_DIAGNOSTICS_START\n' >"${SERIAL_DEVICE}"
    for diagnostic_log in "${diagnostic_logs[@]}"; do
        if [[ "${diagnostic_log}" == /tmp/program.log ]]; then
            emit_file "${diagnostic_log}" 0
        else
            emit_file "${diagnostic_log}"
        fi
    done
    printf '\n===== journalctl payloads =====\n' >"${SERIAL_DEVICE}"
    journalctl -u "${PAYLOADS_UNIT_GLOB}" --boot --no-pager \
        >"${SERIAL_DEVICE}" 2>&1 || true
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
    journalctl -u "${PAYLOADS_UNIT_GLOB}" --boot --no-pager \
        >"${ESP_DIAGNOSTICS}/anaconda-payloads-journal.log" 2>&1 || true
    lsblk --output NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,PARTTYPE,UUID \
        >"${ESP_DIAGNOSTICS}/lsblk.txt" 2>&1 || true
    findmnt --real \
        >"${ESP_DIAGNOSTICS}/findmnt.txt" 2>&1 || true
    df --human-readable --print-type \
        >"${ESP_DIAGNOSTICS}/df.txt" 2>&1 || true
    df --human-readable --inodes \
        >"${ESP_DIAGNOSTICS}/df-inodes.txt" 2>&1 || true
    efibootmgr --verbose \
        >"${ESP_DIAGNOSTICS}/efibootmgr.txt" 2>&1 || true
    sync "${ESP_DIAGNOSTICS}"
fi
