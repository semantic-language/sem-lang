//! Repository interfaces and immutable revision model.

pub mod commit_image;
pub mod commit_wire;
mod head;
pub mod retention;
pub use head::*;
pub use retention::RetentionStore;
pub use zixcel_revision::{
    Backend, CommitIntent, CommitReceipt, CommitState, CommitStore, Conflict, ExternalObjectRef,
    ExternalRegistration, ExternalRegistrationStatus, ExternalRetention, ExternalRoot,
    ExternalRootKind, Failure, OperationId, PreparedCommit, ReclamationPermit, Rejection,
    RevisionRef,
};

use sem_lang_core::Digest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

pub type ObjectId = String;
pub type RevisionId = String;

#[derive(Debug, Clone)]
pub struct RepositoryObject {
    pub id: ObjectId,
    pub payload: Vec<u8>,
    pub digest: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryRevision {
    pub id: RevisionId,
    pub parent: Option<RevisionId>,
    pub objects: Vec<ObjectId>,
    pub payload_delta: Vec<u8>,
    pub created_at: i64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    pub revision: RevisionId,
    pub state_digest: Digest,
    pub serialized: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryError {
    CommitConflict(#[serde(with = "commit_wire::Conflict")] Conflict),
    CommitFailure(#[serde(with = "commit_wire::Failure")] Failure),
    CommitRejected(#[serde(with = "commit_wire::Rejection")] Rejection),
    MissingObject(ObjectId),
    DigestMismatch { expected: String, actual: String },
    MissingRevision(String),
    Conflict(String),
    CorruptObject(String),
    Io(String),
}

impl Display for RepositoryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommitConflict(error) => write!(f, "commit conflict: {error:?}"),
            Self::CommitFailure(error) => write!(f, "{error}"),
            Self::CommitRejected(error) => write!(f, "{error}"),
            Self::MissingObject(id) => write!(f, "missing object {id}"),
            Self::DigestMismatch { expected, actual } => {
                write!(f, "digest mismatch expected={expected} actual={actual}")
            }
            Self::MissingRevision(r) => write!(f, "missing revision {r}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::CorruptObject(msg) => write!(f, "corrupt object: {msg}"),
            Self::Io(msg) => write!(f, "io error: {msg}"),
        }
    }
}

impl std::error::Error for RepositoryError {}
impl From<Failure> for RepositoryError {
    fn from(error: Failure) -> Self {
        Self::CommitFailure(error)
    }
}
impl From<Rejection> for RepositoryError {
    fn from(error: Rejection) -> Self {
        Self::CommitRejected(error)
    }
}

pub trait ObjectStore {
    fn get_object(&self, id: &ObjectId) -> Result<Option<Vec<u8>>, RepositoryError>;
    fn put_object(&mut self, payload: Vec<u8>) -> Result<ObjectId, RepositoryError>;
    fn contains_object(&self, id: &ObjectId) -> bool;
}

pub trait RevisionStore {
    fn load_revision(&self, id: &RevisionId) -> Result<RepositoryRevision, RepositoryError>;
    fn store_revision(&mut self, rev: RepositoryRevision) -> Result<RevisionId, RepositoryError>;
}

pub trait SnapshotStore {
    fn load_snapshot(&self, id: &RevisionId) -> Result<RepositorySnapshot, RepositoryError>;
    fn store_snapshot(
        &mut self,
        snapshot: RepositorySnapshot,
    ) -> Result<RevisionId, RepositoryError>;
}

pub trait Repository: ObjectStore + RevisionStore + HeadStore + SnapshotStore {
    fn verify(&self) -> Result<VerificationReport, RepositoryError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub objects_checked: usize,
    pub revisions_checked: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InMemoryObjectStore {
    objects: BTreeMap<ObjectId, RepositoryObject>,
}

impl InMemoryObjectStore {
    pub fn new() -> Self {
        Self {
            objects: BTreeMap::new(),
        }
    }
}

impl Default for InMemoryObjectStore {
    fn default() -> Self {
        Self::new()
    }
}

fn object_id_from_payload(payload: &[u8]) -> String {
    Digest::object_digest(payload).to_hex()
}

impl ObjectStore for InMemoryObjectStore {
    fn get_object(&self, id: &ObjectId) -> Result<Option<Vec<u8>>, RepositoryError> {
        Ok(self.objects.get(id).map(|obj| obj.payload.clone()))
    }

    fn put_object(&mut self, payload: Vec<u8>) -> Result<ObjectId, RepositoryError> {
        let id = object_id_from_payload(&payload);
        let digest = Digest::object_digest(&payload);
        self.objects.entry(id.clone()).or_insert(RepositoryObject {
            id: id.clone(),
            payload,
            digest,
        });
        Ok(id)
    }

    fn contains_object(&self, id: &ObjectId) -> bool {
        self.objects.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_objects_do_not_alias_and_ids_are_sha256() {
        let mut store = InMemoryObjectStore::new();
        let first = store.put_object(vec![1, 0]).unwrap();
        let second = store.put_object(vec![0, 31]).unwrap();
        assert_ne!(first, second);
        assert_eq!(store.get_object(&first).unwrap(), Some(vec![1, 0]));
        assert_eq!(store.get_object(&second).unwrap(), Some(vec![0, 31]));
        assert_eq!(
            Digest::object_digest(b"hello").to_hex(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn object_store_roundtrip() {
        let mut store = InMemoryObjectStore::new();
        let id = store.put_object(b"hello".to_vec()).unwrap();
        assert!(store.contains_object(&id));
        assert_eq!(store.get_object(&id).unwrap(), Some(b"hello".to_vec()));
    }
}
