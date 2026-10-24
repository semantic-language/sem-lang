//! An actual public producer definition through this independent owner consumer.
use sem_lang::{
    CompileContext, Engine, RuntimeState, SurfaceInput,
    current::{ProcessingOutcome, Reason},
    restoration,
};
use sem_lang_context::{ContextScope, ContextStore, ContextSubject};
use sem_lang_core::TermId;
use sem_lang_memory::definitions;
use sem_lang_repository::{Repository, RevisionRef};
use sem_lang_store_fs::FsRepository;
use sem_lang_vocab::VocabularyModule;
use std::{fs, os::unix::fs::PermissionsExt};
const SOURCE: &str = include_str!("../fixtures/coela-evidence/evidence.sem");
const NAMESPACE: &str = "coela-independent-owner";
fn modules() -> Vec<VocabularyModule> {
    let mut modules = sem_lang_standard::language_set().unwrap().modules.clone();
    modules.push(VocabularyModule::from_sem(SOURCE).unwrap());
    modules
}
#[test]
fn public_coela_source_is_used_after_explicit_adoption_and_durable_reload() {
    let temp = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = temp.path().join("source-store");
    assert!(FsRepository::open_existing(&path).is_err());
    assert!(!path.exists());
    let mut repo = FsRepository::create(&path).unwrap();
    let revision = definitions::adopt(
        &mut repo,
        NAMESPACE,
        None,
        &modules(),
        "test:coela-reviewed",
        &RevisionRef::default(),
    )
    .unwrap();
    let refs = restoration::checkpoint(
        &mut repo,
        &[restoration::RevisionReference {
            namespace: NAMESPACE.into(),
            revision: revision.clone(),
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )
    .unwrap();
    drop(repo);
    let reopened = FsRepository::open_existing(&path).unwrap();
    assert_eq!(
        definitions::read(&reopened, NAMESPACE).unwrap().unwrap().0,
        revision
    );
    let engine = Engine::restore_view(&reopened, refs.clone()).unwrap();
    let term = TermId::from_declaration("coela/engineering/evidence", "ImplementationEvidence");
    let mut context = CompileContext {
        subject: ContextSubject {
            id: "synthetic-reviewer".into(),
            class: "test".into(),
        },
        evidence: vec![],
        at: None,
        locale: "en".into(),
        semantic_revision: engine
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap(),
        context_refs: vec![],
        context_scope: ContextScope::CurrentRequest,
        observation_evidence: vec![],
    };
    for (locale, text) in [("en", "implementation evidence"), ("ja", "実装の証拠")] {
        context.locale = locale.into();
        let compiled = engine
            .compiled_language()
            .compile_current_semantics(text, locale, None, &engine.runtime().state)
            .unwrap();
        assert_eq!(compiled.concept, term);
        assert!(compiled.complete_surface && compiled.trace.residuals.is_empty());
        let ProcessingOutcome::NeedsResolution(result) = engine
            .compile_surface(&SurfaceInput::new("producer", text), &context)
            .unwrap()
        else {
            panic!("unasserted factual relation must require evidence")
        };
        assert_eq!(result.concept, Some(term));
        assert!(!result.issues.is_empty());
        assert!(
            result
                .issues
                .iter()
                .all(|issue| issue.reason == Reason::MissingContext)
        );
        let expanded = result
            .expression
            .as_ref()
            .expect("compiled native expression")
            .canonical();
        assert!(expanded.contains("implementation") && expanded.contains("reported"));
    }
    context.locale = "en".into();
    assert!(!matches!(
        engine
            .compile_surface(
                &SurfaceInput::new("unknown", "approve production"),
                &context
            )
            .unwrap(),
        ProcessingOutcome::Resolved(_)
    ));
    context.semantic_revision = "stale".into();
    assert!(
        engine
            .compile_surface(
                &SurfaceInput::new("stale", "implementation evidence"),
                &context
            )
            .is_err()
    );
    assert!(refs.bindings.is_empty());
    assert!(engine.runtime().state.claims.is_empty());
    assert!(reopened.verify().unwrap().errors.is_empty());
}

#[path = "coela/rejection.rs"]
mod rejection;
