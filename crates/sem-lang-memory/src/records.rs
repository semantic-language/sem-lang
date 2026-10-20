//! One verified immutable-record path for language definition and binding adoption.
use sem_lang_core::Digest;
use sem_lang_repository::commit_image::{BorrowedBackend, SnapshotBackend};
use sem_lang_repository::{
    Repository, RepositoryError, RepositoryRevision, RevisionId, RevisionRef, head_intent,
};
use serde::{Serialize, de::DeserializeOwned};
use zixcel_revision::{CommitOutcome, CommitStore};

pub(crate) const MAX_BYTES: usize = 1_048_576;

pub(crate) fn invalid(message: &str) -> RepositoryError {
    RepositoryError::Conflict(message.into())
}

pub(crate) fn head(prefix: &str, namespace: &str) -> Result<String, RepositoryError> {
    if namespace.is_empty()
        || namespace.len() > 116
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(invalid("invalid adoption namespace"));
    }
    Ok(format!("{prefix}-{namespace}"))
}

pub(crate) trait Record: Serialize + DeserializeOwned {
    const PREFIX: &'static str;
    fn namespace(&self) -> &str;
    fn decision_ref(&self) -> &str;
    fn parent(&self) -> Option<&RevisionId>;
    fn validate<R: Repository>(&self, repository: &R) -> Result<(), RepositoryError>;
    fn validate_parent(&self, _parent: Option<&Self>) -> Result<(), RepositoryError> {
        Ok(())
    }
}

pub(crate) fn read_at<R: Repository, T: Record>(
    repository: &R,
    namespace: &str,
    id: &RevisionId,
) -> Result<T, RepositoryError> {
    let record: T = read_one(repository, namespace, id)?;
    let mut current: Option<T> = None;
    let mut seen = std::collections::BTreeSet::from([id.clone()]);
    loop {
        let child = current.as_ref().unwrap_or(&record);
        let Some(parent_id) = child.parent() else {
            child.validate_parent(None)?;
            break;
        };
        if !seen.insert(parent_id.clone()) || seen.len() > 4096 {
            return Err(invalid("invalid or excessive adoption parent chain"));
        }
        let parent: T = read_one(repository, namespace, parent_id)?;
        child.validate_parent(Some(&parent))?;
        current = Some(parent);
    }
    Ok(record)
}

fn read_one<R: Repository, T: Record>(
    repository: &R,
    namespace: &str,
    id: &RevisionId,
) -> Result<T, RepositoryError> {
    head(T::PREFIX, namespace)?;
    let revision = repository.load_revision(id)?;
    if revision.id != *id || revision.objects.len() != 1 || !revision.payload_delta.is_empty() {
        return Err(invalid("invalid adoption revision"));
    }
    let object = &revision.objects[0];
    if *id != format!("{}-{object}", T::PREFIX) {
        return Err(invalid("adoption revision is not content bound"));
    }
    let bytes = repository
        .get_object(object)?
        .ok_or_else(|| RepositoryError::MissingObject(object.clone()))?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("adoption record exceeds limit"));
    }
    let digest = Digest::object_digest(&bytes).to_hex();
    if digest != *object {
        return Err(RepositoryError::DigestMismatch {
            expected: object.clone(),
            actual: digest,
        });
    }
    let record: T = serde_json::from_slice(&bytes).map_err(|e| invalid(&e.to_string()))?;
    if record.namespace() != namespace || record.parent() != revision.parent.as_ref() {
        return Err(invalid("adoption namespace or parent mismatch"));
    }
    record.validate(repository)?;
    Ok(record)
}

pub(crate) fn read<R: Repository, T: Record>(
    repository: &R,
    namespace: &str,
) -> Result<Option<(RevisionId, T)>, RepositoryError> {
    repository
        .load_head(&head(T::PREFIX, namespace)?)?
        .map(|id| read_at(repository, namespace, &id).map(|record| (id, record)))
        .transpose()
}

pub(crate) fn write<R: Repository + sem_lang_repository::Backend, T: Record>(
    repository: &mut R,
    record: &T,
    expected_commit_revision: &RevisionRef,
) -> Result<RevisionId, RepositoryError> {
    let name = head(T::PREFIX, record.namespace())?;
    let bytes = serde_json::to_vec(record).map_err(|e| invalid(&e.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("adoption record exceeds limit"));
    }
    let object_id = Digest::object_digest(&bytes).to_hex();
    let id = format!("{}-{object_id}", T::PREFIX);
    let publication = sem_lang_repository::HeadPublication {
        root: sem_lang_repository::RepositoryRoot::Revision(id.clone()),
        request_digest: Digest::object_digest(&bytes),
    };
    let prepared = head_intent(
        &name,
        record.decision_ref(),
        expected_commit_revision,
        &publication,
    )?;
    let commits = CommitStore::new(BorrowedBackend(&*repository));
    if commits.receipt(&name, record.decision_ref())?.is_some() {
        // Foundation replay happens before reading today's adoption or validating
        // new meaning. Different bytes/base are a Foundation operation reuse.
        repository.publish_head(&prepared)?;
        return Ok(id);
    }
    record.validate(repository)?;
    let decoded: T = serde_json::from_slice(&bytes).map_err(|e| invalid(&e.to_string()))?;
    decoded.validate(repository)?;
    // This is domain lineage validation against the explicitly selected base,
    // not a comparison with the current concurrency head.
    if repository.head_at(expected_commit_revision)?
        != record
            .parent()
            .cloned()
            .map(sem_lang_repository::RepositoryRoot::Revision)
    {
        return Err(invalid("adoption semantic parent mismatch"));
    }
    if let Some(base) = record.parent() {
        let parent: T = read_at(repository, record.namespace(), base)?;
        record.validate_parent(Some(&parent))?;
    } else {
        record.validate_parent(None)?;
    }
    // The storage owner already holds its repository write fence. A Foundation
    // proposal preflight rejects stale requests before immutable object allocation.
    let preview = SnapshotBackend::new(repository.read(Clone::clone)?)?;
    match CommitStore::new(&preview).commit(&prepared) {
        CommitOutcome::Committed(_) => {}
        outcome => {
            sem_lang_repository::commit_result(outcome)?;
            return Ok(id);
        }
    }
    if repository.put_object(bytes)? != object_id {
        return Err(invalid("repository object identity mismatch"));
    }
    let proposed = RepositoryRevision {
        id: id.clone(),
        parent: record.parent().cloned(),
        objects: vec![object_id],
        payload_delta: vec![],
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| invalid(&e.to_string()))?
            .as_secs() as i64,
        message: None,
    };
    match repository.load_revision(&id) {
        Ok(existing)
            if existing.id == proposed.id
                && existing.parent == proposed.parent
                && existing.objects == proposed.objects
                && existing.payload_delta.is_empty() => {}
        Ok(_) => return Err(invalid("conflicting immutable adoption revision")),
        Err(RepositoryError::MissingRevision(_)) => {
            if repository.store_revision(proposed)? != id {
                return Err(invalid("repository revision identity mismatch"));
            }
        }
        Err(e) => return Err(e),
    }
    repository.publish_head(&prepared)?;
    Ok(id)
}
