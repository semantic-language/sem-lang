#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
use adoption_fixture::adopt;
use sem_lang_memory::definitions::read;
use sem_lang_repository::Repository;
use sem_lang_store_memory::InMemoryRepository;
use sem_lang_vocab::VocabularyModule;

fn definitions() -> Vec<VocabularyModule> {
    let mut module: VocabularyModule = serde_json::from_str(r#"{"schema_version":"0.10.0","vocabulary_id":"example","version":"0.10.0","dependencies":[],"terms":[{"id":"4280fdac26ef9819dbf6c88a361d5366","label":"Designation","description":"Designation","deprecation":null,"replacement":null,"semantic_definition":null}],"types":[],"roles":[],"lexical_bindings":[]}"#).unwrap();
    // The test exercises typed input, never treating a label as its meaning.
    module.terms[0].id =
        sem_lang_core::TermId::from_hex("42000000000000000000000000000000").unwrap();
    module.terms[0].description =
        "A designation of a subject, not evidence of a legal identity".into();
    // The relation is an explicit primitive; the designation composes it.
    // A self-referential label fixture would not prove usable semantics.
    let mut relation = module.terms[0].clone();
    relation.id = sem_lang_core::TermId::from_hex("42000000000000000000000000000001").unwrap();
    relation.label = "Relation".into();
    relation.semantic_definition =
        Some(serde_json::from_str(r#"{"Primitive":"Relation"}"#).unwrap());
    module.roles = serde_json::from_value(serde_json::json!([
        {"id":"43000000000000000000000000000000","label":"subject","cardinality":"ExactlyOne","description":""},
        {"id":"43000000000000000000000000000001","label":"value","cardinality":"ExactlyOne","description":""}
    ])).unwrap();
    module.terms[0].semantic_definition = Some(serde_json::from_str(r#"{"Composition":{"Apply":{"predicate":"42000000000000000000000000000001","arguments":[["43000000000000000000000000000000",{"Variable":"person"}],["43000000000000000000000000000001",{"Literal":"display designation"}]]}}}"#).unwrap());
    module.terms.push(relation);
    vec![module]
}

#[test]
fn typed_definitions_are_explicit_revisioned_and_lossless_across_reload() {
    let mut repository = InMemoryRepository::new();
    assert!(read(&repository, "owner").unwrap().is_none());
    let modules = definitions();
    let expected = serde_json::to_value(&modules).unwrap();
    assert!(adopt(&mut repository, "owner", None, &modules, "").is_err());
    assert!(read(&repository, "owner").unwrap().is_none());
    let revision = adopt(&mut repository, "owner", None, &modules, "decision:1").unwrap();
    assert_eq!(
        adopt(&mut repository, "owner", None, &modules, "decision:1").unwrap(),
        revision,
        "retrying an acknowledged decision must not create another revision"
    );
    let (head, memory) = read(&repository, "owner").unwrap().unwrap();
    assert_eq!(head, revision);
    assert_eq!(memory.decision_ref, "decision:1");
    assert_eq!(serde_json::to_value(&memory.modules).unwrap(), expected);
    let mut registry = sem_lang_vocab::VocabularyRegistry::new();
    for module in memory.modules {
        registry.register(module).unwrap();
    }
    let language = sem_lang_language::CompiledLanguage::embedded(&registry).unwrap();
    let term = modules[0].terms[0].id;
    let trace = language.semantic_expander().expand(term).unwrap();
    assert!(trace.residuals.is_empty());
    assert!(trace.result.canonical().contains("display designation"));
    assert_eq!(language.resources.semantic_definitions.entries.len(), 2);
    assert!(read(&repository, "other-role").unwrap().is_none());
    assert!(adopt(&mut repository, "owner", None, &modules, "decision:stale").is_err());
    assert_eq!(read(&repository, "owner").unwrap().unwrap().0, revision);
    let updated = adopt(
        &mut repository,
        "owner",
        Some(&revision),
        &modules,
        "decision:2",
    )
    .unwrap();
    assert_ne!(revision, updated);
    assert_eq!(
        read(&repository, "owner")
            .unwrap()
            .unwrap()
            .1
            .previous_revision,
        Some(revision)
    );
    assert!(repository.verify().unwrap().errors.is_empty());
}

#[test]
fn old_label_only_or_unknown_fields_cannot_be_adopted_as_definitions() {
    let mut repository = InMemoryRepository::new();
    let mut modules = definitions();
    modules[0].terms[0].semantic_definition = None;
    assert!(adopt(&mut repository, "owner", None, &modules, "decision:1").is_err());
    assert!(read(&repository, "owner").unwrap().is_none());
    let mut encoded = serde_json::to_value(definitions()).unwrap();
    encoded[0]["terms"][0]["permission"] = serde_json::json!("execute");
    assert!(serde_json::from_value::<Vec<VocabularyModule>>(encoded).is_err());
    let mut broken = definitions();
    broken[0].terms[0].semantic_definition =
        Some(serde_json::from_str(r#"{"Alias":"42000000000000000000000000000002"}"#).unwrap());
    assert!(adopt(&mut repository, "owner", None, &broken, "decision:1").is_err());
    assert!(read(&repository, "owner").unwrap().is_none());
    let mut invalid_id = serde_json::to_value(definitions()).unwrap();
    invalid_id[0]["terms"][0]["id"] = serde_json::json!("𐀀".repeat(8));
    assert!(serde_json::from_value::<Vec<VocabularyModule>>(invalid_id).is_err());
    let mut dangling_binding = serde_json::to_value(definitions()).unwrap();
    dangling_binding[0]["rendering_rules"] = serde_json::json!([{
        "term": "420000000000000000000000000000ff", "template": "not a declared term"
    }]);
    assert!(serde_json::from_value::<Vec<VocabularyModule>>(dangling_binding).is_err());
    for field in [
        "semantic_compositions",
        "constraints",
        "rendering_rules",
        "primitive_requirements",
        "grammar_bindings",
    ] {
        let mut obsolete = serde_json::to_value(definitions()).unwrap();
        obsolete[0][field] = serde_json::json!([]);
        assert!(
            serde_json::from_value::<Vec<VocabularyModule>>(obsolete).is_err(),
            "{field}"
        );
    }
    let mut duplicate_roles = serde_json::to_value(definitions()).unwrap();
    let role = serde_json::json!({"id":"42000000000000000000000000000002", "label":"subject",
        "cardinality":"ExactlyOne", "description":"The subject of an assertion"});
    duplicate_roles[0]["roles"] = serde_json::json!([role, role]);
    let duplicate_roles: Vec<VocabularyModule> = serde_json::from_value(duplicate_roles).unwrap();
    assert!(
        adopt(
            &mut repository,
            "owner",
            None,
            &duplicate_roles,
            "decision:1"
        )
        .is_err()
    );
    assert!(
        adopt(
            &mut repository,
            &"x".repeat(117),
            None,
            &definitions(),
            "decision:1"
        )
        .is_err()
    );
    assert!(read(&repository, "owner").unwrap().is_none());
}
