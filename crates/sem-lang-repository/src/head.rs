//! Physical root encoding. Zixcel alone decides CAS, reuse and replay.
use crate::commit_image::BorrowedBackend;
use crate::*;
use zixcel_revision::CommitOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryRoot {
    Object(ObjectId),
    Revision(RevisionId),
    Snapshot(RevisionId),
}
impl RepositoryRoot {
    pub fn reference(&self) -> &str {
        match self {
            Self::Object(id) | Self::Revision(id) | Self::Snapshot(id) => id,
        }
    }
}
/// Bounded owner result: an immutable root and the exact request fingerprint.
/// No semantic record or runtime body is copied into the commit image.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadPublication {
    pub root: RepositoryRoot,
    pub request_digest: Digest,
}
impl HeadPublication {
    pub fn decode(bytes: &[u8]) -> Result<Self, RepositoryError> {
        if bytes.len() > 1024 {
            return Err(Failure::Corrupt.into());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| Failure::Corrupt)?;
        let id = value.root.reference();
        if id.is_empty() || id.len() > 256 || id.bytes().any(|b| !b.is_ascii_graphic()) {
            return Err(Failure::Corrupt.into());
        }
        Ok(value)
    }
}
pub fn head_intent(
    name: &str,
    operation_id: &str,
    expected: &RevisionRef,
    result: &HeadPublication,
) -> Result<PreparedCommit, RepositoryError> {
    let payload = serde_json::to_vec(result).map_err(|_| Failure::Corrupt)?;
    HeadPublication::decode(&payload)?;
    Ok(CommitIntent {
        domain: name.into(),
        operation_id: operation_id.into(),
        expected_revision: expected.clone(),
        parents: expected.commit.iter().cloned().collect(),
        payload,
    }
    .prepare()?)
}
pub fn commit_result(outcome: CommitOutcome) -> Result<CommitReceipt, RepositoryError> {
    match outcome {
        CommitOutcome::Committed(receipt) | CommitOutcome::NoChange(receipt) => Ok(receipt),
        CommitOutcome::Conflict(conflict) => Err(RepositoryError::CommitConflict(conflict)),
        CommitOutcome::Rejected(error) => Err(error.into()),
        CommitOutcome::Failure(error) => Err(error.into()),
    }
}
pub trait HeadStore {
    fn load_head(&self, name: &str) -> Result<Option<RevisionId>, RepositoryError>;
    fn head_revision(&self, name: &str) -> Result<RevisionRef, RepositoryError>;
    fn head_at(&self, revision: &RevisionRef) -> Result<Option<RepositoryRoot>, RepositoryError>;
    fn publish_head(&self, prepared: &PreparedCommit) -> Result<CommitReceipt, RepositoryError>;
    fn root_references(&self) -> Result<Vec<RepositoryRoot>, RepositoryError>;
}
impl<B: Backend> HeadStore for B {
    fn load_head(&self, name: &str) -> Result<Option<RevisionId>, RepositoryError> {
        CommitStore::new(BorrowedBackend(self))
            .current(name)?
            .map(|p| HeadPublication::decode(p.payload()).map(|v| v.root.reference().to_owned()))
            .transpose()
    }
    fn head_revision(&self, name: &str) -> Result<RevisionRef, RepositoryError> {
        let commits = CommitStore::new(BorrowedBackend(self));
        match commits.current(name)? {
            None => Ok(RevisionRef::default()),
            Some(current) => Ok(commits
                .receipt(name, current.operation_id())?
                .ok_or(Failure::Corrupt)?
                .committed_revision),
        }
    }
    /// Exact historical input, not a semantic digest used as a concurrency head.
    fn head_at(&self, revision: &RevisionRef) -> Result<Option<RepositoryRoot>, RepositoryError> {
        let commits = CommitStore::new(BorrowedBackend(self));
        match &revision.commit {
            None if revision.sequence == 0 => Ok(None),
            None => Err(Rejection::InvalidIntent.into()),
            Some(reference) => {
                let prepared = commits.committed(reference)?.ok_or(Failure::Corrupt)?;
                let receipt = commits
                    .receipt(prepared.domain(), prepared.operation_id())?
                    .ok_or(Failure::Corrupt)?;
                if &receipt.committed_revision != revision {
                    return Err(Rejection::InvalidIntent.into());
                }
                Ok(Some(HeadPublication::decode(prepared.payload())?.root))
            }
        }
    }
    fn publish_head(&self, prepared: &PreparedCommit) -> Result<CommitReceipt, RepositoryError> {
        HeadPublication::decode(prepared.payload())?;
        commit_result(CommitStore::new(BorrowedBackend(self)).commit(prepared))
    }
    /// Required current physical roots. Historical receipts remain replayable
    /// metadata, not promises of permanent external availability.
    fn root_references(&self) -> Result<Vec<RepositoryRoot>, RepositoryError> {
        let commits = CommitStore::new(BorrowedBackend(self));
        let mut domains = std::collections::BTreeSet::new();
        let mut roots = Vec::new();
        for reference in commits.roots()? {
            let prepared = commits.committed(&reference)?.ok_or(Failure::Corrupt)?;
            HeadPublication::decode(prepared.payload())?;
            if domains.insert(prepared.domain().to_owned()) {
                let current = commits
                    .current(prepared.domain())?
                    .ok_or(Failure::Corrupt)?;
                roots.push(HeadPublication::decode(current.payload())?.root);
            }
        }
        Ok(roots)
    }
}
/// Verify physical roots without interpreting language records or checkpoints.
pub fn verify_roots<R: Repository>(repository: &R) -> Result<Vec<String>, RepositoryError> {
    let mut errors = Vec::new();
    for root in repository.root_references()? {
        match root {
            RepositoryRoot::Object(id) => {
                if repository.get_object(&id)?.is_none() {
                    errors.push(format!("missing object root {id}"));
                }
            }
            RepositoryRoot::Revision(id) => match repository.load_revision(&id) {
                Ok(_) => {}
                Err(RepositoryError::MissingRevision(_)) => {
                    errors.push(format!("missing revision root {id}"))
                }
                Err(error) => return Err(error),
            },
            RepositoryRoot::Snapshot(id) => match repository.load_snapshot(&id) {
                Ok(_) => {}
                Err(RepositoryError::MissingRevision(_)) => {
                    errors.push(format!("missing snapshot root {id}"))
                }
                Err(error) => return Err(error),
            },
        }
    }
    Ok(errors)
}
