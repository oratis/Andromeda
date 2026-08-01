use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use andromeda_core::TaskId;
use fs4::fs_std::FileExt;
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

/// A cross-process-safe JSON task store with atomic replacement.
#[derive(Debug, Clone)]
pub struct FileTaskStore {
    root: PathBuf,
}

impl FileTaskStore {
    /// Opens or creates a task store.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the state directory cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
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
    /// # Errors
    ///
    /// Returns an I/O or parsing error if the state directory is unreadable or
    /// contains a malformed task record.
    pub fn list(&self) -> Result<Vec<TaskRecord>, StoreError> {
        let mut latest = BTreeMap::<TaskId, TaskRecord>::new();
        for path in self.record_paths()? {
            let record = Self::read_record(&path)?;
            latest
                .entry(record.plan.task_id)
                .and_modify(|current| {
                    if record.revision > current.revision {
                        current.clone_from(&record);
                    }
                })
                .or_insert(record);
        }
        Ok(latest.into_values().collect())
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
        Ok(fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect())
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
