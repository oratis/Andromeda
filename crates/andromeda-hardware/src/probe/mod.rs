use std::io;

use thiserror::Error;

use crate::HardwareReport;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod other;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("hardware probe I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("hardware probe returned invalid data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("platform command failed: {0}")]
    Command(String),
}

/// Collects a privacy-conscious report for the current host.
///
/// # Errors
///
/// Returns an error when the platform's basic hardware interfaces cannot be
/// read. Optional capabilities are reported as unknown instead of failing the
/// whole probe.
pub fn probe_host() -> Result<HardwareReport, ProbeError> {
    #[cfg(target_os = "linux")]
    return linux::probe();
    #[cfg(target_os = "macos")]
    return macos::probe();
    #[cfg(target_os = "windows")]
    return windows::probe();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return other::probe();
}

pub(crate) fn logical_cores() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}
