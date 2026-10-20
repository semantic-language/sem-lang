use sem_lang_core::TermId;
use sem_lang_language::CompiledLanguage;
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition};
use sem_lang_vocab::{VocabularyModule, VocabularyRegistry};

fn module() -> VocabularyModule {
    let mut module = sem_lang_fixtures::domain::reservation_fixture_module();
    module.dependencies.clear();
    module.types.clear();
    module.roles.clear();
    module.terms.truncate(1);
    module.terms[0].label = "Shared display name".into();
    module.terms[0].id = TermId::from_hex("42000000000000000000000000000000").unwrap();
    module.terms[0].semantic_definition =
        Some(SemanticDefinition::Primitive(PrimitiveSemantic::Value));
    module.lexical_bindings.truncate(1);
    module.lexical_bindings[0].target_term = module.terms[0].id;
    module.lexical_bindings[0].surface_form = "first".into();
    module.lexical_bindings[0].language_tag = "en".into();
    module.lexical_bindings[0].semantic_type_condition = None;
    module
}

fn language(modules: &[VocabularyModule]) -> CompiledLanguage {
    let mut registry = VocabularyRegistry::new();
    for module in modules {
        registry.register(module.clone()).unwrap();
    }
    CompiledLanguage::embedded(&registry).unwrap()
}

#[test]
fn definition_changes_invalidate_compilation_identity_without_changing_labels() {
    let module = module();
    let first = language(std::slice::from_ref(&module));
    let mut changed = module.clone();
    changed.terms[0].semantic_definition =
        Some(SemanticDefinition::Primitive(PrimitiveSemantic::Relation));
    let second = language(&[changed]);
    assert_ne!(
        first.digest, second.digest,
        "semantic definition is a compiler input, not just a label"
    );
    let restored = language(&[module]);
    assert_eq!(first.digest, restored.digest);
}

#[test]
fn equal_display_names_never_merge_meanings_and_ambiguous_input_is_not_guessed() {
    let first = module();
    let mut registry = VocabularyRegistry::new();
    registry.register(first.clone()).unwrap();
    let mut conflicting_identity = first.clone();
    conflicting_identity.vocabulary_id = "conflicting".into();
    assert!(registry.register(conflicting_identity).is_err());
    let mut second = first.clone();
    second.vocabulary_id = "other".into();
    second.terms[0].id = TermId::from_hex("42000000000000000000000000000001").unwrap();
    second.terms[0].semantic_definition =
        Some(SemanticDefinition::Primitive(PrimitiveSemantic::Relation));
    second.lexical_bindings[0].target_term = second.terms[0].id;
    second.lexical_bindings[0].surface_form = "second".into();
    let compiled = language(&[first.clone(), second.clone()]);
    assert!(
        compiled.term("Shared display name").is_none(),
        "display name is not a unique ID"
    );
    assert!(compiled.definition("Shared display name").is_none());
    for (text, expected) in [("first", &first), ("second", &second)] {
        let result = compiled.compile_semantics(text, "en").unwrap();
        assert_eq!(result.concept, expected.terms[0].id);
        assert_eq!(
            serde_json::to_value(&result.definition).unwrap(),
            serde_json::to_value(&expected.terms[0].semantic_definition).unwrap()
        );
        assert!(result.normalized.residuals.is_empty());
    }
    second.lexical_bindings[0].surface_form = "first".into();
    // Priority and ordering must not silently select a different meaning.
    second.lexical_bindings[0].priority = u32::MAX;
    let ambiguous = language(&[first, second]);
    assert!(ambiguous.compile_semantics("first", "en").is_err());
}
