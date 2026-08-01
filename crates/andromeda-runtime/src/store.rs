use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use andromeda_core::TaskId;
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::TaskRecord;

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
        if self.latest_path(record.plan.task_id)?.is_some() {
            return Err(StoreError::AlreadyExists(record.plan.task_id));
        }
        let path = self.task_path(record.plan.task_id, record.revision);
        self.write_atomic(&path, record)?;
        FileExt::unlock(&lock)?;
        Ok(())
    }

    /// Loads one task.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] when the task is absent, or a parsing
    /// or I/O error for corrupted/unreadable state.
    pub fn get(&self, task_id: TaskId) -> Result<TaskRecord, StoreError> {
        let path = self
            .latest_path(task_id)?
            .ok_or(StoreError::NotFound(task_id))?;
        Self::read_record(&path)
    }

    /// Lists all tasks in deterministic identifier order.
    ///
    /// A record file that cannot be read or parsed does not fail the whole
    /// listing; it is skipped and reported in [`TaskListing::warnings`].
    ///
    /// # Errors
    ///
    /// Returns an I/O error only when the state directory itself is
    /// unreadable.
    pub fn list(&self) -> Result<TaskListing, StoreError> {
        let mut latest = BTreeMap::<TaskId, TaskRecord>::new();
        let mut warnings = Vec::new();
        for path in self.record_paths()? {
            match Self::read_record(&path) {
                Ok(record) => {
                    latest
                        .entry(record.plan.task_id)
                        .and_modify(|current| {
                            if record.revision > current.revision {
                                current.clone_from(&record);
                            }
                        })
                        .or_insert(record);
                }
                Err(error) => warnings.push(ListWarning {
                    path,
                    reason: error.to_string(),
                }),
            }
        }
        Ok(TaskListing {
            records: latest.into_values().collect(),
            warnings,
        })
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
        self.write_atomic(
            &self.task_path(record.plan.task_id, record.revision),
            record,
        )?;
        FileExt::unlock(&lock)?;
        Ok(())
    }

    fn task_path(&self, task_id: TaskId, revision: u64) -> PathBuf {
        self.root.join(format!("{task_id}.{revision:020}.json"))
    }

    fn latest_path(&self, task_id: TaskId) -> Result<Option<PathBuf>, StoreError> {
        let prefix = format!("{task_id}.");
        Ok(self
            .record_paths()?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            })
            .max())
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
}
