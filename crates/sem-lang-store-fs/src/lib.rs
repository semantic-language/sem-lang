//! Filesystem repository adapter with content-addressed immutable objects.
mod resource;

use sem_lang_core::Digest;
use sem_lang_repository::{Backend, CommitState, Failure};
use sem_lang_repository::{
    ObjectId, ObjectStore, Repository, RepositoryError, RepositoryRevision, RepositorySnapshot,
    RevisionId, RevisionStore, SnapshotStore, VerificationReport,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub enum FsRepositoryError {
    Io(String),
    InvalidIdentifier(String),
    Repository(RepositoryError),
    Internal(String),
}

impl From<RepositoryError> for FsRepositoryError {
    fn from(err: RepositoryError) -> Self {
        Self::Repository(err)
    }
}

impl Display for FsRepositoryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "io error: {msg}"),
            Self::InvalidIdentifier(id) => write!(f, "invalid identifier: {id}"),
            Self::Repository(err) => write!(f, "{err}"),
            Self::Internal(msg) => write!(f, "internal: {msg}"),
        }
    }
}

impl std::error::Error for FsRepositoryError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistFormat {
    version: u32,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistRevision {
    id: String,
    parent: Option<String>,
    objects: Vec<String>,
    payload_delta: Vec<u8>,
    created_at: i64,
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistSnapshot {
    revision: String,
    state_digest: Digest,
    serialized: Vec<u8>,
}

#[derive(Debug)]
pub struct FsRepository {
    root: PathBuf,
    format_version: u32,
    _lock: File,
    commit_access: Mutex<()>,
}

fn format_dir(path: &Path) -> PathBuf {
    path.join(".sem-lang")
}

fn objects_dir(path: &Path) -> PathBuf {
    format_dir(path).join("objects").join("sha256")
}

fn revisions_dir(path: &Path) -> PathBuf {
    format_dir(path).join("revisions")
}

fn snapshots_dir(path: &Path) -> PathBuf {
    format_dir(path).join("snapshots")
}

fn commit_file(path: &Path) -> PathBuf {
    format_dir(path).join("commits.json")
}

impl sem_lang_repository::RetentionStore for FsRepository {
    fn retention_metadata(&self) -> Result<Option<Vec<u8>>, RepositoryError> {
        let path = format_dir(&self.root).join("retention.json");
        let bytes = match resource::read(
            &path,
            sem_lang_repository::retention::MAX_RETENTION_BYTES as u64,
        ) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(repository_error(map_io(e))),
        };
        if !fs::symlink_metadata(path)
            .map_err(|e| RepositoryError::Io(e.to_string()))?
            .file_type()
            .is_file()
        {
            return Err(Failure::Corrupt.into());
        }
        Ok(Some(bytes))
    }
    fn store_retention_metadata(&mut self, bytes: &[u8]) -> Result<(), RepositoryError> {
        if bytes.len() > sem_lang_repository::retention::MAX_RETENTION_BYTES {
            return Err(Failure::Capacity.into());
        }
        // The repository's lifetime-exclusive lock permits exactly one bounded
        // staging file. Repeated crashes cannot accumulate timestamped images.
        let directory = format_dir(&self.root);
        resource::admit(&directory, bytes.len() as u64, true)
            .map_err(|err| repository_error(map_io(err)))?;
        let pending = directory.join("retention.pending");
        match fs::symlink_metadata(&pending) {
            Ok(meta) if meta.file_type().is_file() => {}
            Ok(_) => return Err(Failure::Corrupt.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(RepositoryError::Io(error.to_string())),
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&pending)
            .map_err(|e| RepositoryError::Io(e.to_string()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| RepositoryError::Io(e.to_string()))?;
        fs::rename(pending, directory.join("retention.json"))
            .map_err(|e| RepositoryError::Io(e.to_string()))?;
        sync_directory(&directory).map_err(|e| RepositoryError::Io(e.to_string()))
    }
    fn remove_retained_root(
        &mut self,
        root: &sem_lang_repository::RepositoryRoot,
        permit: &sem_lang_repository::retention::ValidatedReclamation,
    ) -> Result<(), RepositoryError> {
        permit.check_root(root)?;
        let path = match root {
            sem_lang_repository::RepositoryRoot::Object(id) => self.object_path(id),
            sem_lang_repository::RepositoryRoot::Revision(id) => self.revision_path(id),
            sem_lang_repository::RepositoryRoot::Snapshot(id) => self.snapshot_path(id),
        }
        .ok_or(Failure::Corrupt)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_file() => {}
            Ok(_) => return Err(Failure::Corrupt.into()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(RepositoryError::Io(e.to_string())),
        }
        fs::remove_file(&path).map_err(|e| RepositoryError::Io(e.to_string()))?;
        sync_directory(path.parent().ok_or(Failure::Corrupt)?)
            .map_err(|e| RepositoryError::Io(e.to_string()))
    }
}

fn locks_dir(path: &Path) -> PathBuf {
    format_dir(path).join("locks")
}

fn format_file(path: &Path) -> PathBuf {
    format_dir(path).join("format.json")
}

fn object_id(payload: &[u8]) -> String {
    Digest::object_digest(payload).to_hex()
}

impl FsRepository {
    /// Explicitly creates storage, or validates an already complete repository.
    /// An incomplete existing image is never repaired by initialization.
    pub fn create<P: AsRef<Path>>(root: P) -> Result<Self, FsRepositoryError> {
        let root = root.as_ref();
        match fs::symlink_metadata(format_dir(root)) {
            Ok(_) => return Self::open_existing(root),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io(error)),
        }
        fs::create_dir_all(root).map_err(map_io)?;
        // Claim the new layout exclusively. A racing initializer must not
        // overwrite or complete another owner's partially written image.
        fs::create_dir(format_dir(root)).map_err(map_io)?;
        Self::acquire(root, true)
    }

    /// Opens an already initialized repository without creating or repairing
    /// layout, format metadata or its lock file. Exact restoration is a read,
    /// not implicit creation. The same exclusive repository lock is acquired.
    pub fn open_existing<P: AsRef<Path>>(root: P) -> Result<Self, FsRepositoryError> {
        Self::acquire(root.as_ref(), false)
    }

    fn acquire(root: &Path, initialize: bool) -> Result<Self, FsRepositoryError> {
        let root = root.to_path_buf();
        let repository_root = format_dir(&root);

        for directory in [
            repository_root,
            objects_dir(&root),
            revisions_dir(&root),
            snapshots_dir(&root),
            locks_dir(&root),
        ] {
            if initialize {
                fs::create_dir_all(directory).map_err(map_io)?;
            } else if !fs::symlink_metadata(directory)
                .map_err(map_io)?
                .file_type()
                .is_dir()
            {
                return Err(FsRepositoryError::Internal(
                    "invalid repository directory".into(),
                ));
            }
        }

        let lock_path = locks_dir(&root).join("repository.lock");
        let format = format_file(&root);
        if !initialize {
            for file in [&lock_path, &format, &commit_file(&root)] {
                if !fs::symlink_metadata(file)
                    .map_err(map_io)?
                    .file_type()
                    .is_file()
                {
                    return Err(FsRepositoryError::Internal(
                        "invalid repository file".into(),
                    ));
                }
            }
        }
        let lock = OpenOptions::new()
            .create_new(initialize)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(map_io)?;
        lock.try_lock()
            .map_err(|error| FsRepositoryError::Io(format!("repository is busy: {error}")))?;

        let version = if initialize {
            let format_payload = PersistFormat {
                version: 2,
                created_at: now_ms() as i64,
            };
            let serialized = serde_json::to_vec_pretty(&format_payload)
                .map_err(|err| FsRepositoryError::Internal(err.to_string()))?;
            write_atomic(&format, &serialized)?;
            2
        } else {
            let raw = read_file(&format)?;
            let parsed = serde_json::from_slice::<PersistFormat>(&raw)
                .map_err(|err| FsRepositoryError::Internal(err.to_string()))?;
            parsed.version
        };
        if version != 2 {
            return Err(FsRepositoryError::Internal(
                "unsupported repository format; explicit reinitialization required".into(),
            ));
        }

        if initialize {
            write_atomic(
                &commit_file(&root),
                &serde_json::to_vec(&CommitState::default())
                    .map_err(|e| FsRepositoryError::Internal(e.to_string()))?,
            )?;
        }
        let repository = Self {
            root: root.to_path_buf(),
            format_version: version,
            _lock: lock,
            commit_access: Mutex::new(()),
        };
        resource::admit(&format_dir(&root), 0, false).map_err(map_io)?;
        repository
            .read(|_| ())
            .map_err(|e| FsRepositoryError::Repository(RepositoryError::CommitFailure(e)))?;
        Ok(repository)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn version(&self) -> u32 {
        self.format_version
    }

    fn object_path(&self, id: &str) -> Option<PathBuf> {
        validate_identifier(id)?;
        if id.len() < 4 {
            return None;
        }
        let (prefix, rest) = id.split_at(4);
        Some(objects_dir(&self.root).join(prefix).join(rest))
    }

    fn revision_path(&self, id: &str) -> Option<PathBuf> {
        validate_identifier(id).map(|_| revisions_dir(&self.root).join(id))
    }

    fn snapshot_path(&self, id: &str) -> Option<PathBuf> {
        validate_identifier(id).map(|_| snapshots_dir(&self.root).join(id))
    }

    fn put_bytes(&self, path: PathBuf, payload: Vec<u8>) -> Result<(), FsRepositoryError> {
        resource::admit(&format_dir(&self.root), payload.len() as u64, true).map_err(map_io)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(map_io)?;
        }
        write_atomic(&path, &payload)
    }

    fn all_revisions(&self) -> Result<Vec<String>, RepositoryError> {
        let mut ids = Vec::new();
        for entry in resource::entries(&revisions_dir(&self.root))
            .map_err(|err| repository_error(map_io(err)))?
        {
            if entry
                .file_type()
                .map_err(|err| RepositoryError::Io(err.to_string()))?
                .is_file()
            {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if validate_identifier(&file_name).is_some() {
                    ids.push(file_name);
                }
            }
        }
        Ok(ids)
    }
}

impl ObjectStore for FsRepository {
    fn get_object(&self, id: &ObjectId) -> Result<Option<Vec<u8>>, RepositoryError> {
        if validate_identifier(id).is_none() {
            return Err(RepositoryError::Conflict(format!(
                "invalid object id {}",
                id
            )));
        }
        let path = self
            .object_path(id)
            .ok_or_else(|| RepositoryError::Conflict(format!("invalid object id {id}")))?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(RepositoryError::Io(error.to_string())),
            Ok(entry) if !entry.is_file() => {
                return Err(RepositoryError::Io("object is not a regular file".into()));
            }
            Ok(_) => {}
        }
        let data = read_file(&path).map_err(repository_error)?;
        let actual = object_id(&data);
        if actual != *id {
            return Err(RepositoryError::DigestMismatch {
                expected: id.clone(),
                actual,
            });
        }
        Ok(Some(data))
    }

    fn put_object(&mut self, payload: Vec<u8>) -> Result<ObjectId, RepositoryError> {
        if payload.len() as u64 > resource::MAX_FILE_BYTES {
            return Err(Failure::Capacity.into());
        }
        let id = object_id(&payload);
        let path = self
            .object_path(&id)
            .ok_or_else(|| RepositoryError::Conflict("invalid object id".to_string()))?;

        if path.exists() {
            let existing = read_file(&path).map_err(repository_error)?;
            let existing_id = object_id(&existing);
            if existing_id != id {
                return Err(RepositoryError::DigestMismatch {
                    expected: id,
                    actual: existing_id,
                });
            }
            return Ok(id);
        }

        let data = payload;
        self.put_bytes(path, data).map_err(repository_error)?;
        Ok(id)
    }

    fn contains_object(&self, id: &ObjectId) -> bool {
        if validate_identifier(id).is_none() {
            return false;
        }
        self.object_path(id).is_some_and(|p| p.exists())
    }
}

impl RevisionStore for FsRepository {
    fn load_revision(&self, id: &RevisionId) -> Result<RepositoryRevision, RepositoryError> {
        let path = self
            .revision_path(id)
            .ok_or_else(|| RepositoryError::MissingRevision(id.clone()))?;
        let bytes = read_revision_file(&path, id)?;
        let dto = serde_json::from_slice::<PersistRevision>(&bytes)
            .map_err(|err| RepositoryError::CorruptObject(err.to_string()))?;
        Ok(RepositoryRevision {
            id: dto.id,
            parent: dto.parent,
            objects: dto.objects,
            payload_delta: dto.payload_delta,
            created_at: dto.created_at,
            message: dto.message,
        })
    }

    fn store_revision(&mut self, rev: RepositoryRevision) -> Result<RevisionId, RepositoryError> {
        if rev.id.is_empty() {
            return Err(RepositoryError::Conflict(
                "explicit revision identity required".into(),
            ));
        }

        if let Some(parent) = &rev.parent {
            self.load_revision(parent)?;
        }

        let path = self
            .revision_path(&rev.id)
            .ok_or_else(|| RepositoryError::Conflict("invalid revision id".to_string()))?;
        if path.exists() {
            return Err(RepositoryError::Conflict(
                "revision already exists".to_string(),
            ));
        }
        if !path.exists()
            && let Some(parent) = path.parent()
        {
            fs::create_dir_all(parent).map_err(|err| RepositoryError::Io(err.to_string()))?;
        }

        let dto = PersistRevision {
            id: rev.id.clone(),
            parent: rev.parent,
            objects: rev.objects,
            payload_delta: rev.payload_delta,
            created_at: rev.created_at,
            message: rev.message,
        };
        let serialized =
            serde_json::to_vec_pretty(&dto).map_err(|err| RepositoryError::Io(err.to_string()))?;
        self.put_bytes(path, serialized).map_err(repository_error)?;
        Ok(rev.id)
    }
}

impl FsRepository {
    fn read_commit_image(&self) -> Result<CommitState, Failure> {
        let path = commit_file(&self.root);
        let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::Storage)?;
        if !metadata.file_type().is_file() {
            return Err(Failure::Corrupt);
        }
        let bytes = resource::read(&path, resource::MAX_FILE_BYTES).map_err(|e| {
            if e.kind() == io::ErrorKind::FileTooLarge {
                Failure::Capacity
            } else {
                Failure::Storage
            }
        })?;
        let state: CommitState = serde_json::from_slice(&bytes).map_err(|_| Failure::Corrupt)?;
        state.validate()?;
        Ok(state)
    }
}
impl Backend for FsRepository {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        let _guard = self.commit_access.lock().map_err(|_| Failure::Storage)?;
        Ok(read(&self.read_commit_image()?))
    }
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let _guard = self.commit_access.lock().map_err(|_| Failure::Storage)?;
        let mut state = self.read_commit_image()?;
        let before = serde_json::to_vec(&state).map_err(|_| Failure::Corrupt)?;
        let result = write(&mut state)?;
        state.validate()?;
        let after = serde_json::to_vec(&state).map_err(|_| Failure::Corrupt)?;
        if after.len() > 64 * 1024 * 1024 {
            return Err(Failure::Capacity);
        }
        if after != before {
            resource::admit(&format_dir(&self.root), after.len() as u64, true).map_err(|e| {
                if e.kind() == io::ErrorKind::FileTooLarge {
                    Failure::Capacity
                } else {
                    Failure::Storage
                }
            })?;
            write_atomic(&commit_file(&self.root), &after).map_err(|_| Failure::Storage)?;
        }
        Ok(result)
    }
}

impl SnapshotStore for FsRepository {
    fn load_snapshot(&self, id: &RevisionId) -> Result<RepositorySnapshot, RepositoryError> {
        let path = self
            .snapshot_path(id)
            .ok_or_else(|| RepositoryError::MissingRevision(id.clone()))?;
        let bytes = read_revision_file(&path, id)?;
        let dto = serde_json::from_slice::<PersistSnapshot>(&bytes)
            .map_err(|err| RepositoryError::CorruptObject(err.to_string()))?;
        Ok(RepositorySnapshot {
            revision: dto.revision,
            state_digest: dto.state_digest,
            serialized: dto.serialized,
        })
    }

    fn store_snapshot(
        &mut self,
        snapshot: RepositorySnapshot,
    ) -> Result<RevisionId, RepositoryError> {
        if snapshot.revision.is_empty() {
            return Err(RepositoryError::Conflict(
                "explicit snapshot identity required".into(),
            ));
        }
        let path = self
            .snapshot_path(&snapshot.revision)
            .ok_or_else(|| RepositoryError::Conflict("invalid snapshot id".to_string()))?;
        let dto = PersistSnapshot {
            revision: snapshot.revision.clone(),
            state_digest: snapshot.state_digest,
            serialized: snapshot.serialized,
        };
        let serialized =
            serde_json::to_vec_pretty(&dto).map_err(|err| RepositoryError::Io(err.to_string()))?;
        if path.exists() {
            let existing = read_file(&path).map_err(repository_error)?;
            if existing != serialized {
                return Err(RepositoryError::Conflict("snapshot is immutable".into()));
            }
            return Ok(snapshot.revision);
        }
        self.put_bytes(path, serialized).map_err(repository_error)?;
        Ok(snapshot.revision)
    }
}

impl Repository for FsRepository {
    fn verify(&self) -> Result<VerificationReport, RepositoryError> {
        let mut report = VerificationReport {
            objects_checked: 0,
            revisions_checked: 0,
            errors: Vec::new(),
        };

        if !format_file(&self.root).exists() {
            report.errors.push("format.json missing".to_string());
        }

        let mut expected_objects = BTreeMap::new();
        resource::admit(&format_dir(&self.root), 0, false)
            .map_err(|err| repository_error(map_io(err)))?;
        for entry in resource::entries(&objects_dir(&self.root))
            .map_err(|err| repository_error(map_io(err)))?
        {
            if !entry
                .file_type()
                .map_err(|err| RepositoryError::Io(err.to_string()))?
                .is_dir()
            {
                continue;
            }
            for leaf in
                resource::entries(&entry.path()).map_err(|err| repository_error(map_io(err)))?
            {
                if leaf
                    .file_type()
                    .map_err(|err| RepositoryError::Io(err.to_string()))?
                    .is_file()
                {
                    let name = leaf.file_name();
                    if let Some(name) = name.to_str() {
                        let full_id = format!("{}{}", entry.file_name().to_string_lossy(), name);
                        expected_objects.insert(full_id, leaf.path());
                    }
                }
            }
        }

        for (id, path) in expected_objects {
            let data = read_file(&path).map_err(repository_error)?;
            let actual = object_id(&data);
            if actual != id {
                report
                    .errors
                    .push(format!("object digest mismatch for {id}"));
            }
            report.objects_checked = report.objects_checked.saturating_add(1);
        }

        for rev_id in self.all_revisions()? {
            let rev = self.load_revision(&rev_id)?;
            for obj in rev.objects {
                if !self.contains_object(&obj) {
                    report
                        .errors
                        .push(format!("revision {rev_id} references missing object {obj}"));
                }
            }
            report.revisions_checked = report.revisions_checked.saturating_add(1);
        }

        report
            .errors
            .extend(sem_lang_repository::verify_roots(self)?);

        Ok(report)
    }
}

fn validate_identifier(id: &str) -> Option<()> {
    if id.is_empty() {
        return None;
    }
    if id.contains("/") || id.contains('\\') || id.contains("..") {
        return None;
    }
    if id.len() > 256 {
        return None;
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_.".contains(ch))
    {
        return None;
    }
    Some(())
}

fn now_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_millis() as u64,
        Err(_) => 0,
    }
}

fn write_atomic(path: &Path, payload: &[u8]) -> Result<(), FsRepositoryError> {
    if payload.len() as u64 > resource::MAX_FILE_BYTES {
        return Err(FsRepositoryError::Repository(Failure::Capacity.into()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(map_io)?;
    }
    // Exclusive creation must never overwrite a canonical identifier or an
    // earlier crash remnant. Remnants remain charged to the finite store quota.
    let tmp = path.with_extension(format!("tmp-{}.json", now_ms()));

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)
        .map_err(map_io)?;
    // Cleanup is armed only after exclusive creation; never remove somebody
    // else's staging file when create_new rejects a collision.
    struct Pending(PathBuf);
    impl Drop for Pending {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _pending = Pending(tmp.clone());
    {
        file.write_all(payload).map_err(map_io)?;
        file.flush().map_err(map_io)?;
        file.sync_all().map_err(map_io)?;
    }
    drop(file);

    fs::rename(tmp.clone(), path).map_err(map_io)?;

    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), FsRepositoryError> {
    let dir = File::open(path).map_err(map_io)?;
    dir.sync_all().map_err(map_io)
}

fn read_file(path: &Path) -> Result<Vec<u8>, FsRepositoryError> {
    resource::read(path, resource::MAX_FILE_BYTES).map_err(map_io)
}

fn read_revision_file(path: &Path, id: &RevisionId) -> Result<Vec<u8>, RepositoryError> {
    resource::read(path, resource::MAX_FILE_BYTES).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => RepositoryError::MissingRevision(id.clone()),
        io::ErrorKind::FileTooLarge => Failure::Capacity.into(),
        _ => RepositoryError::Io(error.to_string()),
    })
}

fn map_io(err: io::Error) -> FsRepositoryError {
    if err.kind() == io::ErrorKind::FileTooLarge {
        FsRepositoryError::Repository(Failure::Capacity.into())
    } else {
        FsRepositoryError::Io(err.to_string())
    }
}

fn repository_error(error: FsRepositoryError) -> RepositoryError {
    match error {
        FsRepositoryError::Repository(error) => error,
        other => RepositoryError::Io(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_repository::HeadStore;

    #[test]
    fn r1_oversize_storage_is_typed_and_never_reset_or_admitted() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FsRepository::create(dir.path()).unwrap();
        let id = repo.put_object(b"original".to_vec()).unwrap();
        let path = repo.object_path(&id).unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(resource::MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            repo.get_object(&id),
            Err(RepositoryError::CommitFailure(Failure::Capacity))
        ));
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            resource::MAX_FILE_BYTES + 1
        );
        let orphan = format_dir(dir.path()).join("capacity-fixture");
        File::create(&orphan)
            .unwrap()
            .set_len(resource::MAX_STORE_BYTES)
            .unwrap();
        assert!(matches!(
            repo.put_object(b"new".to_vec()),
            Err(RepositoryError::CommitFailure(Failure::Capacity))
        ));
        assert!(!repo.contains_object(&object_id(b"new")));
        drop(repo);
        assert!(matches!(
            FsRepository::open_existing(dir.path()),
            Err(FsRepositoryError::Repository(
                RepositoryError::CommitFailure(Failure::Capacity)
            ))
        ));
        assert_eq!(
            fs::metadata(orphan).unwrap().len(),
            resource::MAX_STORE_BYTES
        );
    }

    #[test]
    fn missing_canonical_identity_never_allocates_a_time_or_sequence_revision() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FsRepository::create(dir.path()).unwrap();
        for _ in 0..100 {
            assert!(
                repo.store_revision(RepositoryRevision {
                    id: String::new(),
                    parent: None,
                    objects: vec![],
                    payload_delta: vec![],
                    created_at: 0,
                    message: None,
                })
                .is_err()
            );
            assert!(
                repo.store_snapshot(RepositorySnapshot {
                    revision: String::new(),
                    state_digest: Digest::object_digest(b"state"),
                    serialized: b"state".to_vec(),
                })
                .is_err()
            );
        }
        assert_eq!(repo.verify().unwrap().revisions_checked, 0);
        assert!(repo.load_head("main").unwrap().is_none());
    }

    #[test]
    fn repository_lock_cas_integrity_and_current_format_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("absent");
        assert!(FsRepository::open_existing(&absent).is_err());
        assert!(!absent.exists());
        let mut repo = FsRepository::create(dir.path()).unwrap();
        assert!(FsRepository::open_existing(dir.path()).is_err());
        assert!(matches!(
            repo.load_revision(&"absent".into()),
            Err(RepositoryError::MissingRevision(_))
        ));
        assert!(matches!(
            repo.load_snapshot(&"absent".into()),
            Err(RepositoryError::MissingRevision(_))
        ));
        let bad_revision = repo.revision_path("broken").unwrap();
        fs::write(&bad_revision, b"not-json").unwrap();
        assert!(matches!(
            repo.load_revision(&"broken".into()),
            Err(RepositoryError::CorruptObject(_))
        ));
        fs::remove_file(bad_revision).unwrap();
        assert!(FsRepository::create(dir.path()).is_err());
        let id = repo.put_object(b"exact".to_vec()).unwrap();
        assert_eq!(id, Digest::object_digest(b"exact").to_hex());
        let format = format_file(dir.path());
        let bytes = fs::read(&format).unwrap();
        fs::remove_file(&format).unwrap();
        assert!(FsRepository::open_existing(dir.path()).is_err());
        assert!(!format.exists());
        assert!(
            FsRepository::create(dir.path()).is_err(),
            "create must not repair incomplete existing storage"
        );
        assert!(!format.exists());
        fs::write(&format, &bytes).unwrap();
        let lock = locks_dir(dir.path()).join("repository.lock");
        fs::remove_file(&lock).unwrap();
        assert!(FsRepository::open_existing(dir.path()).is_err());
        assert!(!lock.exists());
        assert!(FsRepository::create(dir.path()).is_err());
        assert!(!lock.exists());
        fs::write(&lock, []).unwrap();
        fs::write(format_file(dir.path()), br#"{"version":1,"created_at":0}"#).unwrap();
        assert!(FsRepository::create(dir.path()).is_err());
        assert_eq!(
            fs::read(&format).unwrap(),
            br#"{"version":1,"created_at":0}"#
        );
        #[cfg(unix)]
        {
            let backing = dir.path().join("format-backing.json");
            fs::write(&backing, &bytes).unwrap();
            fs::remove_file(&format).unwrap();
            std::os::unix::fs::symlink(&backing, &format).unwrap();
            assert!(FsRepository::open_existing(dir.path()).is_err());
            assert!(FsRepository::create(dir.path()).is_err());
            assert_eq!(fs::read(&backing).unwrap(), bytes);
            assert!(fs::symlink_metadata(&format).unwrap().is_symlink());
        }
    }

    #[test]
    fn snapshot_preserves_digest_and_rejects_unknown_algorithm() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FsRepository::create(dir.path()).unwrap();
        let digest = Digest::semantic_state_digest(b"exact-state");
        let id = repo
            .store_snapshot(RepositorySnapshot {
                revision: "snapshot-1".into(),
                state_digest: digest,
                serialized: b"exact-state".to_vec(),
            })
            .unwrap();
        drop(repo);
        let repo = FsRepository::open_existing(dir.path()).unwrap();
        assert_eq!(repo.load_snapshot(&id).unwrap().state_digest, digest);
        let mut repo = repo;
        assert!(
            repo.store_snapshot(RepositorySnapshot {
                revision: id,
                state_digest: digest,
                serialized: b"replacement".to_vec(),
            })
            .is_err()
        );
        let encoded = serde_json::to_string(&digest).unwrap();
        assert_eq!(serde_json::from_str::<Digest>(&encoded).unwrap(), digest);
        assert!(serde_json::from_str::<Digest>(&encoded.replace("sha256", "unknown")).is_err());
    }

    #[test]
    fn verification_does_not_treat_missing_layout_or_unreadable_heads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let repo = FsRepository::create(dir.path()).unwrap();
        let head = commit_file(dir.path());
        let healthy = fs::read(&head).unwrap();
        fs::write(&head, [0xff]).unwrap();
        assert!(
            repo.verify().is_err(),
            "invalid head bytes must be surfaced"
        );
        assert_eq!(fs::read(&head).unwrap(), [0xff]);
        fs::remove_file(&head).unwrap();
        let references = commit_file(dir.path());
        assert!(repo.verify().is_err(), "missing commit image is not empty");
        assert!(!references.exists());
        fs::write(&references, healthy).unwrap();
        let revisions = revisions_dir(dir.path());
        fs::remove_dir(&revisions).unwrap();
        assert!(
            repo.verify().is_err(),
            "missing revision directory is not empty"
        );
        assert!(!revisions.exists());
    }

    #[test]
    fn object_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = FsRepository::create(dir.path()).expect("open");
        let id = repo.put_object(b"hello".to_vec()).unwrap();
        let loaded = repo.get_object(&id).unwrap().expect("loaded");
        assert_eq!(loaded, b"hello");
    }
}
