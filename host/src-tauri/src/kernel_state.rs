use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;

use app_host_kernel::durable::{CommitOutcome, DurableKernelState, KernelStateStore};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::atomic_json::{
    load_json_document, persist_json_document, standard_writer, AtomicFileWriter, AtomicJsonError,
};
use crate::profiles::PROFILE_REGISTRY_LOCK_FILE;

const FORMAT_VERSION: u32 = 1;

/// A [`standard_writer`] that skips the shared post-rename directory sync.
///
/// Every other store treats a failed directory sync as a failed write. This
/// one cannot: by then the rename has landed, so the transition may well be
/// durable, and saying `NotCommitted` would be a false negative the kernel
/// would act on. It therefore performs that sync itself, via
/// [`FileKernelStateStore::sync_committed_file`], and reports the failure as
/// [`CommitOutcome::Indeterminate`].
struct DeferredDirSyncWriter(Arc<dyn AtomicFileWriter>);

impl AtomicFileWriter for DeferredDirSyncWriter {
    fn write_and_sync(&self, path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
        self.0.write_and_sync(path, bytes)
    }

    fn rename(&self, from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        self.0.rename(from, to)
    }

    fn remove_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        self.0.remove_file(path)
    }

    fn sync_parent_dir(&self, _path: &std::path::Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KernelStateDocument<T> {
    format_version: u32,
    state_sha256: String,
    state: T,
}

pub(crate) struct ProfileRegistryLock {
    _file: File,
}

impl ProfileRegistryLock {
    pub(crate) fn acquire(default_root: &std::path::Path) -> Result<Arc<Self>, String> {
        fs::create_dir_all(default_root)
            .map_err(|error| format!("create Kestral data directory failed: {error}"))?;
        let lock_path = default_root.join(PROFILE_REGISTRY_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("open profile registry lock failed: {error}"))?;
        file.try_lock_exclusive().map_err(|error| {
            format!(
                "Kestral profiles are locked by another host process ('{}'): {error}",
                lock_path.display()
            )
        })?;
        Ok(Arc::new(Self { _file: file }))
    }
}

pub(crate) struct ProfileLock {
    _file: File,
    _registry_lock: Option<Arc<ProfileRegistryLock>>,
}

impl ProfileLock {
    #[cfg(test)]
    pub(crate) fn acquire(kernel_state_path: PathBuf) -> Result<Arc<Self>, String> {
        Self::acquire_with_registry(kernel_state_path, None)
    }

    pub(crate) fn acquire_for_startup(
        kernel_state_path: PathBuf,
        registry_lock: Arc<ProfileRegistryLock>,
    ) -> Result<Arc<Self>, String> {
        Self::acquire_with_registry(kernel_state_path, Some(registry_lock))
    }

    fn acquire_with_registry(
        kernel_state_path: PathBuf,
        registry_lock: Option<Arc<ProfileRegistryLock>>,
    ) -> Result<Arc<Self>, String> {
        let parent = kernel_state_path
            .parent()
            .ok_or_else(|| "kernel state path has no parent directory".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create profile directory failed: {error}"))?;
        let lock_path = kernel_state_path.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("open profile lock failed: {error}"))?;
        file.try_lock_exclusive().map_err(|error| {
            format!(
                "profile is locked by another host process ('{}'): {error}",
                lock_path.display()
            )
        })?;
        Ok(Arc::new(Self {
            _file: file,
            _registry_lock: registry_lock,
        }))
    }
}

pub struct FileKernelStateStore {
    path: PathBuf,
    _profile_lock: Arc<ProfileLock>,
}

impl FileKernelStateStore {
    #[cfg(test)]
    pub fn open(path: PathBuf) -> Result<Arc<Self>, String> {
        let profile_lock = ProfileLock::acquire(path.clone())?;
        Ok(Self::open_with_lock(path, profile_lock))
    }

    pub(crate) fn open_with_lock(path: PathBuf, profile_lock: Arc<ProfileLock>) -> Arc<Self> {
        Arc::new(Self {
            path,
            _profile_lock: profile_lock,
        })
    }

    fn checksum(state: &DurableKernelState) -> Result<String, String> {
        Self::checksum_serializable(state)
    }

    fn checksum_serializable(state: &impl Serialize) -> Result<String, String> {
        let bytes = serde_json::to_vec(state)
            .map_err(|error| format!("serialize kernel state checksum input failed: {error}"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn checksum_raw(state: &RawValue) -> String {
        let mut compact = Vec::with_capacity(state.get().len());
        let mut in_string = false;
        let mut escaped = false;
        for byte in state.get().bytes() {
            if in_string {
                compact.push(byte);
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    in_string = false;
                }
            } else if byte == b'"' {
                in_string = true;
                compact.push(byte);
            } else if !byte.is_ascii_whitespace() {
                compact.push(byte);
            }
        }
        format!("{:x}", Sha256::digest(compact))
    }

    pub(crate) fn validate_persisted(path: &std::path::Path) -> Result<(), String> {
        Self::read_persisted(path).map(|_| ())
    }

    pub(crate) fn read_persisted(
        path: &std::path::Path,
    ) -> Result<Option<DurableKernelState>, String> {
        let Some(document) =
            load_json_document::<KernelStateDocument<Box<RawValue>>>(path, "durable kernel state")?
        else {
            return Ok(None);
        };
        if document.format_version != FORMAT_VERSION {
            return Err(format!(
                "unsupported durable kernel state version: {}",
                document.format_version
            ));
        }
        let actual = Self::checksum_raw(&document.state);
        if actual != document.state_sha256 {
            return Err(format!(
                "durable kernel state checksum mismatch; preserved '{}'",
                path.display()
            ));
        }
        serde_json::from_str::<DurableKernelState>(document.state.get())
            .map(Some)
            .map_err(|error| format!("parse durable kernel state failed: {error}"))
    }

    fn sync_committed_file(&self) -> Result<(), String> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("sync committed kernel state failed: {error}"))?;
        #[cfg(unix)]
        if let Some(parent) = self.path.parent() {
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("sync kernel state directory failed: {error}"))?;
        }
        Ok(())
    }
}

impl KernelStateStore for FileKernelStateStore {
    fn load(&self) -> Result<Option<DurableKernelState>, String> {
        let Some(document) = load_json_document::<KernelStateDocument<Box<RawValue>>>(
            &self.path,
            "durable kernel state",
        )?
        else {
            return Ok(None);
        };
        if document.format_version != FORMAT_VERSION {
            return Err(format!(
                "unsupported durable kernel state version: {}",
                document.format_version
            ));
        }
        // Checksum the raw bytes before Serde parses them, so a corrupted
        // file is caught before deserialization attempts to interpret it.
        let actual = Self::checksum_raw(&document.state);
        if actual != document.state_sha256 {
            return Err(format!(
                "durable kernel state checksum mismatch; preserved '{}'",
                self.path.display()
            ));
        }
        serde_json::from_str(document.state.get())
            .map(Some)
            .map_err(|error| format!("parse durable kernel state failed: {error}"))
    }

    fn commit(&self, state: &DurableKernelState) -> CommitOutcome {
        let document = KernelStateDocument {
            format_version: FORMAT_VERSION,
            state_sha256: match Self::checksum(state) {
                Ok(checksum) => checksum,
                Err(error) => return CommitOutcome::NotCommitted(error),
            },
            state: state.clone(),
        };
        match persist_json_document(
            &self.path,
            &document,
            "durable kernel state",
            &DeferredDirSyncWriter(standard_writer()),
        ) {
            Ok(()) => {}
            Err(AtomicJsonError::NotCommitted(error)) => {
                return CommitOutcome::NotCommitted(error);
            }
            Err(AtomicJsonError::Indeterminate(error)) => {
                return CommitOutcome::Indeterminate(error);
            }
        }
        match self.sync_committed_file() {
            Ok(()) => CommitOutcome::Committed,
            Err(error) => CommitOutcome::Indeterminate(error),
        }
    }
}

#[cfg(test)]
mod tests;
