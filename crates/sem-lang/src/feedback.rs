//! formal feedback plans over the exact canonical memory.
//! Authentication, reference CAS and UI control stay with the calling application.
use crate::{
    CompileContext, Engine,
    current::ProcessingOutcome,
    restoration::{self, CheckpointPlan, RestorationError, RestorationInput},
};
use sem_lang_context::{
    ContextOperation, ContextReference, ContextRequest, ContextSubject, PromotionReceipt,
};
pub use sem_lang_frontend::current::{ConfirmationError, SelectionRequest, SemanticSelection};
use sem_lang_repository::Repository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionReference {
    pub reference: String,
    pub consequence_ref: String,
    pub observation_ref: String,
    pub subject: ContextSubject,
    pub semantic_revision: String,
    pub resolution_ref: String,
    pub decision_ref: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionDocument {
    schema: String,
    restoration: RestorationInput,
    read: RetainedRead,
    request: SelectionRequest,
    consequence_ref: String,
}

/// An immutable owner proposal, not adopted memory or permission. It can only
/// be created after semantic validation; consumers persist the returned ref.
pub struct OptionPlan {
    reference: OptionReference,
    bytes: Vec<u8>,
}
impl OptionPlan {
    pub fn reference(&self) -> &OptionReference {
        &self.reference
    }
    pub fn persist<R: Repository>(
        self,
        repository: &mut R,
    ) -> Result<OptionReference, FeedbackError> {
        let stored = repository
            .put_object(self.bytes)
            .map_err(RestorationError::from)?;
        if stored != self.reference.reference {
            return Err(ConfirmationError::MemoryInvariant.into());
        }
        Ok(self.reference)
    }
}

fn consequence(plan: &FeedbackPlan) -> Result<String, ConfirmationError> {
    sem_lang_core::Digest::from_serializable(
        1,
        &(
            plan.checkpoint.restoration(),
            &plan.context,
            &plan.outcome,
            &plan.receipts,
        ),
    )
    .map(|digest| digest.to_hex())
    .map_err(|_| ConfirmationError::MemoryInvariant)
}

pub fn prepare_option<R: Repository>(
    repository: &R,
    input: &RestorationInput,
    read: &RetainedRead,
    request: &SelectionRequest,
) -> Result<OptionPlan, FeedbackError> {
    let plan = select(repository, input, read, request)?;
    let consequence_ref = consequence(&plan)?;
    let document = OptionDocument {
        schema: "sem-lang://confirmation/option/v1".into(),
        restoration: input.clone(),
        read: read.clone(),
        request: request.clone(),
        consequence_ref: consequence_ref.clone(),
    };
    let bytes = serde_json::to_vec(&document).map_err(|_| ConfirmationError::InvalidInput)?;
    if bytes.len() > 65_536 {
        return Err(ConfirmationError::InvalidInput.into());
    }
    let reference = OptionReference {
        reference: sem_lang_core::Digest::object_digest(&bytes).to_hex(),
        consequence_ref,
        observation_ref: read.observation_ref.clone(),
        subject: request.subject.clone(),
        semantic_revision: request.expected_semantic_revision.clone(),
        resolution_ref: request.resolution_ref.clone(),
        decision_ref: request.decision_ref.clone(),
    };
    Ok(OptionPlan { reference, bytes })
}

pub fn select_option<R: Repository>(
    repository: &R,
    input: &RestorationInput,
    reference: &OptionReference,
) -> Result<FeedbackPlan, FeedbackError> {
    if reference.reference.len() != 64 {
        return Err(ConfirmationError::InvalidInput.into());
    }
    let bytes = repository
        .get_object(&reference.reference)
        .map_err(RestorationError::from)?
        .ok_or(ConfirmationError::UnknownResidual)?;
    if bytes.len() > 65_536
        || sem_lang_core::Digest::object_digest(&bytes).to_hex() != reference.reference
    {
        return Err(ConfirmationError::UnknownResidual.into());
    }
    let document: OptionDocument =
        serde_json::from_slice(&bytes).map_err(|_| ConfirmationError::InvalidInput)?;
    if document.schema != "sem-lang://confirmation/option/v1"
        || &document.restoration != input
        || document.read.observation_ref != reference.observation_ref
        || document.read.subject != reference.subject
        || document.request.subject != reference.subject
        || document.request.resolution_ref != reference.resolution_ref
        || document.request.expected_semantic_revision != reference.semantic_revision
        || document.request.decision_ref != reference.decision_ref
        || document.consequence_ref != reference.consequence_ref
    {
        return Err(ConfirmationError::UnknownResidual.into());
    }
    let plan = select(repository, input, &document.read, &document.request)?;
    if consequence(&plan)? != reference.consequence_ref {
        return Err(ConfirmationError::SelectionConflict.into());
    }
    Ok(plan)
}

#[derive(Debug, Serialize, thiserror::Error)]
pub enum FeedbackError {
    #[error(transparent)]
    Confirmation(#[from] ConfirmationError),
    #[error(transparent)]
    Restoration(#[from] RestorationError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedRead {
    pub observation_ref: String,
    pub subject: ContextSubject,
    pub semantic_revision: String,
    pub context_refs: Vec<ContextReference>,
}

/// Caller stores only restoration/control references. The bounded checkpoint
/// serialization remains owner-owned until the application's commit fence.
pub struct FeedbackPlan {
    pub checkpoint: CheckpointPlan,
    pub observation_ref: String,
    pub context: CompileContext,
    pub outcome: ProcessingOutcome,
    pub receipts: Vec<PromotionReceipt>,
}

fn engine<R: Repository>(
    repository: &R,
    input: &RestorationInput,
) -> Result<Engine, RestorationError> {
    Engine::restore_view(repository, input.clone())
}

pub fn retain<R: Repository>(
    repository: &R,
    input: &RestorationInput,
    request: ContextRequest,
) -> Result<FeedbackPlan, FeedbackError> {
    let mut engine = engine(repository, input)?;
    let ContextOperation::Retain { observation } = &request.operation else {
        return Err(ConfirmationError::InvalidInput.into());
    };
    // Validate the original observation and inspected revision before retention.
    engine
        .compile_surface(&observation.input, &observation.context)
        .map_err(ConfirmationError::from)?;
    let mut surface = observation.input.clone();
    let mut context = observation.context.clone();
    let receipt = engine
        .context_mut()
        .apply(&request)
        .map_err(ConfirmationError::from)?;
    // DiscardAfterCompile still processes the ephemeral input, but a later
    // resume correctly reports SurfaceUnavailable. No fallback surface is saved.
    let observation_ref = receipt
        .observation_ref
        .ok_or(ConfirmationError::MemoryInvariant)?;
    surface.reference = observation_ref.clone();
    context.semantic_revision = engine
        .current_compiler()
        .map_err(ConfirmationError::from)?
        .semantic_revision()
        .map_err(ConfirmationError::from)?;
    let outcome = engine
        .compile_surface(&surface, &context)
        .map_err(ConfirmationError::from)?;
    let checkpoint = restoration::prepare_checkpoint(
        repository,
        &input.definitions,
        &input.bindings,
        &engine.context().store,
        &engine.runtime().state,
    )?;
    Ok(FeedbackPlan {
        checkpoint,
        observation_ref,
        context,
        outcome,
        receipts: vec![],
    })
}

fn retained(
    engine: &Engine,
    request: &RetainedRead,
) -> Result<(crate::SurfaceInput, CompileContext), ConfirmationError> {
    if engine.current_compiler()?.semantic_revision()? != request.semantic_revision {
        return Err(ConfirmationError::StaleRevision);
    }
    let observation = engine
        .context()
        .observation(&request.observation_ref, &request.subject)?;
    let surface = observation.surface_input()?;
    let mut context = observation.context.clone();
    // Explicitly selected current refs, not mutable-head following or fallback.
    context.semantic_revision = request.semantic_revision.clone();
    context.context_refs = request.context_refs.clone();
    Ok((surface, context))
}

pub fn resume<R: Repository>(
    repository: &R,
    input: &RestorationInput,
    request: &RetainedRead,
) -> Result<ProcessingOutcome, FeedbackError> {
    let engine = engine(repository, input)?;
    let (surface, context) = retained(&engine, request)?;
    engine
        .compile_surface(&surface, &context)
        .map_err(|e| ConfirmationError::from(e).into())
}

pub fn select<R: Repository>(
    repository: &R,
    input: &RestorationInput,
    read: &RetainedRead,
    request: &SelectionRequest,
) -> Result<FeedbackPlan, FeedbackError> {
    let engine = engine(repository, input)?;
    let (surface, context) = retained(&engine, read)?;
    let compiler = engine.current_compiler().map_err(ConfirmationError::from)?;
    let selected = compiler.validate_selection(&surface, &context, request)?;
    let checkpoint = restoration::prepare_checkpoint(
        repository,
        &input.definitions,
        &input.bindings,
        selected.memory(),
        &engine.runtime().state,
    )?;
    Ok(FeedbackPlan {
        checkpoint,
        observation_ref: read.observation_ref.clone(),
        context: selected.context().clone(),
        outcome: selected.outcome().clone(),
        receipts: selected.receipts().to_vec(),
    })
}
