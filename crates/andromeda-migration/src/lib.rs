//! Read-only migration inventory for a Windows, macOS, or Linux user profile.
//!
//! The scanner deliberately does less than an importer: it reads known user-data
//! directories, hashes regular files, records application candidates, and reports
//! everything it skipped. It never follows symlinks, opens credential stores, writes
//! to the source profile, or claims an item was migrated.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

/// Current migration-manifest wire version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Conservative default that bounds filesystem work on unexpectedly large profiles.
pub const DEFAULT_MAX_FILES: usize = 100_000;

/// Bound on all directory entries, including directories and skipped objects.
pub const DEFAULT_MAX_ENTRIES: usize = 250_000;

/// Prevents a maliciously deep source tree from exhausting the call stack.
pub const MAX_DIRECTORY_DEPTH: usize = 64;

/// Size of each read while hashing a file.
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

/// The platform family the source profile came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourcePlatform {
    Windows,
    Macos,
    Linux,
    Other,
}

impl SourcePlatform {
    /// Platform of the process running the scan.
    #[must_use]
    pub const fn host() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// User-visible data group used for routing and progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
}

impl DataCategory {
    const fn directory_names(self) -> &'static [&'static str] {
        match self {
            Self::Desktop => &["Desktop"],
            Self::Documents => &["Documents"],
            Self::Downloads => &["Downloads"],
            Self::Pictures => &["Pictures"],
            Self::Music => &["Music"],
            Self::Videos => &["Videos", "Movies"],
        }
    }

    const fn all() -> [Self; 6] {
        [
            Self::Desktop,
            Self::Documents,
            Self::Downloads,
            Self::Pictures,
            Self::Music,
            Self::Videos,
        ]
    }
}

/// One regular file that can be considered by a later importer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// Path relative to the source profile. Absolute paths and user names are not emitted.
    pub relative_path: PathBuf,
    pub category: DataCategory,
    pub size_bytes: u64,
    pub modified_at: Option<DateTime<Utc>>,
    pub sha256: String,
}

/// Installation candidate found without reading application data or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationCandidate {
    pub display_name: String,
    pub relative_path: PathBuf,
    pub source_kind: ApplicationSourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationSourceKind {
    MacosBundle,
    WindowsUserProgram,
    LinuxDesktopEntry,
    ProfileApplication,
}

/// A source entry that was observed but could not safely become a file record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkippedEntry {
    pub relative_path: PathBuf,
    pub reason: SkipReason,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Symlink,
    UnsupportedFileType,
    PermissionDenied,
    ReadError,
    ChangedDuringScan,
    LimitReached,
}

/// Aggregate values make space estimates possible without re-reading every entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSummary {
    pub file_count: usize,
    pub total_bytes: u64,
    pub application_count: usize,
    pub skipped_count: usize,
    pub truncated: bool,
    pub files_by_category: BTreeMap<DataCategory, usize>,
}

/// A reproducible, read-only inventory. It is not evidence that anything was imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationManifest {
    pub schema_version: u32,
    pub manifest_id: Uuid,
    pub collected_at: DateTime<Utc>,
    pub source_platform: SourcePlatform,
    pub hash_algorithm: String,
    pub files: Vec<FileEntry>,
    pub applications: Vec<ApplicationCandidate>,
    pub skipped: Vec<SkippedEntry>,
    pub summary: ScanSummary,
}

/// Bounded scan settings. The profile is always treated as read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    pub profile_root: PathBuf,
    pub source_platform: SourcePlatform,
    pub max_files: usize,
    pub max_entries: usize,
}

impl ScanOptions {
    #[must_use]
    pub fn new(profile_root: impl Into<PathBuf>) -> Self {
        Self {
            profile_root: profile_root.into(),
            source_platform: SourcePlatform::host(),
            max_files: DEFAULT_MAX_FILES,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("source profile {} is not a real directory (symlink roots are refused)", .0.display())]
    InvalidProfile(PathBuf),
    #[error("max_files must be greater than zero")]
    ZeroFileLimit,
    #[error("max_entries must be greater than zero")]
    ZeroEntryLimit,
    #[error("could not inspect source profile {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Scans known user-data and application-inventory locations without mutating them.
///
/// # Errors
/// Returns [`ScanError`] when the profile root itself is unusable or either traversal bound is
/// zero.
/// Errors below the root are retained as [`SkippedEntry`] records so a migration report cannot
/// silently hide data it failed to inventory.
pub fn scan_profile(options: &ScanOptions) -> Result<MigrationManifest, ScanError> {
    if options.max_files == 0 {
        return Err(ScanError::ZeroFileLimit);
    }
    if options.max_entries == 0 {
        return Err(ScanError::ZeroEntryLimit);
    }
    let root_metadata =
        std::fs::symlink_metadata(&options.profile_root).map_err(|source| ScanError::Io {
            path: options.profile_root.clone(),
            source,
        })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(ScanError::InvalidProfile(options.profile_root.clone()));
    }

    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut truncated = false;
    let mut visited_entries = 0usize;
    for category in DataCategory::all() {
        for name in category.directory_names() {
            let category_root = options.profile_root.join(name);
            let Ok(metadata) = std::fs::symlink_metadata(&category_root) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                skipped.push(SkippedEntry {
                    relative_path: relative_or_name(&options.profile_root, &category_root),
                    reason: SkipReason::Symlink,
                    detail: "top-level data category is a symlink and was not followed".to_owned(),
                });
                continue;
            }
            if !metadata.is_dir() {
                continue;
            }
            walk_data_directory(
                &options.profile_root,
                &category_root,
                category,
                0,
                options.max_files,
                options.max_entries,
                &mut visited_entries,
                &mut files,
                &mut skipped,
                &mut truncated,
            );
            if truncated {
                break;
            }
        }
        if truncated {
            break;
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut applications = Vec::new();
    if !truncated {
        discover_applications(
            &options.profile_root,
            options.max_entries,
            &mut visited_entries,
            &mut applications,
            &mut skipped,
            &mut truncated,
        );
    }
    applications.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    applications.dedup_by(|left, right| left.relative_path == right.relative_path);
    skipped.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut files_by_category = BTreeMap::new();
    let mut total_bytes = 0u64;
    for file in &files {
        *files_by_category.entry(file.category).or_insert(0) += 1;
        total_bytes = total_bytes.saturating_add(file.size_bytes);
    }

    Ok(MigrationManifest {
        schema_version: CURRENT_SCHEMA_VERSION,
        manifest_id: Uuid::new_v4(),
        collected_at: Utc::now(),
        source_platform: options.source_platform,
        hash_algorithm: "sha256".to_owned(),
        summary: ScanSummary {
            file_count: files.len(),
            total_bytes,
            application_count: applications.len(),
            skipped_count: skipped.len(),
            truncated,
            files_by_category,
        },
        files,
        applications,
        skipped,
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_data_directory(
    profile_root: &Path,
    directory: &Path,
    category: DataCategory,
    depth: usize,
    max_files: usize,
    max_entries: usize,
    visited_entries: &mut usize,
    files: &mut Vec<FileEntry>,
    skipped: &mut Vec<SkippedEntry>,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }
    let remaining_entries = max_entries.saturating_sub(*visited_entries);
    let Some((paths, read_errors, directory_was_truncated)) =
        read_sorted_paths(profile_root, directory, remaining_entries, skipped)
    else {
        return;
    };
    *visited_entries = visited_entries.saturating_add(read_errors);

    for path in paths {
        if *visited_entries >= max_entries {
            *truncated = true;
            skipped.push(SkippedEntry {
                relative_path: relative_or_name(profile_root, &path),
                reason: SkipReason::LimitReached,
                detail: format!("scan stopped at the configured {max_entries}-entry bound"),
            });
            return;
        }
        *visited_entries += 1;
        if files.len() >= max_files {
            mark_limit(
                profile_root,
                &path,
                format!("scan stopped at the configured {max_files}-file bound"),
                skipped,
                truncated,
            );
            return;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                skipped.push(skipped_io(profile_root, &path, &error));
                continue;
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            skipped.push(SkippedEntry {
                relative_path: relative_or_name(profile_root, &path),
                reason: SkipReason::Symlink,
                detail: "symlinks are recorded but never followed".to_owned(),
            });
        } else if file_type.is_dir() {
            if depth >= MAX_DIRECTORY_DEPTH {
                *truncated = true;
                skipped.push(SkippedEntry {
                    relative_path: relative_or_name(profile_root, &path),
                    reason: SkipReason::LimitReached,
                    detail: format!(
                        "directory was not descended beyond the {MAX_DIRECTORY_DEPTH}-level depth bound"
                    ),
                });
                continue;
            }
            walk_data_directory(
                profile_root,
                &path,
                category,
                depth + 1,
                max_files,
                max_entries,
                visited_entries,
                files,
                skipped,
                truncated,
            );
        } else if file_type.is_file() {
            record_regular_file(profile_root, &path, &metadata, category, files, skipped);
        } else {
            skipped.push(SkippedEntry {
                relative_path: relative_or_name(profile_root, &path),
                reason: SkipReason::UnsupportedFileType,
                detail: "only regular files and directories are inventoried".to_owned(),
            });
        }
        if *truncated {
            return;
        }
    }
    if directory_was_truncated {
        mark_limit(
            profile_root,
            directory,
            format!("scan stopped at the configured {max_entries}-entry bound"),
            skipped,
            truncated,
        );
    }
}

fn record_regular_file(
    profile_root: &Path,
    path: &Path,
    metadata: &std::fs::Metadata,
    category: DataCategory,
    files: &mut Vec<FileEntry>,
    skipped: &mut Vec<SkippedEntry>,
) {
    match hash_stable_file(path, metadata) {
        Ok(sha256) => files.push(FileEntry {
            relative_path: relative_or_name(profile_root, path),
            category,
            size_bytes: metadata.len(),
            modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            sha256,
        }),
        Err(FileReadFailure::Io(error)) => {
            skipped.push(skipped_io(profile_root, path, &error));
        }
        Err(FileReadFailure::Changed) => skipped.push(SkippedEntry {
            relative_path: relative_or_name(profile_root, path),
            reason: SkipReason::ChangedDuringScan,
            detail: "file metadata changed while its checksum was being computed".to_owned(),
        }),
    }
}

fn mark_limit(
    profile_root: &Path,
    path: &Path,
    detail: String,
    skipped: &mut Vec<SkippedEntry>,
    truncated: &mut bool,
) {
    *truncated = true;
    skipped.push(SkippedEntry {
        relative_path: relative_or_name(profile_root, path),
        reason: SkipReason::LimitReached,
        detail,
    });
}

fn read_sorted_paths(
    profile_root: &Path,
    directory: &Path,
    remaining_entries: usize,
    skipped: &mut Vec<SkippedEntry>,
) -> Option<(Vec<PathBuf>, usize, bool)> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            skipped.push(skipped_io(profile_root, directory, &error));
            return None;
        }
    };
    let mut paths = Vec::new();
    let mut read_errors = 0usize;
    let mut truncated = false;
    for (observed, entry) in entries.enumerate() {
        if observed >= remaining_entries {
            truncated = true;
            break;
        }
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => {
                read_errors += 1;
                skipped.push(SkippedEntry {
                    relative_path: relative_or_name(profile_root, directory),
                    reason: io_reason(&error),
                    detail: error.to_string(),
                });
            }
        }
    }
    paths.sort();
    Some((paths, read_errors, truncated))
}

enum FileReadFailure {
    Io(io::Error),
    Changed,
}

fn hash_stable_file(path: &Path, before: &std::fs::Metadata) -> Result<String, FileReadFailure> {
    let mut file = File::open(path).map_err(FileReadFailure::Io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(FileReadFailure::Io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let after = std::fs::metadata(path).map_err(FileReadFailure::Io)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(FileReadFailure::Changed);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn discover_applications(
    profile_root: &Path,
    max_entries: usize,
    visited_entries: &mut usize,
    applications: &mut Vec<ApplicationCandidate>,
    skipped: &mut Vec<SkippedEntry>,
    truncated: &mut bool,
) {
    discover_directory_apps(
        profile_root,
        &profile_root.join("Applications"),
        ApplicationSourceKind::ProfileApplication,
        false,
        max_entries,
        visited_entries,
        applications,
        skipped,
        truncated,
    );
    discover_directory_apps(
        profile_root,
        &profile_root.join("AppData/Local/Programs"),
        ApplicationSourceKind::WindowsUserProgram,
        false,
        max_entries,
        visited_entries,
        applications,
        skipped,
        truncated,
    );
    discover_directory_apps(
        profile_root,
        &profile_root.join(".local/share/applications"),
        ApplicationSourceKind::LinuxDesktopEntry,
        true,
        max_entries,
        visited_entries,
        applications,
        skipped,
        truncated,
    );
}

#[allow(clippy::too_many_arguments)]
fn discover_directory_apps(
    profile_root: &Path,
    directory: &Path,
    default_kind: ApplicationSourceKind,
    desktop_entries_only: bool,
    max_entries: usize,
    visited_entries: &mut usize,
    applications: &mut Vec<ApplicationCandidate>,
    skipped: &mut Vec<SkippedEntry>,
    truncated: &mut bool,
) {
    if *truncated {
        return;
    }
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            skipped.push(skipped_io(profile_root, directory, &error));
            return;
        }
    };
    if metadata.file_type().is_symlink() {
        skipped.push(SkippedEntry {
            relative_path: relative_or_name(profile_root, directory),
            reason: SkipReason::Symlink,
            detail: "application inventory directory symlink was not followed".to_owned(),
        });
        return;
    }
    if !metadata.is_dir() {
        skipped.push(SkippedEntry {
            relative_path: relative_or_name(profile_root, directory),
            reason: SkipReason::UnsupportedFileType,
            detail: "application inventory root is not a directory".to_owned(),
        });
        return;
    }

    let remaining_entries = max_entries.saturating_sub(*visited_entries);
    let Some((paths, read_errors, directory_was_truncated)) =
        read_sorted_paths(profile_root, directory, remaining_entries, skipped)
    else {
        return;
    };
    *visited_entries = visited_entries.saturating_add(read_errors);
    for path in paths {
        if !visit_application_entry(
            profile_root,
            &path,
            max_entries,
            visited_entries,
            skipped,
            truncated,
        ) {
            return;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                skipped.push(skipped_io(profile_root, &path, &error));
                continue;
            }
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            skipped.push(SkippedEntry {
                relative_path: relative_or_name(profile_root, &path),
                reason: SkipReason::Symlink,
                detail: "application candidate symlink was not followed".to_owned(),
            });
            continue;
        }
        if desktop_entries_only {
            if !file_type.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("desktop")
            {
                continue;
            }
            applications.push(ApplicationCandidate {
                display_name: display_name(&path, ".desktop"),
                relative_path: relative_or_name(profile_root, &path),
                source_kind: ApplicationSourceKind::LinuxDesktopEntry,
            });
            continue;
        }
        if !file_type.is_dir() && !file_type.is_file() {
            continue;
        }
        let is_bundle = path.extension().and_then(|extension| extension.to_str()) == Some("app");
        applications.push(ApplicationCandidate {
            display_name: display_name(&path, if is_bundle { ".app" } else { "" }),
            relative_path: relative_or_name(profile_root, &path),
            source_kind: if is_bundle {
                ApplicationSourceKind::MacosBundle
            } else {
                default_kind
            },
        });
    }
    if directory_was_truncated {
        mark_limit(
            profile_root,
            directory,
            format!("scan stopped at the configured {max_entries}-entry bound"),
            skipped,
            truncated,
        );
    }
}

fn visit_application_entry(
    profile_root: &Path,
    path: &Path,
    max_entries: usize,
    visited_entries: &mut usize,
    skipped: &mut Vec<SkippedEntry>,
    truncated: &mut bool,
) -> bool {
    if *visited_entries >= max_entries {
        *truncated = true;
        skipped.push(SkippedEntry {
            relative_path: relative_or_name(profile_root, path),
            reason: SkipReason::LimitReached,
            detail: format!("scan stopped at the configured {max_entries}-entry bound"),
        });
        return false;
    }
    *visited_entries += 1;
    true
}

fn display_name(path: &Path, suffix: &str) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown application");
    name.strip_suffix(suffix).unwrap_or(name).to_owned()
}

fn relative_or_name(profile_root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(profile_root).map_or_else(
        |_| path.file_name().map_or_else(PathBuf::new, PathBuf::from),
        Path::to_path_buf,
    )
}

fn io_reason(error: &io::Error) -> SkipReason {
    if error.kind() == io::ErrorKind::PermissionDenied {
        SkipReason::PermissionDenied
    } else {
        SkipReason::ReadError
    }
}

fn skipped_io(profile_root: &Path, path: &Path, error: &io::Error) -> SkippedEntry {
    SkippedEntry {
        relative_path: relative_or_name(profile_root, path),
        reason: io_reason(error),
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn scan_hashes_known_user_data_and_keeps_paths_relative() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        fs::create_dir(temp.path().join("Documents")).expect("documents");
        fs::write(temp.path().join("Documents/report.docx"), b"office-data").expect("file");
        fs::create_dir(temp.path().join("Pictures")).expect("pictures");
        fs::write(temp.path().join("Pictures/photo.jpg"), b"pixels").expect("file");

        let manifest = scan_profile(&ScanOptions::new(temp.path())).expect("scan");
        assert_eq!(manifest.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(manifest.summary.file_count, 2);
        assert_eq!(manifest.summary.total_bytes, 17);
        assert!(!manifest.summary.truncated);
        assert_eq!(
            manifest.files[0].relative_path,
            PathBuf::from("Documents/report.docx")
        );
        assert_eq!(
            manifest.files[0].sha256,
            "2768ca93550a46fe3442ce4d59d50c272b9f8f693e9a3b8c1ab134db5180d7a1"
        );
        assert!(
            manifest
                .files
                .iter()
                .all(|file| !file.relative_path.is_absolute())
        );
    }

    #[test]
    fn scan_is_bounded_and_reports_what_it_did_not_inventory() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        fs::create_dir(temp.path().join("Desktop")).expect("desktop");
        fs::write(temp.path().join("Desktop/a"), b"a").expect("a");
        fs::write(temp.path().join("Desktop/b"), b"b").expect("b");
        let mut options = ScanOptions::new(temp.path());
        options.max_files = 1;

        let manifest = scan_profile(&options).expect("scan");
        assert_eq!(manifest.summary.file_count, 1);
        assert!(manifest.summary.truncated);
        assert!(
            manifest
                .skipped
                .iter()
                .any(|entry| entry.reason == SkipReason::LimitReached)
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_records_symlinks_without_following_them() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::TempDir::new().expect("tempdir");
        fs::create_dir(temp.path().join("Documents")).expect("documents");
        fs::write(temp.path().join("outside"), b"secret").expect("outside");
        symlink(
            temp.path().join("outside"),
            temp.path().join("Documents/link"),
        )
        .expect("symlink");

        let manifest = scan_profile(&ScanOptions::new(temp.path())).expect("scan");
        assert!(manifest.files.is_empty());
        assert_eq!(manifest.skipped[0].reason, SkipReason::Symlink);
    }

    #[test]
    fn application_candidates_do_not_read_application_data() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("Applications/Pages.app")).expect("app");
        fs::create_dir_all(temp.path().join("AppData/Local/Programs/Example")).expect("program");
        fs::create_dir_all(temp.path().join(".local/share/applications")).expect("desktop dir");
        fs::write(
            temp.path().join(".local/share/applications/editor.desktop"),
            b"this content is deliberately not parsed",
        )
        .expect("desktop entry");

        let manifest = scan_profile(&ScanOptions::new(temp.path())).expect("scan");
        let names: Vec<&str> = manifest
            .applications
            .iter()
            .map(|application| application.display_name.as_str())
            .collect();
        assert_eq!(names, ["editor", "Example", "Pages"]);
        assert_eq!(manifest.summary.application_count, 3);
    }

    #[test]
    fn application_inventory_shares_the_total_entry_bound() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        for name in ["One.app", "Two.app", "Three.app"] {
            fs::create_dir_all(temp.path().join("Applications").join(name)).expect("app");
        }
        let mut options = ScanOptions::new(temp.path());
        options.max_entries = 2;

        let manifest = scan_profile(&options).expect("scan");
        assert_eq!(manifest.summary.application_count, 2);
        assert!(manifest.summary.truncated);
        assert!(
            manifest
                .skipped
                .iter()
                .any(|entry| entry.reason == SkipReason::LimitReached)
        );
    }

    #[cfg(unix)]
    #[test]
    fn profile_root_symlink_is_refused() {
        use std::os::unix::fs::symlink;

        let source = tempfile::TempDir::new().expect("source");
        let parent = tempfile::TempDir::new().expect("parent");
        let link = parent.path().join("profile");
        symlink(source.path(), &link).expect("symlink");
        assert!(matches!(
            scan_profile(&ScanOptions::new(&link)),
            Err(ScanError::InvalidProfile(path)) if path == link
        ));
    }

    #[test]
    fn manifest_json_round_trip_is_stable() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let manifest = scan_profile(&ScanOptions::new(temp.path())).expect("scan");
        let json = serde_json::to_string(&manifest).expect("serialize");
        let parsed: MigrationManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn zero_bound_is_rejected_instead_of_silently_scanning_nothing() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let mut options = ScanOptions::new(temp.path());
        options.max_files = 0;
        assert!(matches!(
            scan_profile(&options),
            Err(ScanError::ZeroFileLimit)
        ));

        options.max_files = DEFAULT_MAX_FILES;
        options.max_entries = 0;
        assert!(matches!(
            scan_profile(&options),
            Err(ScanError::ZeroEntryLimit)
        ));
    }

    #[test]
    fn total_entry_bound_limits_directory_only_trees() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("Documents/a/b/c")).expect("tree");
        let mut options = ScanOptions::new(temp.path());
        options.max_entries = 2;

        let manifest = scan_profile(&options).expect("scan");
        assert!(manifest.summary.truncated);
        assert!(
            manifest
                .skipped
                .iter()
                .any(|entry| entry.detail.contains("2-entry bound"))
        );
    }

    #[test]
    fn schema_and_rust_contract_share_the_same_version_and_enums() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../schemas/migration-manifest.schema.json"
        ))
        .expect("schema JSON");
        assert_eq!(
            schema["properties"]["schema_version"]["const"],
            CURRENT_SCHEMA_VERSION
        );
        assert_eq!(
            schema["properties"]["source_platform"]["enum"],
            serde_json::json!(["windows", "macos", "linux", "other"])
        );
        assert_eq!(
            schema["$defs"]["category"]["enum"],
            serde_json::json!([
                "desktop",
                "documents",
                "downloads",
                "pictures",
                "music",
                "videos"
            ])
        );
    }
}
