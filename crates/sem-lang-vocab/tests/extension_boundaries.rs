use sem_lang_core::TermId;
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition};
use sem_lang_vocab::*;

fn module(id: &str) -> VocabularyModule {
    VocabularyModule {
        schema_version: "0.10.0".into(),
        vocabulary_id: id.into(),
        version: "0.10.0".into(),
        dependencies: vec![],
        terms: vec![],
        types: vec![],
        roles: vec![],
        lexical_bindings: vec![],
    }
}
fn term(id: TermId, definition: SemanticDefinition) -> TermDefinition {
    TermDefinition {
        id,
        label: "display only".into(),
        description: "fixture definition".into(),
        deprecation: None,
        replacement: None,
        semantic_definition: Some(definition),
    }
}

#[test]
fn regional_and_application_extensions_reference_only_declared_meaning_without_copying_it() {
    let id = TermId::new_random(1, 42);
    let mut base = module("base");
    base.terms.push(term(
        id,
        SemanticDefinition::Primitive(PrimitiveSemantic::Relation),
    ));
    let mut ja = module("locale.ja");
    ja.dependencies.push("base".into());
    ja.lexical_bindings.push(LexicalBinding {
        language_tag: "ja".into(),
        surface_form: "試験".into(),
        normalized_form: "試験".into(),
        contribution_kind: ContributionKind::Predicative,
        target_term: id,
        grammatical_condition: None,
        semantic_type_condition: None,
        priority: 1,
        valid_from: None,
        valid_until: None,
        provenance: "curator:fixture".into(),
    });
    let mut registry = VocabularyRegistry::new();
    assert!(
        registry.register(ja.clone()).is_err(),
        "missing dependency cannot be invented"
    );
    registry.register(base.clone()).unwrap();
    for dependencies in [vec!["locale.ja".into()], vec!["base".into(), "base".into()]] {
        let mut invalid = ja.clone();
        invalid.dependencies = dependencies;
        assert!(
            registry.register(invalid).is_err(),
            "self/duplicate imports reject"
        );
    }
    let mut undeclared = ja.clone();
    undeclared.dependencies.clear();
    assert!(
        registry.register(undeclared).is_err(),
        "registered does not mean imported"
    );
    registry
        .register(ja.clone())
        .expect("a locale can name an imported concept without redefining it");
    registry.validate_definitions().unwrap();
    assert_eq!(registry.modules().map(|m| m.terms.len()).sum::<usize>(), 1);
    assert!(registry.get("locale.ja").unwrap().terms.is_empty());
    assert_eq!(
        serde_json::to_value(registry.get("base")).unwrap(),
        serde_json::to_value(Some(&base)).unwrap()
    );

    let mut application = module("application");
    application.terms.push(term(
        TermId::new_random(2, 42),
        SemanticDefinition::Alias(id),
    ));
    let mut rejected = registry.clone();
    rejected.register(application.clone()).unwrap();
    assert!(
        rejected.validate_definitions().is_err(),
        "unrelated registered definitions are not ambient imports"
    );
    application.dependencies.push("base".into());
    registry.register(application).unwrap();
    registry.validate_definitions().unwrap();

    let mut copied = module("duplicate");
    copied.terms = base.terms;
    assert!(
        registry.register(copied).is_err(),
        "an extension cannot replace its dependency"
    );
    let mut broken = ja.clone();
    broken.vocabulary_id = "locale.broken".into();
    broken.lexical_bindings[0].target_term = TermId::new_random(999, 42);
    assert!(registry.register(broken).is_err());
    let mut indirect = ja;
    indirect.vocabulary_id = "locale.indirect".into();
    indirect.dependencies = vec!["application".into()];
    assert!(
        registry.register(indirect).is_err(),
        "direct imports must be explicit"
    );
}
