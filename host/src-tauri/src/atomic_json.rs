use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use uuid::Uuid;

pub(crate) trait AtomicFileWriter: Send + Sync {
    fn write_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Make a completed rename itself durable.
    ///
    /// `write_and_sync` only guarantees the *contents* of the temporary file
    /// survive a crash; the directory entry produced by the rename is separate
    /// metadata. Without this, a power loss right after `rename` returns can
    /// leave the directory still pointing at the pre-rename file, so a store
    /// that reported a successful write comes back stale.
    ///
    /// Defaults to a no-op: test doubles have nothing to sync, and
    /// [`FileKernelStateStore`](crate::kernel_state::FileKernelStateStore)
    /// deliberately opts out so it can report a failed sync as an
    /// indeterminate (rather than failed) commit.
    fn sync_parent_dir(&self, path: &Path) -> io::Result<()> {
        let _ = path;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AtomicJsonError {
    NotCommitted(String),
    Indeterminate(String),
}

impl AtomicJsonError {
    pub(crate) fn is_indeterminate(&self) -> bool {
        matches!(self, Self::Indeterminate(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::NotCommitted(message) => message,
            Self::Indeterminate(message) => format!(
                "{message}; candidate was committed but directory durability is indeterminate; retry or restart Kestral before making further changes"
            ),
        }
    }
}

pub(crate) struct StandardAtomicFileWriter;

impl AtomicFileWriter for StandardAtomicFileWriter {
    fn write_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use std::io::Write;

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn sync_parent_dir(&self, path: &Path) -> io::Result<()> {
        // Only meaningful on Unix: Windows offers no directory handle to sync,
        // and ReplaceFile/MoveFileEx already order the metadata write.
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            return fs::File::open(parent).and_then(|directory| directory.sync_all());
        }
        let _ = path;
        Ok(())
    }
}

pub(crate) fn standard_writer() -> Arc<dyn AtomicFileWriter> {
    Arc::new(StandardAtomicFileWriter)
}

pub(crate) fn load_json_document<T>(path: &Path, label: &str) -> Result<Option<T>, String>
where
    T: DeserializeOwned,
{
    discard_stale_temporary_files(path, label)?;

    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("open {label} failed: {error}")),
    };
    restrict_existing_file(&file, label)?;
    parse_document(file, path, label).map(Some)
}

fn restrict_existing_file(file: &fs::File, label: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("restrict {label} permissions failed: {error}"))?;
    }
    let _ = (file, label);
    Ok(())
}

pub(crate) fn persist_json_document<T>(
    path: &Path,
    document: &T,
    label: &str,
    writer: &dyn AtomicFileWriter,
) -> Result<(), AtomicJsonError>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AtomicJsonError::NotCommitted(format!("create {label} directory failed: {error}"))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(document).map_err(|error| {
        AtomicJsonError::NotCommitted(format!("serialize {label} failed: {error}"))
    })?;
    let temp_path = temporary_path(path);

    if let Err(error) = writer.write_and_sync(&temp_path, &bytes) {
        return Err(AtomicJsonError::NotCommitted(format_failure_with_cleanup(
            format!("write {label} failed: {error}"),
            writer,
            &temp_path,
        )));
    }
    if let Err(error) = writer.rename(&temp_path, path) {
        return Err(AtomicJsonError::NotCommitted(format_failure_with_cleanup(
            format!("replace {label} failed: {error}"),
            writer,
            &temp_path,
        )));
    }
    // The rename has already taken effect here, so there is no temporary file
    // left to clean up — only the durability of that rename is in question.
    writer.sync_parent_dir(path).map_err(|error| {
        AtomicJsonError::Indeterminate(format!("sync {label} directory failed: {error}"))
    })?;
    Ok(())
}

fn parse_document<T>(mut file: fs::File, path: &Path, label: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let mut raw = String::new();
    file.read_to_string(&mut raw)
        .map_err(|error| format!("read {label} failed: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "parse {label} failed; preserved '{}': {error}",
            path.display()
        )
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()))
}

fn discard_stale_temporary_files(path: &Path, label: &str) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let prefix = format!(".{file_name}.");
    let entries = fs::read_dir(parent)
        .map_err(|error| format!("inspect stale {label} temporary files failed: {error}"))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("inspect stale {label} temporary files failed: {error}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let uuid = name
            .strip_prefix(&prefix)
            .and_then(|suffix| suffix.strip_suffix(".tmp"));
        if uuid.is_some_and(|uuid| Uuid::parse_str(uuid).is_ok()) {
            fs::remove_file(entry.path()).map_err(|error| {
                format!(
                    "discard stale {label} temporary file '{}' failed; preserved it for recovery: {error}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
}

fn format_failure_with_cleanup(
    failure: String,
    writer: &dyn AtomicFileWriter,
    temp_path: &Path,
) -> String {
    match writer.remove_file(temp_path) {
        Ok(()) => failure,
        Err(error) if error.kind() == io::ErrorKind::NotFound => failure,
        Err(error) => format!(
            "{failure}; discard temporary file '{}' failed; preserved it for recovery: {error}",
            temp_path.display()
        ),
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum FailingFileOperation {
    Write,
    Rename,
    SyncParent,
}

#[cfg(test)]
pub(crate) struct FailingAtomicFileWriter {
    failure: FailingFileOperation,
}

#[cfg(test)]
impl FailingAtomicFileWriter {
    pub(crate) fn new(failure: FailingFileOperation) -> Self {
        Self { failure }
    }
}

#[cfg(test)]
impl AtomicFileWriter for FailingAtomicFileWriter {
    fn write_and_sync(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if matches!(self.failure, FailingFileOperation::Write) {
            return Err(io::Error::other("injected write failure"));
        }
        StandardAtomicFileWriter.write_and_sync(path, bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if matches!(self.failure, FailingFileOperation::Rename) {
            return Err(io::Error::other("injected rename failure"));
        }
        StandardAtomicFileWriter.rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        StandardAtomicFileWriter.remove_file(path)
    }

    fn sync_parent_dir(&self, path: &Path) -> io::Result<()> {
        if matches!(self.failure, FailingFileOperation::SyncParent) {
            return Err(io::Error::other("injected parent directory sync failure"));
        }
        StandardAtomicFileWriter.sync_parent_dir(path)
    }
}

#[cfg(test)]
mod tests;
