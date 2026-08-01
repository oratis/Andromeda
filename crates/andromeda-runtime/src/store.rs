use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use andromeda_core::TaskId;
use fs2::FileExt;
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
        let path = self.task_path(record.plan.task_id);
        if path.exists() {
            return Err(StoreError::AlreadyExists(record.plan.task_id));
        }
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
        let path = self.task_path(task_id);
        if !path.exists() {
            return Err(StoreError::NotFound(task_id));
        }
        Self::read_record(&path)
    }

    /// Lists all tasks in deterministic identifier order.
    ///
    /// # Errors
    ///
    /// Returns an I/O or parsing error if the state directory is unreadable or
    /// contains a malformed task record.
    pub fn list(&self) -> Result<Vec<TaskRecord>, StoreError> {
        let mut paths = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths.iter().map(|path| Self::read_record(path)).collect()
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
        self.write_atomic(&self.task_path(record.plan.task_id), record)?;
        FileExt::unlock(&lock)?;
        Ok(())
    }

    fn task_path(&self, task_id: TaskId) -> PathBuf {
        self.root.join(format!("{task_id}.json"))
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
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
