//! owner-only semantic validation before any persistence.
//! Uses existing ContextEngine promotion/correction; does not authorize a caller.
use super::*;
use sem_lang_context::{
    ContextDefaultCandidate, ContextError, ContextReference, ContextStore, ObservationError,
    PromotionOperation, PromotionReceipt, PromotionRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticSelection {
    pub residual_ref: String,
    pub candidate: ContextDefaultCandidate,
    pub operation: PromotionOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    pub subject: ContextSubject,
    pub expected_semantic_revision: String,
    pub resolution_ref: String,
    pub decision_ref: String,
    pub selections: Vec<SemanticSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ConfirmationError {
    #[error("commit conflict: {0:?}")]
    CommitConflict(
        #[serde(with = "sem_lang_context::commit_wire::Conflict")] sem_lang_context::CommitConflict,
    ),
    #[error(transparent)]
    Commit(
        #[serde(with = "sem_lang_context::commit_wire::Failure")] sem_lang_context::CommitFailure,
    ),
    #[error(transparent)]
    CommitRejected(
        #[serde(with = "sem_lang_context::commit_wire::Rejection")]
        sem_lang_context::CommitRejection,
    ),
    #[error("context busy after {wait_millis}ms")]
    Busy { wait_millis: u64 },
    #[error("context already initialized")]
    AlreadyExists,
    #[error("context repository unavailable ({kind}): {detail}")]
    RepositoryUnavailable { kind: String, detail: String },
    #[error("context persistence: {0}")]
    Persistence(String),
    #[error(transparent)]
    Repository(sem_lang_context::ContextRepositoryError),
    #[error("invalid semantic selection input")]
    InvalidInput,
    #[error("semantic selection subject mismatch")]
    SubjectMismatch,
    #[error("stale semantic selection revision")]
    StaleRevision,
    #[error("semantic residual reference mismatch")]
    UnknownResidual,
    #[error("duplicate semantic selection")]
    DuplicateSelection,
    #[error("selection does not match the adopted context association")]
    InapplicableSelection,
    #[error("selection conflicts with resolved meaning or constraints")]
    SelectionConflict,
    #[error("invalid semantic candidate")]
    InvalidCandidate,
    #[error("explicit correction is required")]
    CorrectionRequired,
    #[error("semantic memory invariant violated")]
    MemoryInvariant,
    #[error(transparent)]
    Observation(#[from] ObservationError),
    #[error(transparent)]
    Processing(#[from] ProcessingError),
}

impl From<ContextError> for ConfirmationError {
    fn from(error: ContextError) -> Self {
        match error {
            ContextError::CommitConflict(error) => Self::CommitConflict(error),
            ContextError::Observation(error) => Self::Observation(error),
            ContextError::RevisionConflict => Self::StaleRevision,
            ContextError::CorrectionRequired => Self::CorrectionRequired,
            ContextError::EmptyPromotionReference => Self::InvalidInput,
            ContextError::CandidateConflict
            | ContextError::UnsafePromotion
            | ContextError::InvalidContext(_)
            | ContextError::Unresolved(_) => Self::InvalidCandidate,
            ContextError::Busy { wait_millis } => Self::Busy { wait_millis },
            ContextError::Commit(error) => Self::Commit(error),
            ContextError::CommitRejected(error) => Self::CommitRejected(error),
            ContextError::AlreadyExists => Self::AlreadyExists,
            ContextError::RepositoryUnavailable { kind, detail } => {
                Self::RepositoryUnavailable { kind, detail }
            }
            ContextError::Persistence(error) => Self::Persistence(error),
            ContextError::Repository(error) => Self::Repository(error),
        }
    }
}

/// A validated speculative transition, not an independently live memory session.
/// No setters or deserializer: callers cannot construct this as a bypass.
pub struct ValidatedSelection {
    memory: ContextStore,
    context: CompileContext,
    outcome: ProcessingOutcome,
    receipts: Vec<PromotionReceipt>,
}
impl ValidatedSelection {
    pub fn memory(&self) -> &ContextStore {
        &self.memory
    }
    pub fn context(&self) -> &CompileContext {
        &self.context
    }
    pub fn outcome(&self) -> &ProcessingOutcome {
        &self.outcome
    }
    pub fn receipts(&self) -> &[PromotionReceipt] {
        &self.receipts
    }
}

impl SemanticResolution {
    /// Content identity, never a vector position or translated display label.
    pub fn issue_reference(&self, issue: &ResolutionIssue) -> Result<String, ProcessingError> {
        if self.issues.iter().filter(|v| *v == issue).count() != 1 {
            return Err(ProcessingError::InvalidReference);
        }
        Digest::from_serializable(1, &(self.identity.as_str(), issue))
            .map(|digest| digest.to_hex())
            .map_err(|_| ProcessingError::ContractViolation)
    }
}

fn result(outcome: &ProcessingOutcome) -> &SemanticResolution {
    match outcome {
        ProcessingOutcome::Resolved(v) | ProcessingOutcome::NeedsResolution(v) => v,
    }
}

impl CurrentStateCompiler<'_> {
    /// Evaluate the exact combination using the canonical compiler. Callers may
    /// authenticate/select a proposal but must not replicate this meaning logic.
    pub fn validate_selection(
        &self,
        input: &SurfaceInput,
        context: &CompileContext,
        request: &SelectionRequest,
    ) -> Result<ValidatedSelection, ConfirmationError> {
        if request.subject != context.subject {
            return Err(ConfirmationError::SubjectMismatch);
        }
        if request.expected_semantic_revision != context.semantic_revision
            || self.semantic_revision()? != request.expected_semantic_revision
        {
            return Err(ConfirmationError::StaleRevision);
        }
        if !token(&request.decision_ref)
            || request.selections.is_empty()
            || request.selections.len() > 64
        {
            return Err(ConfirmationError::InvalidInput);
        }
        // Bound all variable-size proposal fields before cloning current memory
        // or asking the existing operation to serialize a candidate.
        for selection in &request.selections {
            if selection.residual_ref.len() != 64 {
                return Err(ConfirmationError::UnknownResidual);
            }
            validate_candidate(&selection.candidate)?;
            match &selection.operation {
                PromotionOperation::Replace { candidate } => validate_candidate(candidate)?,
                PromotionOperation::CorrectLongTerm { assertion_ref } if !token(assertion_ref) => {
                    return Err(ConfirmationError::InvalidInput);
                }
                PromotionOperation::SelectShortTerm
                | PromotionOperation::PromoteLongTerm
                | PromotionOperation::Retain
                | PromotionOperation::Reject
                | PromotionOperation::CorrectLongTerm { .. } => {}
            }
        }
        let before = self.compile(input, context)?;
        let before = result(&before);
        if request.resolution_ref != before.identity {
            return Err(ConfirmationError::UnknownResidual);
        }
        let expression = before
            .expression
            .as_ref()
            .ok_or(ConfirmationError::InapplicableSelection)?;
        let (mut patterns, mut issues, mut constraints, mut associations) =
            (vec![], vec![], vec![], vec![]);
        self.collect_patterns(
            expression,
            &mut patterns,
            &mut issues,
            &mut constraints,
            &mut associations,
            0,
        )?;
        let mut selected = BTreeSet::new();
        let mut variables = BTreeSet::new();
        let mut memory = ContextEngine::new(self.context.store.clone());
        let mut next_context = context.clone();
        let mut receipts = Vec::new();
        for selection in &request.selections {
            if !selected.insert(&selection.residual_ref) {
                return Err(ConfirmationError::DuplicateSelection);
            }
            let issue = before
                .issues
                .iter()
                .find(|issue| {
                    before
                        .issue_reference(issue)
                        .is_ok_and(|reference| reference == selection.residual_ref)
                })
                .ok_or(ConfirmationError::UnknownResidual)?;
            let variable = issue
                .variable
                .as_deref()
                .ok_or(ConfirmationError::InapplicableSelection)?;
            if !variables.insert(variable) {
                return Err(ConfirmationError::DuplicateSelection);
            }
            let applicable: Vec<_> = associations
                .iter()
                .filter(|(_, name)| name == variable)
                .collect();
            let candidate = &selection.candidate;
            if applicable.is_empty()
                || applicable.iter().any(|(association, _)| {
                    association.kind != candidate.kind
                        || !association.scope.permits(context.context_scope)
                        || association.validity.applicable(context.at) != Some(true)
                })
                || !candidate.inferred_scope.permits(context.context_scope)
                || candidate.inferred_validity.applicable(context.at) != Some(true)
                || (!issue.candidates.is_empty()
                    && !issue
                        .candidates
                        .contains(&Value::Text(candidate.value.clone())))
            {
                return Err(ConfirmationError::InapplicableSelection);
            }
            let candidate_ref = candidate.reference()?;
            // Each selected residual has one formal operation/receipt. The
            // aggregate decision is deterministic, never an array-index ID.
            let decision_ref = if request.selections.len() == 1 {
                request.decision_ref.clone()
            } else {
                Digest::from_serializable(1, &(&request.decision_ref, &selection.residual_ref))
                    .map_err(|_| ConfirmationError::InvalidInput)?
                    .to_hex()
            };
            let receipt = memory.promote(
                candidate.clone(),
                &PromotionRequest {
                    subject_ref: context.subject.clone(),
                    candidate_ref: candidate_ref.clone(),
                    expected_memory_revision: memory.memory_revision()?,
                    expected_commit_revision: memory.commit_revision()?,
                    decision_ref,
                    operation: selection.operation.clone(),
                },
            )?;
            match &selection.operation {
                PromotionOperation::SelectShortTerm => {
                    let reference = ContextReference::Decision(candidate_ref);
                    if !next_context.context_refs.contains(&reference) {
                        next_context.context_refs.push(reference);
                    }
                }
                PromotionOperation::PromoteLongTerm
                | PromotionOperation::CorrectLongTerm { .. } => {
                    if let PromotionOperation::CorrectLongTerm { assertion_ref } =
                        &selection.operation
                    {
                        next_context
                            .context_refs
                            .retain(|v| v != &ContextReference::Default(assertion_ref.clone()));
                    }
                    let assertion = memory
                        .store
                        .persistent_defaults
                        .iter()
                        .find(|v| {
                            v.subject == context.subject
                                && v.kind == candidate.kind
                                && v.scope == candidate.inferred_scope
                                && v.value == candidate.value
                                && v.validity == candidate.inferred_validity
                        })
                        .ok_or(ConfirmationError::MemoryInvariant)?;
                    let reference = ContextReference::Default(assertion.id.clone());
                    if !next_context.context_refs.contains(&reference) {
                        next_context.context_refs.push(reference);
                    }
                }
                PromotionOperation::Retain
                | PromotionOperation::Reject
                | PromotionOperation::Replace { .. } => {}
            }
            receipts.push(receipt);
        }
        let compiler = CurrentStateCompiler::new(self.compiled_language, self.state, &memory);
        next_context.semantic_revision = compiler.semantic_revision()?;
        let after = compiler.compile(input, &next_context)?;
        let resolved = result(&after);
        if before.concept != resolved.concept
            || before.expression != resolved.expression
            || before
                .bindings
                .iter()
                .any(|binding| !resolved.bindings.contains(binding))
            || resolved
                .issues
                .iter()
                .any(|issue| !before.issues.contains(issue))
        {
            return Err(ConfirmationError::SelectionConflict);
        }
        for selection in &request.selections {
            if matches!(
                selection.operation,
                PromotionOperation::SelectShortTerm
                    | PromotionOperation::PromoteLongTerm
                    | PromotionOperation::CorrectLongTerm { .. }
            ) {
                let issue = before
                    .issues
                    .iter()
                    .find(|issue| {
                        before
                            .issue_reference(issue)
                            .is_ok_and(|id| id == selection.residual_ref)
                    })
                    .ok_or(ConfirmationError::UnknownResidual)?;
                if !resolved.bindings.iter().any(|binding| {
                    Some(binding.variable.id.as_str()) == issue.variable.as_deref()
                        && binding.value == Value::Text(selection.candidate.value.clone())
                }) {
                    return Err(ConfirmationError::SelectionConflict);
                }
            }
        }
        Ok(ValidatedSelection {
            memory: memory.store,
            context: next_context,
            outcome: after,
            receipts,
        })
    }
}

fn validate_candidate(candidate: &ContextDefaultCandidate) -> Result<(), ConfirmationError> {
    if candidate.supporting_evidence.len() > 64
        || candidate.counter_evidence.len() > 64
        || candidate
            .supporting_evidence
            .iter()
            .chain(&candidate.counter_evidence)
            .any(|e| !token(&e.reference) || e.detail.len() > 512 || e.detail.contains('\0'))
        || !candidate.inferred_validity.valid()
        || candidate.value.contains('\0')
    {
        return Err(ConfirmationError::InvalidCandidate);
    }
    candidate.reference()?;
    Ok(())
}
