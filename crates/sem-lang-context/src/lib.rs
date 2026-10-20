mod input;
pub use input::{
    CompileContext, ObservationError, ObservationReceipt, ObservationRequest, RetainedObservation,
    SurfaceInput,
};
mod current;
pub use current::*;
pub use sem_lang_core::{ContextKind, ContextScope, ContextValidity};
mod file;
#[cfg(test)]
extern crate self as sem_lang_context;
mod publication;
pub use publication::{ContextOperation, ContextReceipt, ContextRequest};
mod promotion;
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
pub use file::MAX_CONTEXT_BYTES;
pub use promotion::{
    CandidateDisposition, DecidedCandidate, PromotionOperation, PromotionReceipt, PromotionRequest,
};
use sem_lang_repository::{ObjectId, ObjectStore};
pub use sem_lang_repository::{RepositoryError as ContextRepositoryError, commit_wire};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
pub use zixcel_revision::{
    Conflict as CommitConflict, Failure as CommitFailure, Rejection as CommitRejection,
    RevisionRef as CommitRevision,
};

pub type Confidence = f32;
pub type SemanticValue = String;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ContextSource {
    ExplicitUserStatement,
    ActiveWorkingContext,
    SessionDerived,
    PersistentDefault,
    EnvironmentObservation,
    LocationDerived,
    ExternalProvided,
    InferenceProposal,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ContextAuthority {
    Explicit,
    ActiveContext,
    SessionDefault,
    AcceptedAssertion,
    PersistentDefault,
    Environment,
    Derived,
    Inferred,
}

impl ContextAuthority {
    pub fn rank(self) -> u8 {
        match self {
            Self::Explicit => 7,
            Self::ActiveContext => 6,
            Self::SessionDefault | Self::AcceptedAssertion => 5,
            Self::PersistentDefault => 4,
            Self::Environment => 3,
            Self::Derived => 2,
            Self::Inferred => 1,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextBinding {
    pub subject: ContextSubject,
    pub kind: ContextKind,
    pub value: SemanticValue,
    pub source: ContextSource,
    pub authority: ContextAuthority,
    pub scope: ContextScope,
    pub validity: ContextValidity,
    pub sensitivity: u8,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ContextSubject {
    pub id: String,
    pub class: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub reference: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextObservation {
    pub subject: ContextSubject,
    pub id: String,
    pub kind: ContextKind,
    pub value: SemanticValue,
    pub source: ContextSource,
    pub scope: ContextScope,
    pub evidence: EvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextDefaultCandidate {
    pub kind: ContextKind,
    pub value: SemanticValue,
    pub inferred_scope: ContextScope,
    pub inferred_validity: ContextValidity,
    pub supporting_evidence: Vec<EvidenceRef>,
    pub counter_evidence: Vec<EvidenceRef>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentContextualDefault {
    pub id: String,
    pub subject: ContextSubject,
    pub kind: ContextKind,
    pub value: SemanticValue,
    pub scope: ContextScope,
    pub validity: ContextValidity,
    pub evidence: Vec<EvidenceRef>,
    pub revision: String,
    /// Explicit promotion evidence; authority is verified by the caller.
    pub promotion_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRevision {
    pub id: String,
    pub defaults: Vec<PersistentContextualDefault>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextStore {
    pub decided_candidates: BTreeMap<String, DecidedCandidate>,
    pub commit_state: zixcel_revision::CommitState,
    pub persistent_defaults: Vec<PersistentContextualDefault>,
    pub observations: Vec<ContextObservation>,
    pub revisions: Vec<ContextRevision>,
    pub session_timezone: Option<ContextBinding>,
    pub active_timezone: Option<ContextBinding>,
    pub interpretation_rules: BTreeMap<String, String>,
    pub short_term: Vec<RetainedObservation>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum SurfaceRetention {
    DiscardAfterCompile,
    KeepForSession,
    KeepAsEvidence,
    KeepForAudit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ContextError {
    #[error("commit conflict: {0:?}")]
    CommitConflict(#[serde(with = "commit_wire::Conflict")] zixcel_revision::Conflict),
    #[error(transparent)]
    Commit(
        #[from]
        #[serde(with = "commit_wire::Failure")]
        zixcel_revision::Failure,
    ),
    #[error(transparent)]
    CommitRejected(
        #[from]
        #[serde(with = "commit_wire::Rejection")]
        zixcel_revision::Rejection,
    ),
    #[error(transparent)]
    Observation(#[from] ObservationError),
    #[error("context busy after {wait_millis}ms")]
    Busy { wait_millis: u64 },
    #[error("context already initialized")]
    AlreadyExists,
    #[error("candidate reference or subject conflict")]
    CandidateConflict,
    #[error("an explicit correction is required to change an accepted assertion")]
    CorrectionRequired,
    #[error("context revision conflict")]
    RevisionConflict,
    #[error("invalid context: {0}")]
    InvalidContext(String),
    #[error("context remains unresolved: {0}")]
    Unresolved(String),
    #[error("context candidate is not safe to promote")]
    UnsafePromotion,
    #[error("promotion reference must not be empty")]
    EmptyPromotionReference,
    #[error("context repository unavailable ({kind}): {detail}")]
    RepositoryUnavailable { kind: String, detail: String },
    #[error("context persistence: {0}")]
    Persistence(String),
    #[error("context repository: {0}")]
    Repository(#[from] sem_lang_repository::RepositoryError),
}

pub trait ContextInducer {
    fn induce(&self, observations: &[ContextObservation]) -> Vec<ContextDefaultCandidate>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ConservativeContextInducer;

impl ContextInducer for ConservativeContextInducer {
    fn induce(&self, observations: &[ContextObservation]) -> Vec<ContextDefaultCandidate> {
        let mut groups: BTreeMap<
            (ContextSubject, ContextKind, String, ContextScope),
            Vec<&ContextObservation>,
        > = BTreeMap::new();
        for observation in observations {
            groups
                .entry((
                    observation.subject.clone(),
                    observation.kind,
                    observation.value.clone(),
                    observation.scope,
                ))
                .or_default()
                .push(observation);
        }
        groups
            .into_iter()
            .filter(|(_, values)| values.len() >= 2)
            .map(
                |((_subject, kind, value, scope), values)| ContextDefaultCandidate {
                    kind,
                    value,
                    inferred_scope: if scope == ContextScope::Session {
                        ContextScope::PersistentAgentDefault
                    } else {
                        scope
                    },
                    inferred_validity: ContextValidity::default(),
                    supporting_evidence: values.iter().map(|item| item.evidence.clone()).collect(),
                    counter_evidence: Vec::new(),
                    confidence: 1.0,
                },
            )
            .collect()
    }
}

impl ContextStore {
    pub fn validate(&self) -> Result<(), ContextError> {
        self.commit_state.validate()?;
        self.validate_observations()?;
        Ok(())
    }
    /// Explicit first creation, serialized with all later mutations. Reads never
    /// call this method, and an existing/corrupt file is never replaced by it.
    pub fn create(path: impl AsRef<Path>) -> Result<(), ContextError> {
        let path = path.as_ref();
        let _lock = file::lock(&path.with_extension("lock"))?;
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Err(ContextError::AlreadyExists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(file::io(error)),
        }
        file::replace(
            path,
            &serde_json::to_vec(&Self::default())
                .map_err(|error| ContextError::Persistence(error.to_string()))?,
        )
    }

    /// Existing file persistence with a bounded exclusive read/modify/write.
    /// Foundation publication and owner validation both run under this storage lock.
    fn with_locked_engine<T>(
        path: impl AsRef<Path>,
        operation: impl FnOnce(&mut ContextEngine) -> Result<T, ContextError>,
    ) -> Result<T, ContextError> {
        let path = path.as_ref();
        let _lock = file::lock(&path.with_extension("lock"))?;
        let mut engine = ContextEngine::new(Self::load(path)?);
        let before = serde_json::to_vec(&engine.store)
            .map_err(|e| ContextError::Persistence(e.to_string()))?;
        let result = operation(&mut engine)?;
        engine.store.validate()?;
        let after = serde_json::to_vec(&engine.store)
            .map_err(|e| ContextError::Persistence(e.to_string()))?;
        if before != after {
            file::replace(path, &after)?;
        }
        Ok(result)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ContextError> {
        self.validate()?;
        file::save_atomic(
            path.as_ref(),
            &serde_json::to_vec(self).map_err(|e| ContextError::Persistence(e.to_string()))?,
        )
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ContextError> {
        let bytes = file::read_bounded(path.as_ref())?;
        let store: Self =
            serde_json::from_slice(&bytes).map_err(|e| ContextError::Persistence(e.to_string()))?;
        store.validate()?;
        Ok(store)
    }

    pub fn save_to_repository<R: ObjectStore>(
        &self,
        repository: &mut R,
    ) -> Result<ObjectId, ContextError> {
        self.validate()?;
        let payload = serde_json::to_vec(self)
            .map_err(|error| ContextError::Persistence(error.to_string()))?;
        if payload.len() > MAX_CONTEXT_BYTES {
            return Err(ContextError::InvalidContext(
                "context exceeds capacity".into(),
            ));
        }
        let expected = sem_lang_core::Digest::object_digest(&payload).to_hex();
        let actual = repository.put_object(payload)?;
        if actual != expected {
            return Err(
                sem_lang_repository::RepositoryError::DigestMismatch { expected, actual }.into(),
            );
        }
        Ok(actual)
    }

    pub fn load_from_repository<R: ObjectStore>(
        repository: &R,
        id: &ObjectId,
    ) -> Result<Self, ContextError> {
        let payload = repository
            .get_object(id)?
            .ok_or_else(|| sem_lang_repository::RepositoryError::MissingObject(id.clone()))?;
        if payload.len() > MAX_CONTEXT_BYTES {
            return Err(ContextError::InvalidContext(
                "context exceeds capacity".into(),
            ));
        }
        let actual = sem_lang_core::Digest::object_digest(&payload).to_hex();
        if actual != *id {
            return Err(sem_lang_repository::RepositoryError::DigestMismatch {
                expected: id.clone(),
                actual,
            }
            .into());
        }
        let store: Self = serde_json::from_slice(&payload).map_err(|error| {
            sem_lang_repository::RepositoryError::CorruptObject(error.to_string())
        })?;
        store.validate()?;
        Ok(store)
    }
}

#[derive(Debug, Clone)]
pub struct ContextEngine {
    pub store: ContextStore,
    subject_ref: Option<ContextSubject>,
}

impl ContextEngine {
    pub fn subject(&self) -> Option<&ContextSubject> {
        self.subject_ref.as_ref()
    }

    pub fn with_subject(mut self, subject: ContextSubject) -> Self {
        self.set_subject(subject);
        self
    }

    /// Explicit caller scope, not a semantic write or persisted identity guess.
    pub fn set_subject(&mut self, subject: ContextSubject) {
        self.subject_ref = Some(subject);
    }

    pub fn new(store: ContextStore) -> Self {
        Self {
            store,
            subject_ref: None,
        }
    }

    pub fn short_term(&self) -> &[RetainedObservation] {
        &self.store.short_term
    }

    pub fn short_term_size_bytes(&self) -> Result<usize, ContextError> {
        serde_json::to_vec(&self.store.short_term)
            .map(|bytes| bytes.len())
            .map_err(|error| ContextError::Persistence(error.to_string()))
    }

    pub fn induce_defaults(&self) -> Vec<ContextDefaultCandidate> {
        let mut candidates: Vec<_> = ConservativeContextInducer
            .induce(&self.store.observations)
            .into_iter()
            .filter(|candidate| {
                !self
                    .store
                    .decided_candidates
                    .values()
                    .any(|record| &record.candidate == candidate)
            })
            .filter(|candidate| {
                !self.store.persistent_defaults.iter().any(|accepted| {
                    accepted.kind == candidate.kind
                        && accepted.value == candidate.value
                        && accepted.scope == candidate.inferred_scope
                        && accepted.validity == candidate.inferred_validity
                })
            })
            .collect();
        candidates.extend(
            self.store
                .decided_candidates
                .values()
                .filter(|record| record.disposition == CandidateDisposition::Pending)
                .map(|record| record.candidate.clone()),
        );
        candidates
    }

    fn binding(
        &self,
        subject: ContextSubject,
        value: &str,
        source: ContextSource,
        authority: ContextAuthority,
        scope: ContextScope,
    ) -> ContextBinding {
        ContextBinding {
            subject,
            kind: ContextKind::TimeZone,
            value: value.to_string(),
            source,
            authority,
            scope,
            validity: ContextValidity::default(),
            sensitivity: 0,
        }
    }
}

#[cfg(test)]
mod acceptance_tests {
    use super::*;
    #[test]
    fn memory_storage_and_explicit_corrections_survive_without_temporal_compiler() {
        let mut repository = sem_lang_repository::InMemoryObjectStore::new();
        let mut engine = ContextEngine::new(ContextStore::default());
        test_support::timezone(&mut engine, "Asia/Tokyo", "test:explicit:1").unwrap();
        test_support::timezone(&mut engine, "UTC", "test:explicit:2").unwrap();
        let id = engine.store.save_to_repository(&mut repository).unwrap();
        let restored = ContextStore::load_from_repository(&repository, &id).unwrap();
        assert_eq!(restored.persistent_defaults[0].value, "UTC");
        assert_eq!(restored.revisions.len(), 2);
        assert_eq!(restored.revisions[0].defaults[0].value, "Asia/Tokyo");
        assert!(ContextAuthority::Explicit.rank() > ContextAuthority::Environment.rank());
        assert!(ContextAuthority::Environment.rank() > ContextAuthority::Derived.rank());
    }
}
