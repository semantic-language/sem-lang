//! A locale is a reviewed extension of meaning, not another meaning or permission.
use sem_lang::current::ProcessingOutcome;
use sem_lang::{
    CompileContext, Engine, InMemoryRepository, RuntimeState, SurfaceInput, restoration,
};
use sem_lang_context::{ContextScope, ContextStore, ContextSubject};
use sem_lang_memory::definitions;
use sem_lang_repository::RevisionRef;
use sem_lang_vocab::VocabularyModule;

#[test]
fn adopted_locale_resolves_same_concept_without_changing_facts_or_creating_bindings() {
    let base = sem_lang::standard::inference();
    let term = base.terms[0].id;
    let mut lexical = base.lexical_bindings[0].clone();
    lexical.language_tag = "ja".into();
    lexical.surface_form = "推論する".into();
    lexical.normalized_form = lexical.surface_form.clone();
    lexical.provenance = "review:ja-fixture".into();
    let locale = VocabularyModule {
        schema_version: "0.10.0".into(),
        vocabulary_id: "locale.ja.fixture".into(),
        version: "0.10.0".into(),
        dependencies: vec![base.vocabulary_id.clone()],
        terms: vec![],
        types: vec![],
        roles: vec![],
        lexical_bindings: vec![lexical],
    };
    let mut repo = InMemoryRepository::default();
    let modules = [base, locale];
    let revision = definitions::adopt(
        &mut repo,
        "reviewed",
        None,
        &modules,
        "decision:locale",
        &RevisionRef::default(),
    )
    .unwrap();
    assert_eq!(
        revision,
        definitions::adopt(
            &mut repo,
            "reviewed",
            None,
            &modules,
            "decision:locale",
            &RevisionRef::default()
        )
        .unwrap()
    );
    let restored = restoration::checkpoint(
        &mut repo,
        &[restoration::RevisionReference {
            namespace: "reviewed".into(),
            revision: revision.clone(),
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )
    .unwrap();
    let engine = Engine::restore_view(&repo, restored.clone()).unwrap();
    let mut context = CompileContext {
        subject: ContextSubject {
            id: "fictional-person".into(),
            class: "person".into(),
        },
        evidence: vec![],
        at: None,
        locale: "ja".into(),
        semantic_revision: engine
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![],
    };
    let resolve = |context: &CompileContext, text| {
        let ProcessingOutcome::Resolved(value) = engine
            .compile_surface(&SurfaceInput::new("fixture", text), context)
            .unwrap()
        else {
            panic!("explicit lexical binding must resolve")
        };
        value
    };
    let japanese = resolve(&context, "推論する");
    context.locale = "en".into();
    let english = resolve(&context, "infer");
    assert_eq!(japanese.concept, Some(term));
    assert_eq!(japanese.concept, english.concept);
    assert_eq!(japanese.expression, english.expression);
    assert!(matches!(
        engine
            .compile_surface(&SurfaceInput::new("unknown-locale", "推論する"), &context)
            .unwrap(),
        ProcessingOutcome::NeedsResolution(_)
    ));
    assert_eq!(
        definitions::read(&repo, "reviewed").unwrap().unwrap().0,
        revision
    );
    assert!(engine.runtime().state.claims.is_empty());
    assert!(
        restored.bindings.is_empty(),
        "knowing a phrase does not adopt a capability"
    );
    let reopened = Engine::restore_view(&repo, restored).unwrap();
    assert_eq!(reopened.runtime().state.claims.len(), 0);
}
