//! Filesystem adapter physical budgets; unrelated to semantic memory duration.
use std::{
    fs::{self, DirEntry, OpenOptions},
    io::{self, Read},
    path::Path,
};
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_STORE_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_ENTRIES: usize = 4096;

pub(crate) fn capacity() -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        "repository resource capacity exceeded",
    )
}
pub(crate) fn read(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "repository input is not a regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x800 | 0x20000); // O_NONBLOCK | O_NOFOLLOW
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "repository input is not a regular file",
        ));
    }
    if meta.len() > limit {
        return Err(capacity());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(capacity());
    }
    Ok(bytes)
}
pub(crate) fn entries(path: &Path) -> io::Result<Vec<DirEntry>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "repository directory rejected",
        ));
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(path)? {
        if out.len() == MAX_ENTRIES {
            return Err(capacity());
        }
        out.push(entry?);
    }
    Ok(out)
}
/// A bounded inventory counts staging/orphan bytes too. It never reclaims them
/// or decides which semantic/canonical references may be retired.
pub(crate) fn admit(root: &Path, additional: u64, writing: bool) -> io::Result<()> {
    // A write may add one prefix directory, one staging file and one final
    // file. Reserve their peak metadata cost before any mutation.
    let mut remaining_entries = MAX_ENTRIES - if writing { 3 } else { 0 };
    let mut remaining_bytes = MAX_STORE_BYTES
        .checked_sub(additional)
        .ok_or_else(capacity)?;
    let mut pending = vec![(root.to_owned(), 0)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > 5 {
            return Err(capacity());
        }
        for entry in entries(&directory)? {
            remaining_entries = remaining_entries.checked_sub(1).ok_or_else(capacity)?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "repository symlink rejected",
                ));
            }
            if metadata.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if metadata.is_file() {
                remaining_bytes = remaining_bytes
                    .checked_sub(metadata.len())
                    .ok_or_else(capacity)?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "repository special file rejected",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writes_still_reserve_metadata_capacity() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_ENTRIES - 2 {
            fs::write(root.path().join(index.to_string()), []).unwrap();
        }
        admit(root.path(), 0, false).unwrap();
        assert_eq!(
            admit(root.path(), 0, true).unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), MAX_ENTRIES - 2);
    }
}
