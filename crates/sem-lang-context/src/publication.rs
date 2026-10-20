//! Formal context mutations. Meaning stays here; publication is Zixcel's operation.
use crate::promotion::DOMAIN;
use crate::*;
use sem_lang_core::Digest;
use sem_lang_repository::commit_image::SnapshotBackend;
use zixcel_revision::{
    CommitIntent, CommitOutcome, CommitReceipt, CommitStore, Failure, OperationId, RevisionRef,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ContextOperation {
    Retain {
        observation: ObservationRequest,
    },
    Discard {
        reference: String,
    },
    Clear {
        targets: Vec<ContextReference>,
    },
    ClearShortTerm,
    BindTimezone {
        value: String,
    },
    ObserveTimezone {
        value: String,
        evidence_ref: String,
    },
    LearnInterpretation {
        surface: String,
        basis_expression: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequest {
    pub operation_id: OperationId,
    pub expected_commit_revision: RevisionRef,
    pub expected_memory_revision: String,
    pub subject_ref: ContextSubject,
    pub scope: ContextScope,
    pub operation: ContextOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextReceipt {
    pub commit: CommitReceipt,
    pub input_revision: String,
    pub output_revision: String,
    pub changed: bool,
    pub observation_ref: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextResult {
    pub(crate) request_digest: String,
    pub(crate) output_revision: String,
    pub(crate) changed: bool,
    pub(crate) observation_ref: Option<String>,
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>, ContextError> {
    serde_json::to_vec(value).map_err(|e| ContextError::InvalidContext(e.to_string()))
}
fn invalid(message: &str) -> ContextError {
    ContextError::InvalidContext(message.into())
}
fn text(value: &str) -> Result<(), ContextError> {
    if value.trim().is_empty() || value.len() > 16_384 || value.contains('\0') {
        return Err(invalid("empty, excessive or invalid semantic value"));
    }
    Ok(())
}
impl ContextStore {
    pub fn publish(
        path: impl AsRef<Path>,
        request: &ContextRequest,
    ) -> Result<ContextReceipt, ContextError> {
        Self::with_locked_engine(path, |engine| engine.apply(request))
    }
    pub fn publish_promotion(
        path: impl AsRef<Path>,
        candidate: ContextDefaultCandidate,
        request: &PromotionRequest,
    ) -> Result<PromotionReceipt, ContextError> {
        Self::with_locked_engine(path, |engine| engine.promote(candidate, request))
    }
}
impl ContextEngine {
    pub fn apply(&mut self, request: &ContextRequest) -> Result<ContextReceipt, ContextError> {
        let backend = SnapshotBackend::new(self.store.commit_state.clone())?;
        let commits = CommitStore::new(&backend);
        let request_digest = Digest::object_digest(&encode(request)?).to_hex();
        if let Some(receipt) = commits.receipt(DOMAIN, &request.operation_id)? {
            let original = commits
                .committed(&receipt.commit_ref)?
                .ok_or(Failure::Corrupt)?;
            let mut result: ContextResult =
                serde_json::from_slice(original.payload()).map_err(|_| Failure::Corrupt)?;
            // Rebuild identity with the supplied formal request. Foundation itself
            // distinguishes exact replay from operation reuse, before the current head.
            result.request_digest = request_digest;
            let replay = CommitIntent {
                domain: DOMAIN.into(),
                operation_id: request.operation_id.clone(),
                expected_revision: request.expected_commit_revision.clone(),
                parents: request
                    .expected_commit_revision
                    .commit
                    .iter()
                    .cloned()
                    .collect(),
                payload: encode(&result)?,
            }
            .prepare()?;
            return match commits.commit(&replay) {
                CommitOutcome::NoChange(commit) => Ok(ContextReceipt {
                    commit,
                    input_revision: request.expected_memory_revision.clone(),
                    output_revision: result.output_revision,
                    changed: result.changed,
                    observation_ref: result.observation_ref,
                }),
                CommitOutcome::Conflict(conflict) => Err(ContextError::CommitConflict(conflict)),
                CommitOutcome::Rejected(error) => Err(error.into()),
                CommitOutcome::Failure(error) => Err(error.into()),
                CommitOutcome::Committed(_) => Err(Failure::Corrupt.into()),
            };
        }
        text(&request.subject_ref.id)?;
        text(&request.subject_ref.class)?;
        if self
            .subject_ref
            .as_ref()
            .is_some_and(|subject| subject != &request.subject_ref)
        {
            return Err(ObservationError::SubjectMismatch.into());
        }
        if self.memory_revision()? != request.expected_memory_revision {
            return Err(ContextError::RevisionConflict);
        }
        let mut next = self.clone();
        next.set_subject(request.subject_ref.clone());
        let observation_ref = next.mutate_context(request)?;
        next.store.validate()?;
        let output_revision = next.memory_revision()?;
        let changed = output_revision != request.expected_memory_revision;
        let prepared = CommitIntent {
            domain: DOMAIN.into(),
            operation_id: request.operation_id.clone(),
            expected_revision: request.expected_commit_revision.clone(),
            parents: request
                .expected_commit_revision
                .commit
                .iter()
                .cloned()
                .collect(),
            payload: encode(&ContextResult {
                request_digest,
                output_revision: output_revision.clone(),
                changed,
                observation_ref: observation_ref.clone(),
            })?,
        }
        .prepare()?;
        let commit = match commits.commit(&prepared) {
            CommitOutcome::Committed(receipt) => receipt,
            CommitOutcome::Conflict(conflict) => {
                return Err(ContextError::CommitConflict(conflict));
            }
            CommitOutcome::Rejected(error) => return Err(error.into()),
            CommitOutcome::Failure(error) => return Err(error.into()),
            CommitOutcome::NoChange(_) => return Err(Failure::Corrupt.into()),
        };
        next.store.commit_state = backend.into_state()?;
        self.store = next.store;
        Ok(ContextReceipt {
            commit,
            input_revision: request.expected_memory_revision.clone(),
            output_revision,
            changed,
            observation_ref,
        })
    }

    fn mutate_context(&mut self, request: &ContextRequest) -> Result<Option<String>, ContextError> {
        let subject = &request.subject_ref;
        let scope = request.scope;
        match &request.operation {
            ContextOperation::Retain { observation } => {
                if &observation.context.subject != subject {
                    return Err(ObservationError::SubjectMismatch.into());
                }
                if observation.context.context_scope != scope {
                    return Err(invalid("observation scope mismatch"));
                }
                self.read_current(
                    &observation.context.context_refs,
                    subject,
                    scope,
                    observation.context.at,
                )
                .map_err(|e| invalid(&e.to_string()))?;
                return self
                    .retain_semantics(observation.clone())
                    .map(|receipt| Some(receipt.reference));
            }
            ContextOperation::Discard { reference } => {
                self.clear_target(
                    &ContextReference::Observation(reference.clone()),
                    subject,
                    scope,
                )?;
            }
            ContextOperation::Clear { targets } => {
                if targets.is_empty()
                    || targets.len() > 64
                    || targets
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != targets.len()
                {
                    return Err(invalid("invalid clear targets"));
                }
                for target in targets {
                    self.clear_target(target, subject, scope)?;
                }
            }
            ContextOperation::ClearShortTerm => {
                self.store.short_term.retain(|record| {
                    &record.context.subject != subject || record.context.context_scope != scope
                });
                self.store
                    .observations
                    .retain(|record| &record.subject != subject || record.scope != scope);
                self.store.decided_candidates.retain(|_, record| {
                    &record.subject_ref != subject
                        || record.candidate.inferred_scope != scope
                        || record.disposition == CandidateDisposition::Promoted
                });
                for binding in [
                    &mut self.store.active_timezone,
                    &mut self.store.session_timezone,
                ] {
                    if binding
                        .as_ref()
                        .is_some_and(|record| &record.subject == subject && record.scope == scope)
                    {
                        *binding = None;
                    }
                }
            }
            ContextOperation::BindTimezone { value } => {
                text(value)?;
                if !matches!(scope, ContextScope::Session | ContextScope::CurrentRequest) {
                    return Err(invalid(
                        "timezone binding requires session or current request scope",
                    ));
                }
                let binding = self.binding(
                    subject.clone(),
                    value,
                    ContextSource::ExplicitUserStatement,
                    ContextAuthority::Explicit,
                    scope,
                );
                let slot = if scope == ContextScope::Session {
                    &mut self.store.session_timezone
                } else {
                    &mut self.store.active_timezone
                };
                if slot
                    .as_ref()
                    .is_some_and(|record| &record.subject != subject)
                {
                    return Err(ObservationError::SubjectMismatch.into());
                }
                *slot = Some(binding);
            }
            ContextOperation::ObserveTimezone {
                value,
                evidence_ref,
            } => {
                text(value)?;
                text(evidence_ref)?;
                if scope != ContextScope::PersistentAgentDefault {
                    return Err(invalid("timezone observation scope mismatch"));
                }
                let id = Digest::object_digest(&encode(&(subject, scope, value, evidence_ref))?)
                    .to_hex();
                if !self.store.observations.iter().any(|record| record.id == id) {
                    self.store.observations.push(ContextObservation {
                        id,
                        subject: subject.clone(),
                        kind: ContextKind::TimeZone,
                        value: value.clone(),
                        source: ContextSource::ExplicitUserStatement,
                        scope,
                        evidence: EvidenceRef {
                            reference: evidence_ref.clone(),
                            detail: "explicit timezone confirmation".into(),
                        },
                    });
                }
            }
            ContextOperation::LearnInterpretation {
                surface,
                basis_expression,
            } => {
                text(surface)?;
                text(basis_expression)?;
                // A subject/scope-qualified key prevents one subject's explicit
                // interpretation from silently replacing another subject's rule.
                let key = Digest::object_digest(&encode(&(subject, scope, surface))?).to_hex();
                self.store
                    .interpretation_rules
                    .insert(key, basis_expression.clone());
            }
        }
        Ok(None)
    }

    fn clear_target(
        &mut self,
        target: &ContextReference,
        subject: &ContextSubject,
        scope: ContextScope,
    ) -> Result<(), ContextError> {
        match target {
            ContextReference::Observation(reference) => {
                let record = self.observation(reference, subject)?;
                if record.context.context_scope != scope {
                    return Err(invalid("clear scope mismatch"));
                }
                self.store
                    .short_term
                    .retain(|record| &record.reference != reference);
            }
            ContextReference::Decision(reference) => {
                let record = self
                    .store
                    .decided_candidates
                    .get(reference)
                    .ok_or_else(|| invalid("missing clear target"))?;
                if &record.subject_ref != subject
                    || record.candidate.inferred_scope != scope
                    || record.disposition == CandidateDisposition::Promoted
                {
                    return Err(invalid("invalid short-term clear target"));
                }
                self.store.decided_candidates.remove(reference);
            }
            ContextReference::Active | ContextReference::Session => {
                let slot = if *target == ContextReference::Active {
                    &mut self.store.active_timezone
                } else {
                    &mut self.store.session_timezone
                };
                let record = slot
                    .as_ref()
                    .ok_or_else(|| invalid("missing clear target"))?;
                if &record.subject != subject || record.scope != scope {
                    return Err(invalid("clear subject or scope mismatch"));
                }
                *slot = None;
            }
            ContextReference::Default(_) => {
                return Err(invalid(
                    "long-term assertions require an explicit semantic correction",
                ));
            }
        }
        Ok(())
    }
}
