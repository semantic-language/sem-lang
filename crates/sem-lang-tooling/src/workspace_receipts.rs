//! Immutable filesystem projection of successful graph mutation receipts.

use super::{ImmutableReceipt, MAX_RECEIPT_BYTES, ToolingError, change_error};
use sem_lang_core::Digest;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Publish a successful mutation receipt as an immutable projection of the
/// canonical graph commit. The graph remains authoritative. If publication is
/// interrupted after that commit, applying the same content-addressed plan
/// again reaches the same graph state and retries this exact publication.
pub(super) fn persist_receipt<T: ImmutableReceipt>(
    project_file: &Path,
    receipt: &T,
) -> Result<(), ToolingError> {
    let digest = receipt.plan_digest();
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(change_error("operation receipt plan digest is invalid"));
    }

    let mut bytes = serde_json::to_vec_pretty(receipt)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(change_error("operation receipt exceeds the size limit"));
    }
    let receipt_digest = Digest::document_digest(&bytes).to_hex();

    let base = crate::project_directory(project_file)?;
    let directory = crate::resolve_project_path(
        &base,
        crate::project::RUNTIME_RECEIPTS_PATH,
        "operation receipt store",
    )?;
    // The plan digest identifies the command; it is not the content identity
    // of its result. The Foundation chooses the original owner output for that
    // command, and this projection is addressed by the digest of those exact
    // bytes. Distinct observations therefore cannot overwrite one another.
    let target = directory.join(format!("{receipt_digest}.json"));
    if receipt_matches(&target, &bytes)? {
        return Ok(());
    }

    let (temporary, mut file) = create_receipt_temporary(&directory, &receipt_digest)?;

    let publication = (|| -> Result<(), ToolingError> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);

        match fs::hard_link(&temporary, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !receipt_matches(&target, &bytes)? {
                    return Err(change_error(
                        "operation receipt conflicts with an existing immutable receipt",
                    ));
                }
            }
            Err(error) => return Err(error.into()),
        }
        fs::remove_file(&temporary)?;
        #[cfg(unix)]
        fs::File::open(&directory)?.sync_all()?;
        Ok(())
    })();

    if publication.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    publication
}

fn create_receipt_temporary(
    directory: &Path,
    digest: &str,
) -> Result<(PathBuf, fs::File), ToolingError> {
    for attempt in 0_u8..16 {
        let path = directory.join(format!(".{digest}.{}.{attempt}.tmp", std::process::id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(change_error("operation receipt store is busy"))
}

/// `false` means that no receipt has been published yet. Any existing value
/// must be the same bounded regular file; immutable conflicts are corruption,
/// never an overwrite request.
fn receipt_matches(path: &Path, expected: &[u8]) -> Result<bool, ToolingError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_RECEIPT_BYTES
    {
        return Err(change_error(
            "operation receipt is not a bounded physical file",
        ));
    }
    let current = fs::read(path)?;
    if current == expected {
        Ok(true)
    } else {
        Err(change_error(
            "operation receipt conflicts with an existing immutable receipt",
        ))
    }
}
