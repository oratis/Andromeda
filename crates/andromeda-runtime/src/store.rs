use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use andromeda_core::TaskId;
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::TaskRecord;

/// How many times a lock-free `get` re-resolves the latest revision when the
/// file it selected disappears underneath it. A concurrent writer's
/// compaction can delete the revision a reader just chose; re-resolving picks
/// up the newer revision. A small constant suffices because each writer
/// advances the revision at most once per lock acquisition.
const GET_MAX_ATTEMPTS: usize = 8;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("task {0} was not found")]
    NotFound(TaskId),
    #[error("task {task_id} revision conflict: expected {expected}, found {actual}")]
    RevisionConflict {
        task_id: TaskId,
        expected: u64,
        actual: u64,
    },
    #[error("task {task_id} record revision must be {expected}, found {actual}")]
    InvalidRecordRevision {
        task_id: TaskId,
        expected: u64,
        actual: u64,
    },
    #[error("task {0} already exists")]
    AlreadyExists(TaskId),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid task data: {0}")]
    Json(#[from] serde_json::Error),
}

/// A non-fatal problem encountered while listing task records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWarning {
    /// Path of the record file that could not be loaded.
    pub path: PathBuf,
    /// Human-readable reason the record was skipped.
    pub reason: String,
}

/// The result of listing the store: healthy records plus per-file warnings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskListing {
    pub records: Vec<TaskRecord>,
    /// Records that were skipped because they could not be read or parsed.
    pub warnings: Vec<ListWarning>,
}

/// A cross-process-safe JSON task store with atomic replacement.
#[derive(Debug, Clone)]
pub struct FileTaskStore {
    root: PathBuf,
}

impl FileTaskStore {
    /// Opens or creates a task store and removes orphaned temporary files
    /// left behind by writers that crashed before their atomic rename.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the state directory cannot be created,
    /// locked, or scanned for orphaned temporary files.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let store = Self { root };
        store.remove_orphan_temp_files()?;
        Ok(store)
    }

    /// Creates a record at revision zero.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AlreadyExists`] for a duplicate task or an I/O
    /// error when durable persistence fails.
    pub fn create(&self, record: &TaskRecord) -> Result<(), StoreError> {
        let lock = self.lock_exclusive()?;
        if record.revision != 0 {
            return Err(StoreError::InvalidRecordRevision {
                task_id: record.plan.task_id,
                expected: 0,
                actual: record.revision,
            });
        }
        if self.latest_revision(record.plan.task_id)?.is_some() {
            return Err(StoreError::AlreadyExists(record.plan.task_id));
        }
        self.commit(record)?;
        FileExt::unlock(&lock)?;
        Ok(())
    }

    /// Loads one task.
    ///
    /// This is lock-free. It reads the task's `latest` pointer (falling back
    /// to a directory scan) and then the corresponding revision file. If a
    /// concurrent writer's compaction deletes that file between the two steps,
    /// the read is retried against the freshly-resolved latest revision.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] when the task is absent, or a parsing
    /// or I/O error for corrupted/unreadable state.
    pub fn get(&self, task_id: TaskId) -> Result<TaskRecord, StoreError> {
        for _ in 0..GET_MAX_ATTEMPTS {
            let Some(revision) = self.latest_revision(task_id)? else {
                return Err(StoreError::NotFound(task_id));
            };
            match Self::read_record(&self.task_path(task_id, revision)) {
                Ok(record) => return Ok(record),
                // The revision we selected was compacted away by a concurrent
                // writer; re-resolve and try again.
                Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Err(StoreError::NotFound(task_id))
    }

    /// Lists all tasks in deterministic identifier order.
    ///
    /// Each task's latest revision is resolved the same way [`Self::get`]
    /// resolves it (pointer first, scan fallback), so listing and reading
    /// never disagree. A record file that cannot be read or parsed does not
    /// fail the whole listing; it is skipped and reported in
    /// [`TaskListing::warnings`].
    ///
    /// # Errors
    ///
    /// Returns an I/O error only when the state directory itself is
    /// unreadable.
    pub fn list(&self) -> Result<TaskListing, StoreError> {
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        for task_id in self.task_ids()? {
            let Some(revision) = self.latest_revision(task_id)? else {
                continue;
            };
            let path = self.task_path(task_id, revision);
            match Self::read_record(&path) {
                Ok(record) => records.push(record),
                // Vanished between enumeration and read (compaction race); a
                // newer revision now exists, so skip this stale path silently.
                Err(StoreError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => warnings.push(ListWarning {
                    path,
                    reason: error.to_string(),
                }),
            }
        }
        Ok(TaskListing { records, warnings })
    }

    /// Replaces a task only when its current revision matches `expected`.
    ///
    /// # Errors
    ///
    /// Returns a revision conflict for concurrent updates, not found for an
    /// absent task, or an I/O/parsing error.
    pub fn save(&self, record: &TaskRecord, expected: u64) -> Result<(), StoreError> {
        let lock = self.lock_exclusive()?;
        let current = self.get(record.plan.task_id)?;
        if current.revision != expected {
            return Err(StoreError::RevisionConflict {
                task_id: record.plan.task_id,
                expected,
                actual: current.revision,
            });
        }
        let next = expected.saturating_add(1);
        if record.revision != next {
            return Err(StoreError::InvalidRecordRevision {
                task_id: record.plan.task_id,
                expected: next,
                actual: record.revision,
            });
        }
        self.commit(record)?;
        FileExt::unlock(&lock)?;
        Ok(())
    }

    /// Durably publishes `record` as the task's latest revision: writes the
    /// revision file, advances the `latest` pointer, then compacts away every
    /// superseded revision file. Must be called under the exclusive lock.
    ///
    /// Ordering matters for crash-safety: the revision file is durable before
    /// the pointer moves, and compaction runs only after the pointer names the
    /// survivor — so a crash can leave an extra (harmless) revision file but
    /// never a pointer dangling to a missing file.
    fn commit(&self, record: &TaskRecord) -> Result<(), StoreError> {
        let task_id = record.plan.task_id;
        let revision = record.revision;
        self.write_atomic(&self.task_path(task_id, revision), record)?;
        self.write_pointer(task_id, revision)?;
        self.compact(task_id, revision)?;
        Ok(())
    }

    /// Deletes every revision file for `task_id` other than `keep`. Runs under
    /// the exclusive lock, so no concurrent writer competes; concurrent
    /// lock-free readers tolerate the deletion via [`Self::get`]'s retry.
    fn compact(&self, task_id: TaskId, keep: u64) -> Result<(), StoreError> {
        let prefix = format!("{task_id}.");
        for path in self.record_paths()? {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with(&prefix) {
                continue;
            }
            if revision_of(name, &prefix).is_some_and(|revision| revision != keep) {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    fn task_path(&self, task_id: TaskId, revision: u64) -> PathBuf {
        self.root.join(format!("{task_id}.{revision:020}.json"))
    }

    fn pointer_path(&self, task_id: TaskId) -> PathBuf {
        self.root.join(format!("{task_id}.latest"))
    }

    /// Atomically records `revision` as the task's current latest in a small
    /// pointer file, so the common-case lookup is a single read instead of a
    /// full-directory scan.
    fn write_pointer(&self, task_id: TaskId, revision: u64) -> Result<(), StoreError> {
        let temporary = self.root.join(format!(".{}.tmp", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(revision.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, self.pointer_path(task_id))?;
        #[cfg(unix)]
        sync_directory(&self.root)?;
        Ok(())
    }

    /// Resolves the task's latest revision number. Prefers the `latest`
    /// pointer (validated to still name an existing file) and falls back to a
    /// directory scan when the pointer is missing, unparsable, or stale — for
    /// example for a store written before pointers existed, or immediately
    /// after a crash between the revision write and the pointer write.
    fn latest_revision(&self, task_id: TaskId) -> Result<Option<u64>, StoreError> {
        if let Some(revision) = self.read_pointer(task_id)? {
            if self.task_path(task_id, revision).exists() {
                return Ok(Some(revision));
            }
        }
        self.scan_latest_revision(task_id)
    }

    fn read_pointer(&self, task_id: TaskId) -> Result<Option<u64>, StoreError> {
        match fs::read_to_string(self.pointer_path(task_id)) {
            Ok(text) => Ok(text.trim().parse::<u64>().ok()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn scan_latest_revision(&self, task_id: TaskId) -> Result<Option<u64>, StoreError> {
        let prefix = format!("{task_id}.");
        let mut latest = None;
        for path in self.record_paths()? {
            if let Some(revision) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| revision_of(name, &prefix))
            {
                latest = latest.max(Some(revision));
            }
        }
        Ok(latest)
    }

    /// Enumerates every task id with at least one revision file or pointer.
    fn task_ids(&self) -> Result<BTreeSet<TaskId>, StoreError> {
        let mut ids = BTreeSet::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if let Some(id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(task_id_of_name)
            {
                ids.insert(id);
            }
        }
        Ok(ids)
    }

    fn record_paths(&self) -> Result<Vec<PathBuf>, StoreError> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    /// Deletes stale `.{uuid}.tmp` files under the exclusive store lock.
    ///
    /// Writers only produce temporary files while holding the same lock, so
    /// any temporary file observed here belongs to a crashed writer.
    fn remove_orphan_temp_files(&self) -> Result<(), StoreError> {
        let lock = self.lock_exclusive()?;
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            let is_orphan_temp = path.extension().is_some_and(|extension| extension == "tmp")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'));
            if is_orphan_temp {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        FileExt::unlock(&lock)?;
        Ok(())
    }

    fn lock_exclusive(&self) -> Result<File, StoreError> {
        let lock_path = self.root.join(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;
        Ok(lock)
    }

    fn read_record(path: &Path) -> Result<TaskRecord, StoreError> {
        let reader = BufReader::new(File::open(path)?);
        serde_json::from_reader(reader).map_err(StoreError::from)
    }

    fn write_atomic(&self, destination: &Path, record: &TaskRecord) -> Result<(), StoreError> {
        let temporary = self.root.join(format!(".{}.tmp", Uuid::new_v4()));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, record)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&temporary, destination)?;
        #[cfg(unix)]
        sync_directory(&self.root)?;
        Ok(())
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

/// Extracts the task id from a store file name. Handles both revision files
/// (`{task_id}.{revision}.json`) and pointer files (`{task_id}.latest`);
/// returns `None` for the lock file, temporary files, or any other name.
fn task_id_of_name(name: &str) -> Option<TaskId> {
    let stem = name
        .strip_suffix(".json")
        .or_else(|| name.strip_suffix(".latest"))?;
    let id = stem.split_once('.').map_or(stem, |(id, _revision)| id);
    TaskId::from_str(id).ok()
}

/// Parses the revision number out of a `{prefix}{revision}.json` file name,
/// where `prefix` is `"{task_id}."`.
fn revision_of(name: &str, prefix: &str) -> Option<u64> {
    name.strip_prefix(prefix)?
        .strip_suffix(".json")?
        .parse::<u64>()
        .ok()
}

#[cfg(test)]
mod tests {
    use andromeda_core::{ActionPlan, Intent, TaskState};
    use tempfile::TempDir;

    use super::*;

    fn record() -> TaskRecord {
        TaskRecord {
            plan: ActionPlan::new(Intent::new("store test", "test"), Vec::new()),
            state: TaskState::Draft,
            revision: 0,
            capabilities: Vec::new(),
            events: Vec::new(),
            outcomes: Vec::new(),
        }
    }

    #[test]
    fn open_removes_orphan_temp_files() {
        let temp = TempDir::new().expect("tempdir");
        let orphan = temp.path().join(format!(".{}.tmp", Uuid::new_v4()));
        fs::write(&orphan, b"partial write").expect("orphan");

        let store = FileTaskStore::open(temp.path()).expect("store");
        assert!(fs::metadata(&orphan).is_err(), "orphan must be removed");

        let created = record();
        store.create(&created).expect("create");
        assert_eq!(store.get(created.plan.task_id).expect("get"), created);
    }

    #[test]
    fn list_skips_corrupt_records_with_warnings() {
        let temp = TempDir::new().expect("tempdir");
        let store = FileTaskStore::open(temp.path()).expect("store");
        let created = record();
        store.create(&created).expect("create");
        let corrupt = temp
            .path()
            .join(format!("{}.{:020}.json", TaskId::new(), 0));
        fs::write(&corrupt, b"{ not json").expect("corrupt record");

        let listing = store.list().expect("list");
        assert_eq!(listing.records, vec![created]);
        assert_eq!(listing.warnings.len(), 1);
        assert_eq!(listing.warnings[0].path, corrupt);
    }

    #[test]
    fn duplicate_create_reports_already_exists() {
        let temp = TempDir::new().expect("tempdir");
        let store = FileTaskStore::open(temp.path()).expect("store");
        let created = record();
        store.create(&created).expect("create");
        assert!(matches!(
            store.create(&created),
            Err(StoreError::AlreadyExists(id)) if id == created.plan.task_id
        ));
    }

    fn json_file_count(dir: &Path) -> usize {
        fs::read_dir(dir)
            .expect("state directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
            })
            .count()
    }

    #[test]
    fn compaction_reclaims_superseded_revisions() {
        let temp = TempDir::new().expect("tempdir");
        let store = FileTaskStore::open(temp.path()).expect("store");
        let mut current = record();
        store.create(&current).expect("create");

        // Many revisions of the same task.
        for expected in 0..25u64 {
            current.revision = expected + 1;
            store.save(&current, expected).expect("save");
            // Reads keep returning the newest revision throughout.
            assert_eq!(
                store.get(current.plan.task_id).expect("get").revision,
                expected + 1
            );
        }

        // Only the surviving revision file remains (old ones were reclaimed),
        // and the latest record is still readable.
        assert_eq!(json_file_count(temp.path()), 1);
        let loaded = store.get(current.plan.task_id).expect("get");
        assert_eq!(loaded.revision, 25);
    }

    #[test]
    fn get_falls_back_to_scan_without_a_pointer() {
        // Simulates a store written before pointers existed: a lone revision
        // file with no `.latest` pointer must still resolve.
        let temp = TempDir::new().expect("tempdir");
        let store = FileTaskStore::open(temp.path()).expect("store");
        let created = record();
        let raw = temp
            .path()
            .join(format!("{}.{:020}.json", created.plan.task_id, 0));
        let mut writer = serde_json::to_string_pretty(&created).expect("serialize");
        writer.push('\n');
        fs::write(&raw, writer).expect("write revision file");

        let loaded = store.get(created.plan.task_id).expect("get");
        assert_eq!(loaded, created);
    }
}
