//! Explicit semantic decisions. Authenticating a subject or decision is the
//! caller's responsibility; neither an evidence string nor a score authorizes it.
use crate::*;
use sem_lang_core::Digest;
use sem_lang_repository::commit_image::SnapshotBackend;
use zixcel_revision::{CommitIntent, CommitOutcome, CommitStore, RevisionRef};

pub(crate) const DOMAIN: &str = "sem-lang/context";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PromotionOperation {
    SelectShortTerm,
    PromoteLongTerm,
    Retain,
    Reject,
    Replace { candidate: ContextDefaultCandidate },
    CorrectLongTerm { assertion_ref: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionRequest {
    pub subject_ref: ContextSubject,
    pub candidate_ref: String,
    pub expected_memory_revision: String,
    /// Commit fencing is distinct from the semantic content projection.
    pub expected_commit_revision: RevisionRef,
    pub decision_ref: String,
    pub operation: PromotionOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateDisposition {
    Pending,
    SelectedShortTerm,
    Promoted,
    Rejected,
    Replaced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecidedCandidate {
    pub subject_ref: ContextSubject,
    pub candidate: ContextDefaultCandidate,
    pub disposition: CandidateDisposition,
    pub replacement_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionReceipt {
    pub request: PromotionRequest,
    pub input_revision: String,
    pub output_revision: String,
    pub changed: bool,
}

use crate::publication::ContextResult;

fn encoded<T: Serialize>(value: &T) -> Result<Vec<u8>, ContextError> {
    serde_json::to_vec(value).map_err(|e| ContextError::InvalidContext(e.to_string()))
}

impl ContextDefaultCandidate {
    pub fn reference(&self) -> Result<String, ContextError> {
        if !self.confidence.is_finite()
            || !(0.0..=1.0).contains(&self.confidence)
            || self.value.trim().is_empty()
            || self.value.len() > 16_384
        {
            return Err(ContextError::UnsafePromotion);
        }
        Ok(Digest::object_digest(&encoded(self)?).to_hex())
    }
}

impl ContextEngine {
    pub fn commit_revision(&self) -> Result<RevisionRef, ContextError> {
        let backend = SnapshotBackend::new(self.store.commit_state.clone())?;
        let commits = CommitStore::new(&backend);
        match commits.current(DOMAIN)? {
            None => Ok(RevisionRef::default()),
            Some(current) => Ok(commits
                .receipt(DOMAIN, current.operation_id())?
                .ok_or(zixcel_revision::Failure::Corrupt)?
                .committed_revision),
        }
    }

    /// Current semantic data, excluding receipts and past revision projections.
    pub fn memory_revision(&self) -> Result<String, ContextError> {
        Ok(Digest::from_serializable(
            1,
            &(
                &self.store.persistent_defaults,
                &self.store.observations,
                &self.store.session_timezone,
                &self.store.active_timezone,
                &self.store.interpretation_rules,
                &self.store.short_term,
                &self.store.decided_candidates,
            ),
        )
        .map_err(|error| ContextError::InvalidContext(error.to_string()))?
        .to_hex())
    }

    /// Publishes an explicitly validated semantic selection through Foundation.
    pub fn promote(
        &mut self,
        candidate: ContextDefaultCandidate,
        request: &PromotionRequest,
    ) -> Result<PromotionReceipt, ContextError> {
        let backend = SnapshotBackend::new(self.store.commit_state.clone())?;
        let commits = CommitStore::new(&backend);
        // Exact replay is a foundation operation. Owner code only checks that
        // this semantic request is the request bound to the original result.
        let operation_id = request.decision_ref.clone();
        if let Some(committed) = commits.receipt(DOMAIN, &operation_id)? {
            let original = commits
                .committed(&committed.commit_ref)?
                .ok_or(zixcel_revision::Failure::Corrupt)?;
            let mut result: ContextResult = serde_json::from_slice(original.payload())
                .map_err(|_| zixcel_revision::Failure::Corrupt)?;
            if Digest::object_digest(&encoded(&candidate)?).to_hex() != request.candidate_ref {
                return Err(ContextError::CandidateConflict);
            }
            result.request_digest = Digest::object_digest(&encoded(request)?).to_hex();
            let replay = CommitIntent {
                domain: DOMAIN.into(),
                operation_id,
                expected_revision: request.expected_commit_revision.clone(),
                parents: request
                    .expected_commit_revision
                    .commit
                    .iter()
                    .cloned()
                    .collect(),
                payload: encoded(&result)?,
            }
            .prepare()?;
            match commits.commit(&replay) {
                CommitOutcome::NoChange(_) => {
                    return Ok(PromotionReceipt {
                        request: request.clone(),
                        input_revision: request.expected_memory_revision.clone(),
                        output_revision: result.output_revision,
                        changed: result.changed,
                    });
                }
                CommitOutcome::Failure(error) => return Err(error.into()),
                CommitOutcome::Conflict(conflict) => {
                    return Err(ContextError::CommitConflict(conflict));
                }
                CommitOutcome::Rejected(error) => return Err(error.into()),
                CommitOutcome::Committed(_) => {
                    return Err(zixcel_revision::Failure::Corrupt.into());
                }
            }
        }
        if request.decision_ref.trim().is_empty() {
            return Err(ContextError::EmptyPromotionReference);
        }
        if request.subject_ref.id.trim().is_empty()
            || request.subject_ref.class.trim().is_empty()
            || request.subject_ref.id.len() > 512
            || request.subject_ref.class.len() > 512
            || request.decision_ref.len() > 512
            || request.expected_memory_revision.len() != 64
        {
            return Err(ContextError::InvalidContext(
                "invalid explicit promotion identity".into(),
            ));
        }
        if candidate.reference()? != request.candidate_ref {
            return Err(ContextError::CandidateConflict);
        }
        // This is semantic validation, not the commit CAS: observations and
        // explicit short-term clearing also change the selected meaning input.
        if self.memory_revision()? != request.expected_memory_revision {
            return Err(ContextError::RevisionConflict);
        }
        let mut next = Self::new(self.store.clone());
        let mut record = DecidedCandidate {
            subject_ref: request.subject_ref.clone(),
            candidate: candidate.clone(),
            disposition: CandidateDisposition::Pending,
            replacement_ref: None,
        };
        if let Some(previous) = next.store.decided_candidates.get(&request.candidate_ref)
            && previous.subject_ref != request.subject_ref
        {
            return Err(ContextError::CandidateConflict);
        }
        match &request.operation {
            PromotionOperation::Retain => {}
            PromotionOperation::Reject => record.disposition = CandidateDisposition::Rejected,
            PromotionOperation::SelectShortTerm => {
                for previous in next.store.decided_candidates.values_mut() {
                    if previous.subject_ref == request.subject_ref
                        && previous.candidate.kind == candidate.kind
                        && previous.disposition == CandidateDisposition::SelectedShortTerm
                    {
                        previous.disposition = CandidateDisposition::Replaced;
                        previous.replacement_ref = Some(request.candidate_ref.clone());
                    }
                }
                record.disposition = CandidateDisposition::SelectedShortTerm;
            }
            PromotionOperation::Replace {
                candidate: replacement,
            } => {
                let reference = replacement.reference()?;
                if reference == request.candidate_ref {
                    return Err(ContextError::CandidateConflict);
                }
                if next.store.decided_candidates.contains_key(&reference) {
                    return Err(ContextError::CandidateConflict);
                }
                next.store.decided_candidates.insert(
                    reference.clone(),
                    DecidedCandidate {
                        subject_ref: request.subject_ref.clone(),
                        candidate: replacement.clone(),
                        disposition: CandidateDisposition::Pending,
                        replacement_ref: None,
                    },
                );
                record.disposition = CandidateDisposition::Replaced;
                record.replacement_ref = Some(reference);
            }
            PromotionOperation::PromoteLongTerm | PromotionOperation::CorrectLongTerm { .. } => {
                let existing = next.store.persistent_defaults.iter().position(|item| {
                    item.subject == request.subject_ref
                        && item.kind == candidate.kind
                        && item.scope == candidate.inferred_scope
                });
                match (&request.operation, existing) {
                    (PromotionOperation::CorrectLongTerm { assertion_ref }, Some(index))
                        if &next.store.persistent_defaults[index].id == assertion_ref => {}
                    (PromotionOperation::CorrectLongTerm { .. }, _) => {
                        return Err(ContextError::CandidateConflict);
                    }
                    (_, Some(index))
                        if next.store.persistent_defaults[index].value != candidate.value
                            || next.store.persistent_defaults[index].validity
                                != candidate.inferred_validity =>
                    {
                        return Err(ContextError::CorrectionRequired);
                    }
                    _ => {}
                }
                let revision = Digest::object_digest(&encoded(request)?).to_hex();
                let value = PersistentContextualDefault {
                    id: revision.clone(),
                    subject: request.subject_ref.clone(),
                    kind: candidate.kind,
                    value: candidate.value.clone(),
                    scope: candidate.inferred_scope,
                    validity: candidate.inferred_validity.clone(),
                    evidence: candidate.supporting_evidence.clone(),
                    revision: revision.clone(),
                    promotion_ref: request.decision_ref.clone(),
                };
                match existing {
                    Some(index)
                        if next.store.persistent_defaults[index].value == value.value
                            && next.store.persistent_defaults[index].validity == value.validity => {
                    }
                    Some(index) => next.store.persistent_defaults[index] = value,
                    None => next.store.persistent_defaults.push(value),
                }
                if encoded(&next.store.persistent_defaults)?
                    != encoded(&self.store.persistent_defaults)?
                {
                    next.store.revisions.push(ContextRevision {
                        id: revision,
                        defaults: next.store.persistent_defaults.clone(),
                    });
                }
                record.disposition = CandidateDisposition::Promoted;
            }
        }
        next.store
            .decided_candidates
            .insert(request.candidate_ref.clone(), record);
        let changed = next.memory_revision()? != request.expected_memory_revision;
        let output_revision = next.memory_revision()?;
        let receipt = PromotionReceipt {
            request: request.clone(),
            input_revision: request.expected_memory_revision.clone(),
            changed,
            output_revision,
        };
        let prepared = CommitIntent {
            domain: DOMAIN.into(),
            operation_id,
            parents: request
                .expected_commit_revision
                .commit
                .iter()
                .cloned()
                .collect(),
            expected_revision: request.expected_commit_revision.clone(),
            payload: encoded(&ContextResult {
                request_digest: Digest::object_digest(&encoded(request)?).to_hex(),
                output_revision: receipt.output_revision.clone(),
                changed: receipt.changed,
                observation_ref: None,
            })?,
        }
        .prepare()?;
        match commits.commit(&prepared) {
            CommitOutcome::Committed(_) => {}
            CommitOutcome::Conflict(conflict) => {
                return Err(ContextError::CommitConflict(conflict));
            }
            CommitOutcome::Rejected(error) => return Err(error.into()),
            CommitOutcome::Failure(error) => return Err(error.into()),
            CommitOutcome::NoChange(_) => return Err(zixcel_revision::Failure::Corrupt.into()),
        }
        next.store.commit_state = backend.into_state()?;
        self.store = next.store;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::context_fixture::ContextFixture;

    #[test]
    fn streamed_memory_revision_preserves_projection_and_mutation_identity() {
        let mut engine = ContextEngine::new(ContextStore::default());
        let mut revisions = std::collections::BTreeSet::new();
        for zone in ["UTC", "Asia/Tokyo", "Europe/Paris"] {
            engine.record_timezone_confirmation(zone);
            let store = &engine.store;
            let expected = Digest::object_digest(
                &encoded(&(
                    &store.persistent_defaults,
                    &store.observations,
                    &store.session_timezone,
                    &store.active_timezone,
                    &store.interpretation_rules,
                    &store.short_term,
                    &store.decided_candidates,
                ))
                .unwrap(),
            )
            .to_hex();
            assert_eq!(engine.memory_revision().unwrap(), expected);
            assert!(revisions.insert(expected));
        }
    }
}
