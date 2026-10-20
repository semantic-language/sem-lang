//! Physical owner metadata port, not semantic memory or a projection store.
use crate::{
    ExternalObjectRef, ExternalRetention, Failure, ReclamationPermit, RepositoryError,
    RepositoryRoot,
};

/// Implementations serialize mutations with the repository's exclusive writer.
/// Missing metadata reads as absent without creating a file. A write must be
/// atomic and durable before returning; only bounded reference metadata belongs here.
pub trait RetentionStore {
    fn retention_metadata(&self) -> Result<Option<Vec<u8>>, RepositoryError>;
    fn store_retention_metadata(&mut self, bytes: &[u8]) -> Result<(), RepositoryError>;
    /// Internal storage primitive; the semantic owner must obtain and validate
    /// the matching Graph reclamation permit before calling it. Retry is safe.
    fn remove_retained_root(
        &mut self,
        root: &RepositoryRoot,
        permit: &ValidatedReclamation,
    ) -> Result<(), RepositoryError>;
}

pub const MAX_RETENTION_BYTES: usize = 1_048_576;

pub fn external_root(root: &RepositoryRoot) -> Result<ExternalObjectRef, RepositoryError> {
    Ok(ExternalObjectRef {
        owner: "sem-lang/repository".into(),
        object: serde_json::to_string(root).map_err(|_| Failure::Corrupt)?,
    })
}
/// Ephemeral proof of an exact persisted Graph fence. Not serializable, not a
/// Grant and not a new authority. Only the validated constructor can create it.
pub struct ValidatedReclamation {
    root: RepositoryRoot,
    permit: ReclamationPermit,
}
impl ValidatedReclamation {
    pub fn permit(&self) -> &ReclamationPermit {
        &self.permit
    }
    pub fn check_root(&self, root: &RepositoryRoot) -> Result<(), RepositoryError> {
        if self.root != *root {
            return Err(Failure::Corrupt.into());
        }
        Ok(())
    }
}
pub fn validate_permit<P: ExternalRetention>(
    port: &P,
    root: &RepositoryRoot,
    permit: &ReclamationPermit,
) -> Result<ValidatedReclamation, RepositoryError> {
    if external_root(root)? != permit.object {
        return Err(Failure::Corrupt.into());
    }
    if port.claim_external_reclamation(&permit.object, 0)?.as_ref() != Some(permit) {
        return Err(Failure::Corrupt.into());
    }
    Ok(ValidatedReclamation {
        root: root.clone(),
        permit: permit.clone(),
    })
}
