//! semantic facade with owner-validated feedback transitions.

#[cfg(feature = "memory-store")]
mod admission;

#[cfg(feature = "memory-store")]
pub mod current;
#[cfg(feature = "memory-store")]
pub mod feedback;
#[cfg(feature = "memory-store")]
pub mod restoration;
#[cfg(feature = "memory-store")]
pub mod retention;
#[cfg(feature = "semantic-execution")]
pub mod semantic_execution;
pub mod standard;
#[cfg(feature = "semantic-execution")]
pub mod surface_admission;

use sem_lang_context::{ContextEngine, ContextStore};
use sem_lang_language::CompiledLanguage;
use sem_lang_linguistics::LinguisticKnowledge;
use sem_lang_repository::Repository;
use sem_lang_runtime::RuntimeEngine;
use sem_lang_vocab::{VocabularyError, VocabularyRegistry};
use thiserror::Error;

#[cfg(feature = "evolution")]
use sem_lang_evolution::{InterpreterError, ReviewDecision, ReviewerError};

pub use sem_lang_core::{
    CanonicalExpression, CausalDag, CausalReference, Claim, Constraint, EffectIntent, EntityId,
    EvaluationResult, Mutation, MutationDelta, PacketId, RuntimeState, SemanticGraph,
    SemanticPacket, TypedHole, Value,
};
pub use sem_lang_frontend::{CompileContext, ExceptionCase, ExceptionKind, SurfaceInput};
pub use sem_lang_language::{SemanticMatch, SemanticMatchMethod};
pub use sem_lang_repository::{ObjectStore, Repository as RepositoryTrait, RepositoryError};
pub use sem_lang_runtime::entity_index;
pub use sem_lang_runtime::{
    PacketApplyError, PacketReducer, PacketRequest, RuntimeStateTransition,
};
pub use sem_lang_vocab::VocabularyModule;

#[cfg(feature = "json")]
pub use sem_lang_codec::Codec;

#[cfg(feature = "memory-store")]
pub use sem_lang_store_memory::InMemoryRepository;

#[cfg(feature = "fs-store")]
pub use sem_lang_store_fs::FsRepository;

#[cfg(feature = "evolution")]
pub use sem_lang_evolution::{
    ExceptionInterpreter, ExtensionProposal, ExtensionReviewer, InterpretationProposal,
};

/// Compile context alias used by public examples.
pub type Context = CompileContext;

/// Builder entrypoint.
pub struct EngineBuilder {
    repository: Option<Box<dyn Repository>>,
    registry: VocabularyRegistry,
    knowledge: LinguisticKnowledge,
    context: ContextEngine,
    runtime_state: RuntimeState,
    #[cfg(feature = "evolution")]
    interpreter: Option<Box<dyn ExceptionInterpreter>>,
    #[cfg(feature = "evolution")]
    reviewer: Option<Box<dyn ExtensionReviewer>>,
}

impl EngineBuilder {
    pub fn new() -> Self {
        Self {
            repository: None,
            registry: VocabularyRegistry::new(),
            knowledge: LinguisticKnowledge::new(),
            context: ContextEngine::new(ContextStore::default()),
            runtime_state: RuntimeState::new(),
            #[cfg(feature = "evolution")]
            interpreter: None,
            #[cfg(feature = "evolution")]
            reviewer: None,
        }
    }

    pub fn repository<R: Repository + 'static>(mut self, repository: R) -> Self {
        self.repository = Some(Box::new(repository));
        self
    }

    pub fn with_vocabulary_module(
        mut self,
        module: VocabularyModule,
    ) -> Result<Self, EngineBuildError> {
        self.registry
            .register(module)
            .map_err(EngineBuildError::Vocabulary)?;
        Ok(self)
    }

    #[cfg(feature = "evolution")]
    pub fn with_exception_interpreter<I: ExceptionInterpreter + 'static>(
        mut self,
        interpreter: I,
    ) -> Self {
        self.interpreter = Some(Box::new(interpreter));
        self
    }

    #[cfg(feature = "evolution")]
    pub fn with_extension_reviewer<I: ExtensionReviewer + 'static>(mut self, reviewer: I) -> Self {
        self.reviewer = Some(Box::new(reviewer));
        self
    }

    pub fn with_runtime_state(mut self, state: RuntimeState) -> Self {
        self.runtime_state = state;
        self
    }

    pub fn with_linguistic_knowledge(mut self, knowledge: LinguisticKnowledge) -> Self {
        self.knowledge = knowledge;
        self
    }

    pub fn with_context_store(mut self, store: sem_lang_context::ContextStore) -> Self {
        self.context = sem_lang_context::ContextEngine::new(store);
        self
    }

    pub fn build(self) -> Result<Engine, EngineBuildError> {
        if self.repository.is_none() {
            return Err(EngineBuildError::MissingRepository);
        }
        let compiled_language = CompiledLanguage::build(&self.knowledge, &self.registry, "initial")
            .map_err(|error| EngineBuildError::Language(error.to_string()))?;
        let mut runtime = RuntimeEngine::new(self.registry.clone());
        runtime.state = self.runtime_state;
        Ok(Engine {
            #[cfg(feature = "memory-store")]
            restoration: None,
            #[cfg(feature = "memory-store")]
            adopted_bindings: Vec::new(),
            runtime,
            registry: self.registry,
            knowledge: self.knowledge,
            context: self.context,
            compiled_language,
            repository: self.repository,
            #[cfg(feature = "evolution")]
            interpreter: self.interpreter,
            #[cfg(feature = "evolution")]
            reviewer: self.reviewer,
        })
    }
}

impl Default for EngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Public façade object.
pub struct Engine {
    #[cfg(feature = "memory-store")]
    restoration: Option<restoration::RestorationInput>,
    #[cfg(feature = "memory-store")]
    adopted_bindings: Vec<sem_lang_memory::bindings::RevisionedBinding>,
    runtime: RuntimeEngine,
    repository: Option<Box<dyn Repository>>,
    registry: VocabularyRegistry,
    knowledge: LinguisticKnowledge,
    compiled_language: CompiledLanguage,
    context: ContextEngine,
    #[cfg(feature = "evolution")]
    interpreter: Option<Box<dyn ExceptionInterpreter>>,
    #[cfg(feature = "evolution")]
    reviewer: Option<Box<dyn ExtensionReviewer>>,
}

impl Engine {
    pub fn builder() -> EngineBuilder {
        EngineBuilder::new()
    }

    pub fn runtime(&self) -> &RuntimeEngine {
        &self.runtime
    }

    /// Disposable lookup against this exact restored semantic owner image.
    /// Neither the rules nor lookup results authorize adoption or execution.
    pub fn entity_index(
        &self,
        rules: &[entity_index::IndexRule],
        now: i64,
    ) -> Result<entity_index::EntityIndex, entity_index::IndexError> {
        entity_index::EntityIndex::build(&self.runtime.state, &self.registry, rules, now)
    }

    pub fn compile_semantics(
        &self,
        text: &str,
        locale: &str,
    ) -> Result<sem_lang_language::SemanticCompilation, sem_lang_language::SemanticCompileError>
    {
        self.compiled_language.compile_semantics(text, locale)
    }

    pub fn context(&self) -> &ContextEngine {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut ContextEngine {
        &mut self.context
    }

    pub fn apply_packet(
        &self,
        state: &RuntimeState,
        packet: &SemanticPacket,
        request: &sem_lang_runtime::PacketRequest,
    ) -> Result<RuntimeStateTransition, EngineError> {
        self.runtime
            .reduce(state, packet, request)
            .map_err(EngineError::PacketApplication)
    }

    #[cfg(feature = "evolution")]
    pub fn propose_exception(
        &self,
        case: &ExceptionCase,
    ) -> Result<InterpretationProposal, EngineError> {
        self.interpreter
            .as_ref()
            .ok_or(EngineError::EvolutionUnavailable)?
            .propose(case)
            .map_err(EngineError::Interpreter)
    }

    #[cfg(feature = "evolution")]
    pub fn review_proposal(
        &self,
        proposal: &ExtensionProposal,
    ) -> Result<ReviewDecision, EngineError> {
        self.reviewer
            .as_ref()
            .ok_or(EngineError::EvolutionUnavailable)?
            .review(proposal, &self.registry)
            .map_err(EngineError::Review)
    }

    pub fn repository(&self) -> Option<&dyn Repository> {
        self.repository.as_deref()
    }

    pub fn registry(&self) -> &VocabularyRegistry {
        &self.registry
    }

    pub fn linguistic_knowledge(&self) -> &LinguisticKnowledge {
        &self.knowledge
    }

    pub fn compiled_language(&self) -> &CompiledLanguage {
        &self.compiled_language
    }

    pub fn close(self) {
        drop(self);
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    PacketApplication(#[from] sem_lang_runtime::PacketApplyError),
    #[error("runtime error: {0}")]
    Runtime(#[from] sem_lang_runtime::RuntimeError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("vocabulary: {0}")]
    Vocabulary(#[from] VocabularyError),
    #[cfg(feature = "evolution")]
    #[error("interpreter: {0}")]
    Interpreter(#[from] InterpreterError),
    #[cfg(feature = "evolution")]
    #[error("review: {0}")]
    Review(#[from] ReviewerError),
    #[error("evolution unavailable")]
    EvolutionUnavailable,
}

#[derive(Debug, Error)]
pub enum EngineBuildError {
    #[error("missing repository")]
    MissingRepository,
    #[error("vocabulary error: {0}")]
    Vocabulary(#[from] VocabularyError),
    #[error("language build error: {0}")]
    Language(String),
}

/// Public façade prelude.
pub mod prelude {
    #[cfg(feature = "json")]
    pub use crate::Codec;
    #[cfg(feature = "memory-store")]
    pub use crate::InMemoryRepository;
    #[cfg(feature = "evolution")]
    pub use crate::InterpretationProposal;
    pub use crate::{
        Context, Engine, EngineBuildError, EngineBuilder, EngineError, Exception, ExceptionCase,
        RuntimeState, RuntimeStateTransition, SurfaceInput,
    };
    pub use sem_lang_context::{
        ContextAuthority, ContextBinding, ContextEngine, ContextKind, ContextScope, ContextSource,
        ContextStore, ObservationError, ObservationReceipt, ObservationRequest,
        PersistentContextualDefault, RetainedObservation, SurfaceRetention,
    };
    pub use sem_lang_core::EffectIntent;
    #[cfg(feature = "evolution")]
    pub use sem_lang_evolution::ExceptionInterpreter;
    #[cfg(feature = "evolution")]
    pub use sem_lang_evolution::ExtensionProposal;
    #[cfg(feature = "evolution")]
    pub use sem_lang_evolution::{
        BasicExceptionInterpreter, BasicExtensionReviewer, ReviewDecision,
    };
    pub use sem_lang_induction::ConservativeContextInducer;
    pub use sem_lang_language::{
        CanonicalSemanticIr, CompiledLanguage, GeneratedLanguageResources, LanguageExplanation,
        SemanticCompilation, SemanticCompileError,
    };
    pub use sem_lang_linguistics::{
        DefinitionOrigin, KernelKnowledge, KernelVocabulary, KnowledgeOrigin, LanguageDefinition,
        LinguisticKnowledge, SemanticDefinition, SemanticExpansionTrace, SemanticExpr,
        SenseDefinition, VocabularyLayer, domain_vocabulary, semantic_basis_vocabulary,
        standard_semantic_vocabulary,
    };
    pub use sem_lang_repository::Repository;
    #[cfg(feature = "fs-store")]
    pub use sem_lang_store_fs::FsRepository;
    pub use sem_lang_usage::{LinguisticContext, UsageMemory, UsageObservation};
    pub use sem_lang_vocab::{VocabularyModule, VocabularyRegistry};
}

pub type Exception = ExceptionCase;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_requires_repository() {
        let result = Engine::builder().build();
        assert!(result.is_err());
    }

    #[cfg(feature = "memory-store")]
    #[test]
    fn explicitly_supplied_state_has_one_runtime_owner() {
        let mut state = RuntimeState::new();
        state
            .annotations
            .insert("accepted".into(), "test state".into());
        let expected = serde_json::to_value(&state).unwrap();
        let engine = Engine::builder()
            .repository(InMemoryRepository::new())
            .with_runtime_state(state)
            .build()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&engine.runtime().state).unwrap(),
            expected
        );
    }

    #[cfg(feature = "memory-store")]
    #[test]
    fn base_engine_builds_without_vocabulary_fixture() {
        let engine = Engine::builder()
            .repository(InMemoryRepository::new())
            .build()
            .expect("base engine");
        assert!(engine.registry().modules().next().is_none());
        assert!(engine.compile_semantics("unregistered", "en-US").is_err());
    }
}
