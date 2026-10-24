use sem_lang::{CompileContext, Engine};
use sem_lang_context::{ContextScope, ContextSubject};
use sem_lang_vocab::VocabularyModule;
pub(super) fn modules() -> Vec<VocabularyModule> {
    let mut modules = sem_lang_standard::language_set().unwrap().modules.clone();
    modules.push(VocabularyModule::from_sem(super::SOURCE).unwrap());
    modules
}

pub(super) fn context(engine: &Engine) -> CompileContext {
    CompileContext {
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
    }
}
