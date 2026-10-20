//! In-memory repository implementation.

use sem_lang_core::Digest;
use sem_lang_repository::{Backend, CommitState, Failure, commit_image::SnapshotBackend};
use sem_lang_repository::{
    ObjectId, ObjectStore, Repository, RepositoryError, RepositoryRevision, RepositorySnapshot,
    RevisionId, RevisionStore, SnapshotStore, VerificationReport,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct InMemoryRepository {
    objects: BTreeMap<ObjectId, Vec<u8>>,
    object_digests: BTreeMap<ObjectId, Digest>,
    revisions: BTreeMap<RevisionId, RepositoryRevision>,
    snapshots: BTreeMap<RevisionId, RepositorySnapshot>,
    commits: SnapshotBackend,
    retention: Option<Vec<u8>>,
}

impl InMemoryRepository {
    pub fn new() -> Self {
        Self {
            objects: BTreeMap::new(),
            object_digests: BTreeMap::new(),
            revisions: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            commits: SnapshotBackend::default(),
            retention: None,
        }
    }
}

impl Default for InMemoryRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl sem_lang_repository::RetentionStore for InMemoryRepository {
    fn retention_metadata(&self) -> Result<Option<Vec<u8>>, RepositoryError> {
        Ok(self.retention.clone())
    }
    fn store_retention_metadata(&mut self, bytes: &[u8]) -> Result<(), RepositoryError> {
        if bytes.len() > sem_lang_repository::retention::MAX_RETENTION_BYTES {
            return Err(Failure::Capacity.into());
        }
        self.retention = Some(bytes.to_vec());
        Ok(())
    }
    fn remove_retained_root(
        &mut self,
        root: &sem_lang_repository::RepositoryRoot,
        permit: &sem_lang_repository::retention::ValidatedReclamation,
    ) -> Result<(), RepositoryError> {
        permit.check_root(root)?;
        match root {
            sem_lang_repository::RepositoryRoot::Object(id) => {
                self.objects.remove(id);
                self.object_digests.remove(id);
            }
            sem_lang_repository::RepositoryRoot::Revision(id) => {
                self.revisions.remove(id);
            }
            sem_lang_repository::RepositoryRoot::Snapshot(id) => {
                self.snapshots.remove(id);
            }
        }
        Ok(())
    }
}

impl ObjectStore for InMemoryRepository {
    fn get_object(&self, id: &ObjectId) -> Result<Option<Vec<u8>>, RepositoryError> {
        Ok(self.objects.get(id).cloned())
    }

    fn put_object(&mut self, payload: Vec<u8>) -> Result<ObjectId, RepositoryError> {
        let digest = Digest::object_digest(&payload);
        let id = digest.to_hex();

        match self.objects.get(&id) {
            Some(existing) => {
                let existing_digest = Digest::object_digest(existing);
                if existing_digest.to_hex() != digest.to_hex() {
                    return Err(RepositoryError::DigestMismatch {
                        expected: digest.to_hex(),
                        actual: existing_digest.to_hex(),
                    });
                }
            }
            None => {
                self.objects.insert(id.clone(), payload);
                self.object_digests.insert(id.clone(), digest);
            }
        }

        Ok(id)
    }

    fn contains_object(&self, id: &ObjectId) -> bool {
        self.objects.contains_key(id)
    }
}

impl RevisionStore for InMemoryRepository {
    fn load_revision(&self, id: &RevisionId) -> Result<RepositoryRevision, RepositoryError> {
        self.revisions
            .get(id)
            .cloned()
            .ok_or_else(|| RepositoryError::MissingRevision(id.clone()))
    }

    fn store_revision(&mut self, rev: RepositoryRevision) -> Result<RevisionId, RepositoryError> {
        if rev.id.is_empty() {
            return Err(RepositoryError::Conflict(
                "explicit revision identity required".into(),
            ));
        }
        if let Some(parent) = &rev.parent
            && !self.revisions.contains_key(parent)
        {
            return Err(RepositoryError::MissingRevision(parent.clone()));
        }
        let id = rev.id.clone();
        if self.revisions.contains_key(&id) {
            return Err(RepositoryError::Conflict(format!(
                "revision {id} already exists",
            )));
        }
        let mut normalized = rev;
        normalized.id = id.clone();
        self.revisions.insert(id.clone(), normalized);
        Ok(id)
    }
}

impl Backend for InMemoryRepository {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        (&self.commits).read(read)
    }
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        (&self.commits).write(write)
    }
}

impl SnapshotStore for InMemoryRepository {
    fn load_snapshot(&self, id: &RevisionId) -> Result<RepositorySnapshot, RepositoryError> {
        self.snapshots
            .get(id)
            .cloned()
            .ok_or_else(|| RepositoryError::MissingRevision(id.clone()))
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
        let id = snapshot.revision.clone();
        let mut snapshot = snapshot;
        snapshot.revision = id.clone();
        if let Some(existing) = self.snapshots.get(&id) {
            if existing.state_digest != snapshot.state_digest
                || existing.serialized != snapshot.serialized
            {
                return Err(RepositoryError::Conflict("snapshot is immutable".into()));
            }
            return Ok(id);
        }
        self.snapshots.insert(id.clone(), snapshot);
        Ok(id)
    }
}

impl Repository for InMemoryRepository {
    fn verify(&self) -> Result<VerificationReport, RepositoryError> {
        let mut report = VerificationReport {
            objects_checked: 0,
            revisions_checked: 0,
            errors: Vec::new(),
        };
        for (id, payload) in &self.objects {
            let digest = self
                .object_digests
                .get(id)
                .cloned()
                .unwrap_or_else(|| Digest::object_digest(payload));
            if digest.to_hex() != Digest::object_digest(payload).to_hex() {
                report.errors.push(format!("object digest mismatch: {id}"));
            }
            report.objects_checked += 1;
        }
        for rev in self.revisions.values() {
            if let Some(parent) = &rev.parent
                && !self.revisions.contains_key(parent)
            {
                report
                    .errors
                    .push(format!("revision {} has missing parent {}", rev.id, parent));
            }
            for obj in &rev.objects {
                if !self.objects.contains_key(obj) {
                    report
                        .errors
                        .push(format!("revision {id} missing object {obj}", id = rev.id));
                }
            }
            report.revisions_checked += 1;
        }
        report
            .errors
            .extend(sem_lang_repository::verify_roots(self)?);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_repository::HeadStore;

    #[test]
    fn missing_canonical_identity_never_allocates_a_time_or_sequence_revision() {
        let mut repo = InMemoryRepository::new();
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
    fn repository_roundtrip() {
        let mut repo = InMemoryRepository::new();
        let obj = b"hello".to_vec();
        let id = repo.put_object(obj).unwrap();
        assert!(repo.contains_object(&id));
        let head = RepositoryRevision {
            id: "r1".to_string(),
            parent: None,
            objects: vec![id.clone()],
            payload_delta: vec![],
            created_at: 0,
            message: None,
        };
        repo.store_revision(head).unwrap();
        assert!(repo.load_revision(&"r1".to_string()).is_ok());
    }
}
