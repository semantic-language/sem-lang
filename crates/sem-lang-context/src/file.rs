//! Bounded file image storage, not a checkpoint/CAS or replay protocol.
use crate::ContextError;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

pub const MAX_CONTEXT_BYTES: usize = 1_048_576;

fn invalid(message: &str) -> ContextError {
    ContextError::InvalidContext(message.into())
}

pub(crate) fn io(error: std::io::Error) -> ContextError {
    ContextError::RepositoryUnavailable {
        kind: format!("{:?}", error.kind()),
        detail: error.to_string(),
    }
}

pub(crate) fn read_bounded(path: &Path) -> Result<Vec<u8>, ContextError> {
    if !fs::symlink_metadata(path).map_err(io)?.is_file() {
        return Err(invalid("context is not a regular file"));
    }
    let mut bytes = Vec::new();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x800 | 0x20000); // O_NONBLOCK | O_NOFOLLOW
    }
    let file = options.open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.len() > MAX_CONTEXT_BYTES as u64 {
        return Err(invalid("context input capacity or file type rejected"));
    }
    file.take(MAX_CONTEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MAX_CONTEXT_BYTES {
        return Err(invalid("context exceeds capacity"));
    }
    Ok(bytes)
}

pub(crate) fn lock(path: &Path) -> Result<File, ContextError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.is_file()
    {
        return Err(invalid("context lock is not a regular file"));
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(io)?;
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(ContextError::Busy { wait_millis: 500 });
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(io(error)),
        }
    }
}

// Caller holds the sibling lock for the whole read/compare/write transition.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> Result<(), ContextError> {
    if bytes.len() > MAX_CONTEXT_BYTES {
        return Err(invalid("context exceeds capacity"));
    }
    let temporary = path.with_extension("pending");
    if let Ok(metadata) = fs::symlink_metadata(&temporary) {
        if !metadata.is_file() {
            return Err(invalid("context pending file is not regular"));
        }
        fs::remove_file(&temporary).map_err(io)?;
    }
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(io)?;
        }
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&temporary, path).map_err(io)?;
        #[cfg(unix)]
        {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .map_or_else(std::env::current_dir, Ok)
                .map_err(io)?;
            File::open(parent).map_err(io)?.sync_all().map_err(io)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn save_atomic(path: &Path, bytes: &[u8]) -> Result<(), ContextError> {
    let _guard = lock(&path.with_extension("lock"))?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.is_file()
    {
        return Err(invalid("context is not a regular file"));
    }
    replace(path, bytes)
}
