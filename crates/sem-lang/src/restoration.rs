//! exact restoration and validated immutable checkpoint plans.
//! No head following, provider access, authorization or additional memory store.
use crate::admission::Admission;
pub use crate::admission::SemanticReferenceError;
use crate::{Engine, EngineBuildError};
use sem_lang_context::ContextStore;
use sem_lang_core::{ApplicationReceipt, Digest, RuntimeState, SemanticPacket};
use sem_lang_memory::{
    bindings::{self, AdoptionState, RevisionedBinding},
    definitions,
};
use sem_lang_repository::{Repository, RepositoryError, RepositorySnapshot};
use sem_lang_runtime::{PacketApplyError, PacketReducer, PacketRequest};
use sem_lang_vocab::VocabularyRegistry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_BYTES: usize = 1_048_576;
const SCHEMA: &str = "sem-lang://runtime/checkpoint/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionReference {
    pub namespace: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorationInput {
    pub definitions: Vec<RevisionReference>,
    pub bindings: Vec<RevisionReference>,
    /// Content-addressed ContextStore object, not a mutable head or profile.
    pub memory: String,
    /// Content-bound snapshot in the existing SnapshotStore.
    pub runtime_snapshot: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema: String,
    definitions: Vec<RevisionReference>,
    bindings: Vec<RevisionReference>,
    memory: String,
    state: RuntimeState,
}

#[derive(Debug, Serialize, thiserror::Error)]
pub enum RestorationError {
    #[error(transparent)]
    SemanticReference(#[from] SemanticReferenceError),
    #[error(transparent)]
    Context(#[from] sem_lang_context::ContextError),
    #[error("missing {kind} reference: {reference}")]
    MissingReference {
        kind: &'static str,
        reference: String,
    },
    #[error("corrupt {kind} object: {detail}")]
    CorruptObject { kind: &'static str, detail: String },
    #[error("digest mismatch: expected {expected}, actual {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("wrong object kind: {0}")]
    WrongObjectKind(&'static str),
    #[error("revision mismatch: {0}")]
    RevisionMismatch(&'static str),
    #[error("incompatible restoration reference set")]
    IncompatibleReferences,
    #[error("invalid adoption revision or parent: {0}")]
    InvalidRevision(RepositoryError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Build(
        #[from]
        #[serde(serialize_with = "sem_lang_repository::commit_wire::display_error")]
        EngineBuildError,
    ),
    #[error(transparent)]
    Application(#[from] PacketApplyError),
}

fn repository_error(kind: &'static str, error: RepositoryError) -> RestorationError {
    match error {
        RepositoryError::MissingObject(reference) | RepositoryError::MissingRevision(reference) => {
            RestorationError::MissingReference { kind, reference }
        }
        RepositoryError::DigestMismatch { expected, actual } => {
            RestorationError::DigestMismatch { expected, actual }
        }
        RepositoryError::CorruptObject(detail) => RestorationError::CorruptObject { kind, detail },
        error @ RepositoryError::Conflict(_) => RestorationError::InvalidRevision(error),
        error @ (RepositoryError::Io(_)
        | RepositoryError::CommitConflict(_)
        | RepositoryError::CommitFailure(_)
        | RepositoryError::CommitRejected(_)) => RestorationError::Repository(error),
    }
}

fn adopted<R: Repository>(
    repository: &R,
    definition_refs: &[RevisionReference],
    binding_refs: &[RevisionReference],
) -> Result<(VocabularyRegistry, Vec<RevisionedBinding>), RestorationError> {
    if definition_refs.is_empty() || definition_refs.len() > 64 || binding_refs.len() > 64 {
        return Err(RestorationError::IncompatibleReferences);
    }
    let mut registry = VocabularyRegistry::new();
    let mut pinned = BTreeSet::new();
    for reference in definition_refs {
        if !reference.revision.starts_with("definitions-") {
            return Err(RestorationError::WrongObjectKind("definition"));
        }
        if !pinned.insert((&reference.namespace, &reference.revision)) {
            return Err(RestorationError::IncompatibleReferences);
        }
        let record =
            definitions::read_revision(repository, &reference.namespace, &reference.revision)
                .map_err(|e| repository_error("definition", e))?;
        for module in record.modules {
            registry
                .register(module)
                .map_err(EngineBuildError::Vocabulary)?;
        }
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for reference in binding_refs {
        if !reference.revision.starts_with("bindings-") {
            return Err(RestorationError::WrongObjectKind("binding"));
        }
        if !seen.insert((&reference.namespace, &reference.revision)) {
            return Err(RestorationError::IncompatibleReferences);
        }
        let record = bindings::read_revision(repository, &reference.namespace, &reference.revision)
            .map_err(|e| repository_error("binding", e))?;
        if record.state != AdoptionState::Adopted {
            return Err(RestorationError::IncompatibleReferences);
        }
        for item in record.bindings {
            for definition in
                std::iter::once(&item.binding.semantic_ref).chain(&item.binding.constraints)
            {
                if !pinned.contains(&(&definition.namespace, &definition.revision)) {
                    return Err(RestorationError::IncompatibleReferences);
                }
            }
            result.push(item);
        }
    }
    Ok((registry, result))
}

fn memory<R: Repository>(
    repository: &R,
    reference: &str,
) -> Result<ContextStore, RestorationError> {
    ContextStore::load_from_repository(repository, &reference.to_owned()).map_err(|error| {
        match error {
            sem_lang_context::ContextError::Repository(source) => {
                repository_error("memory", source)
            }
            error => RestorationError::Context(error),
        }
    })
}

pub(crate) struct Restored {
    pub registry: VocabularyRegistry,
    pub bindings: Vec<RevisionedBinding>,
    pub context: ContextStore,
    pub state: RuntimeState,
}

/// Owner-only reference reconstruction from an exact immutable checkpoint.
/// This does not follow a mutable head or install a missing dictionary.
pub(crate) fn checkpoint_input<R: Repository>(
    repository: &R,
    id: &str,
) -> Result<RestorationInput, RestorationError> {
    let snapshot = repository.load_snapshot(&id.to_owned())?;
    if snapshot.serialized.len() > MAX_BYTES {
        return Err(RestorationError::WrongObjectKind("runtime"));
    }
    let checkpoint: Checkpoint = serde_json::from_slice(&snapshot.serialized).map_err(|error| {
        RestorationError::CorruptObject {
            kind: "runtime",
            detail: error.to_string(),
        }
    })?;
    let input = RestorationInput {
        definitions: checkpoint.definitions,
        bindings: checkpoint.bindings,
        memory: checkpoint.memory,
        runtime_snapshot: id.into(),
    };
    read(repository, &input)?;
    Ok(input)
}

pub(crate) fn read<R: Repository>(
    repository: &R,
    input: &RestorationInput,
) -> Result<Restored, RestorationError> {
    let (registry, bindings) = adopted(repository, &input.definitions, &input.bindings)?;
    let context = memory(repository, &input.memory)?;
    if !input.runtime_snapshot.starts_with("runtime-") {
        return Err(RestorationError::WrongObjectKind("runtime"));
    }
    let snapshot = repository
        .load_snapshot(&input.runtime_snapshot)
        .map_err(|e| repository_error("runtime", e))?;
    if snapshot.revision != input.runtime_snapshot {
        return Err(RestorationError::RevisionMismatch("runtime"));
    }
    if snapshot.serialized.len() > MAX_BYTES {
        return Err(RestorationError::CorruptObject {
            kind: "runtime",
            detail: "capacity exceeded".into(),
        });
    }
    let actual = Digest::object_digest(&snapshot.serialized);
    if snapshot.state_digest != actual
        || input.runtime_snapshot != format!("runtime-{}", actual.to_hex())
    {
        return Err(RestorationError::DigestMismatch {
            expected: input.runtime_snapshot.clone(),
            actual: actual.to_hex(),
        });
    }
    let checkpoint: Checkpoint = serde_json::from_slice(&snapshot.serialized).map_err(|e| {
        RestorationError::CorruptObject {
            kind: "runtime",
            detail: e.to_string(),
        }
    })?;
    checkpoint
        .state
        .commit_state
        .validate()
        .map_err(PacketApplyError::from)?;
    if checkpoint.schema != SCHEMA {
        return Err(RestorationError::WrongObjectKind("runtime"));
    }
    if checkpoint.definitions != input.definitions
        || checkpoint.bindings != input.bindings
        || checkpoint.memory != input.memory
    {
        return Err(RestorationError::IncompatibleReferences);
    }
    Admission::new(&registry).state(&checkpoint.state)?;
    Ok(Restored {
        registry,
        bindings,
        context,
        state: checkpoint.state,
    })
}

/// Persist through existing ObjectStore/SnapshotStore; does not publish a head.
pub fn checkpoint<R: Repository>(
    repository: &mut R,
    definitions: &[RevisionReference],
    bindings: &[RevisionReference],
    context: &ContextStore,
    state: &RuntimeState,
) -> Result<RestorationInput, RestorationError> {
    prepare_checkpoint(repository, definitions, bindings, context, state)?.persist(repository)
}

/// All semantic/reference/size checks precede the first durable write. The
/// application may serialize its reference CAS before persisting this plan.
pub fn prepare_checkpoint<R: Repository>(
    repository: &R,
    definitions: &[RevisionReference],
    bindings: &[RevisionReference],
    context: &ContextStore,
    state: &RuntimeState,
) -> Result<CheckpointPlan, RestorationError> {
    let (registry, _) = adopted(repository, definitions, bindings)?;
    Admission::new(&registry).state(state)?;
    context.validate()?;
    state
        .commit_state
        .validate()
        .map_err(PacketApplyError::from)?;
    let memory_bytes =
        serde_json::to_vec(context).map_err(|e| RestorationError::CorruptObject {
            kind: "memory",
            detail: e.to_string(),
        })?;
    if memory_bytes.len() > MAX_BYTES {
        return Err(RestorationError::CorruptObject {
            kind: "memory",
            detail: "capacity exceeded".into(),
        });
    }
    let memory_id = Digest::object_digest(&memory_bytes).to_hex();
    let document = Checkpoint {
        schema: SCHEMA.into(),
        definitions: definitions.to_vec(),
        bindings: bindings.to_vec(),
        memory: memory_id.clone(),
        state: state.clone(),
    };
    let serialized =
        serde_json::to_vec(&document).map_err(|e| RestorationError::CorruptObject {
            kind: "runtime",
            detail: e.to_string(),
        })?;
    if serialized.len() > MAX_BYTES {
        return Err(RestorationError::CorruptObject {
            kind: "runtime",
            detail: "capacity exceeded".into(),
        });
    }
    let digest = Digest::object_digest(&serialized);
    let runtime_snapshot = format!("runtime-{}", digest.to_hex());
    let input = RestorationInput {
        definitions: definitions.to_vec(),
        bindings: bindings.to_vec(),
        memory: memory_id,
        runtime_snapshot,
    };
    Ok(CheckpointPlan {
        input,
        memory_bytes,
        digest,
        serialized,
    })
}

/// Validated, bounded serialization; not a live state, log or alternate memory.
/// Fields are private and there is no deserializer or unchecked constructor.
pub struct CheckpointPlan {
    input: RestorationInput,
    memory_bytes: Vec<u8>,
    digest: Digest,
    serialized: Vec<u8>,
}
impl CheckpointPlan {
    pub fn restoration(&self) -> &RestorationInput {
        &self.input
    }
    pub fn persist<R: Repository>(
        self,
        repository: &mut R,
    ) -> Result<RestorationInput, RestorationError> {
        let stored = repository.put_object(self.memory_bytes)?;
        if stored != self.input.memory {
            return Err(RestorationError::DigestMismatch {
                expected: self.input.memory,
                actual: stored,
            });
        }
        let stored = repository.store_snapshot(RepositorySnapshot {
            revision: self.input.runtime_snapshot.clone(),
            state_digest: self.digest,
            serialized: self.serialized,
        })?;
        if stored != self.input.runtime_snapshot {
            return Err(RestorationError::RevisionMismatch("runtime writer"));
        }
        read(repository, &self.input)?;
        Ok(self.input)
    }
}

#[derive(Debug, Serialize)]
pub struct AppliedCheckpoint {
    pub commit: sem_lang_repository::CommitReceipt,
    pub restoration: RestorationInput,
    pub receipt: ApplicationReceipt,
}

/// The inner packet and outer repository have distinct canonical commit scopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRequest {
    pub packet: PacketRequest,
    pub expected_commit_revision: sem_lang_repository::RevisionRef,
}

/// All requests are explicit. Replay precedes current-head validation and never
/// republishes an old result as the new head. No provider execution occurs here.
pub fn apply<R: Repository + sem_lang_repository::Backend>(
    repository: &mut R,
    input: &RestorationInput,
    packet: &SemanticPacket,
    request: &CheckpointRequest,
    head: &str,
) -> Result<AppliedCheckpoint, RestorationError> {
    use sem_lang_repository::commit_image::{BorrowedBackend, SnapshotBackend};
    use sem_lang_repository::{
        CommitStore, Failure, HeadPublication, RepositoryRoot, commit_result, head_intent,
    };
    let bytes = serde_json::to_vec(&(input, packet, &request.packet)).map_err(|e| {
        RestorationError::CorruptObject {
            kind: "application request",
            detail: e.to_string(),
        }
    })?;
    if bytes.len() > MAX_BYTES {
        return Err(RestorationError::CorruptObject {
            kind: "application request",
            detail: "capacity exceeded".into(),
        });
    }
    let request_digest = Digest::object_digest(&bytes);
    // Validate against exact immutable definitions, never today's mutable head.
    // Even replay must not publish before full semantic admission succeeds.
    let restored = read(repository, input)?;
    Admission::new(&restored.registry).packet(packet)?;
    let commits = CommitStore::new(BorrowedBackend(&*repository));
    if let Some(receipt) = commits
        .receipt(head, &request.packet.operation_id)
        .map_err(RepositoryError::from)?
    {
        let prepared = commits
            .committed(&receipt.commit_ref)
            .map_err(RepositoryError::from)?
            .ok_or(RepositoryError::CommitFailure(Failure::Corrupt))?;
        let mut result = HeadPublication::decode(prepared.payload())?;
        result.request_digest = request_digest;
        let replay = head_intent(
            head,
            &request.packet.operation_id,
            &request.expected_commit_revision,
            &result,
        )?;
        let commit = repository.publish_head(&replay)?;
        let RepositoryRoot::Snapshot(runtime_snapshot) = result.root else {
            return Err(RestorationError::WrongObjectKind("runtime replay"));
        };
        let next = RestorationInput {
            runtime_snapshot,
            ..input.clone()
        };
        let restored = read(repository, &next)?;
        let receipt = PacketReducer::reduce(&restored.state, packet, &request.packet)?
            .receipt()
            .clone();
        return Ok(AppliedCheckpoint {
            restoration: next,
            receipt,
            commit,
        });
    }
    if repository.head_at(&request.expected_commit_revision)?
        != Some(RepositoryRoot::Snapshot(input.runtime_snapshot.clone()))
    {
        return Err(RestorationError::RevisionMismatch("runtime input base"));
    }
    let transition = PacketReducer::reduce(&restored.state, packet, &request.packet)?;
    let receipt = transition.receipt().clone();
    let plan = prepare_checkpoint(
        repository,
        &input.definitions,
        &input.bindings,
        &restored.context,
        &transition.into_state(),
    )?;
    let result = HeadPublication {
        root: RepositoryRoot::Snapshot(plan.restoration().runtime_snapshot.clone()),
        request_digest,
    };
    let prepared = head_intent(
        head,
        &request.packet.operation_id,
        &request.expected_commit_revision,
        &result,
    )?;
    let preview = SnapshotBackend::new(
        repository
            .read(Clone::clone)
            .map_err(RepositoryError::from)?,
    )
    .map_err(RepositoryError::from)?;
    commit_result(CommitStore::new(&preview).commit(&prepared))?;
    let next = plan.persist(repository)?;
    let commit = repository.publish_head(&prepared)?;
    Ok(AppliedCheckpoint {
        restoration: next,
        receipt,
        commit,
    })
}

impl Engine {
    /// Restore a bounded in-memory view while the caller keeps its repository
    /// fence. No repository is copied, opened, initialized or implicitly repaired.
    pub fn restore_view<R: Repository>(
        repository: &R,
        input: RestorationInput,
    ) -> Result<Self, RestorationError> {
        Self::from_restored(read(repository, &input)?, input, None)
    }

    pub fn restore<R: Repository + 'static>(
        repository: R,
        input: RestorationInput,
    ) -> Result<Self, RestorationError> {
        let restored = read(&repository, &input)?;
        Self::from_restored(restored, input, Some(Box::new(repository)))
    }

    pub(crate) fn from_restored(
        restored: Restored,
        input: RestorationInput,
        repository: Option<Box<dyn Repository>>,
    ) -> Result<Self, RestorationError> {
        // Restore the validated state directly. Building an unrelated initial
        // language first duplicated work and required a fake repository for plans.
        let knowledge = sem_lang_linguistics::LinguisticKnowledge::new();
        let compiled_language = sem_lang_language::CompiledLanguage::build(
            &knowledge,
            &restored.registry,
            &input.runtime_snapshot,
        )
        .map_err(|e| EngineBuildError::Language(e.to_string()))?;
        Ok(Self {
            runtime: sem_lang_runtime::RuntimeEngine::with_state(
                restored.registry.clone(),
                restored.state,
            ),
            repository,
            registry: restored.registry,
            knowledge,
            compiled_language,
            context: sem_lang_context::ContextEngine::new(restored.context),
            restoration: Some(input),
            adopted_bindings: restored.bindings,
            #[cfg(feature = "evolution")]
            interpreter: None,
            #[cfg(feature = "evolution")]
            reviewer: None,
        })
    }

    pub fn restoration_input(&self) -> Option<&RestorationInput> {
        self.restoration.as_ref()
    }
    pub fn adopted_bindings(&self) -> &[RevisionedBinding] {
        &self.adopted_bindings
    }
}
