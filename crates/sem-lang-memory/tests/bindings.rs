#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
use adoption_fixture::write;
use sem_lang_core::TermId;
use sem_lang_memory::{bindings::*, definitions};
use sem_lang_repository::{ObjectStore, RevisionStore};
use sem_lang_store_memory::InMemoryRepository;
use sem_lang_vocab::VocabularyModule;

fn module() -> VocabularyModule {
    let mut module: VocabularyModule = serde_json::from_str(r#"{"schema_version":"0.10.0","vocabulary_id":"example","version":"0.10.0","dependencies":[],"terms":[{"id":"42436820882964cfc19b6db3750b8d17","label":"Relation","description":"Relation","deprecation":null,"replacement":null,"semantic_definition":null}],"types":[],"roles":[],"lexical_bindings":[]}"#).unwrap();
    module.terms[0].semantic_definition =
        Some(serde_json::from_str(r#"{"Primitive":"Relation"}"#).unwrap());
    module
}

fn setup() -> (InMemoryRepository, SemanticBinding) {
    let mut repo = InMemoryRepository::new();
    let module = module();
    let term = module.terms[0].id;
    let revision =
        adoption_fixture::adopt(&mut repo, "owner", None, &[module], "decision:definition")
            .unwrap();
    let binding = SemanticBinding {
        binding_id: "residence".into(),
        semantic_ref: DefinitionRef {
            namespace: "owner".into(),
            revision,
            term,
        },
        target_ref: CapabilityRef {
            namespace: "example".into(),
            capability: "residence".into(),
            revision: Some("1".into()),
        },
        usage: BindingUsage::Resource,
        direction: Direction::Bidirectional,
        constraints: vec![],
        provenance_ref: "publisher:declaration:1".into(),
    };
    (repo, binding)
}

fn adopt(repo: &mut InMemoryRepository, binding: &SemanticBinding) -> String {
    let proposed = write(
        repo,
        "owner",
        None,
        std::slice::from_ref(binding),
        AdoptionState::Proposed,
        "decision:offer",
    )
    .unwrap();
    let reviewed = write(
        repo,
        "owner",
        Some(&proposed),
        std::slice::from_ref(binding),
        AdoptionState::Reviewed,
        "decision:review",
    )
    .unwrap();
    write(
        repo,
        "owner",
        Some(&reviewed),
        std::slice::from_ref(binding),
        AdoptionState::Adopted,
        "decision:adopt",
    )
    .unwrap()
}

#[test]
fn b1_b2_adoption_is_explicit_and_exact_revisions_never_follow_definition_heads() {
    let (mut repo, binding) = setup();
    assert!(
        resolve(&repo, "owner", &binding.semantic_ref)
            .unwrap()
            .is_empty()
    );
    assert!(
        write(
            &mut repo,
            "owner",
            None,
            std::slice::from_ref(&binding),
            AdoptionState::Adopted,
            "decision:skip"
        )
        .is_err()
    );
    let head = adopt(&mut repo, &binding);
    let resolved = resolve(&repo, "owner", &binding.semantic_ref).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].binding, binding);
    let original_binding_revision = resolved[0].revision.clone();
    let new_definition = adoption_fixture::adopt(
        &mut repo,
        "owner",
        Some(&binding.semantic_ref.revision),
        &[module()],
        "decision:new-definition",
    )
    .unwrap();
    assert_ne!(new_definition, binding.semantic_ref.revision);
    assert_eq!(
        resolve(&repo, "owner", &binding.semantic_ref).unwrap(),
        resolved
    );
    let new_ref = DefinitionRef {
        revision: new_definition,
        ..binding.semantic_ref.clone()
    };
    assert!(resolve(&repo, "owner", &new_ref).unwrap().is_empty());
    let mut changed = binding.clone();
    changed.target_ref.capability = "other/residence".into();
    assert!(
        write(
            &mut repo,
            "owner",
            Some(&head),
            &[changed.clone()],
            AdoptionState::Adopted,
            "decision:silent-rebind"
        )
        .is_err()
    );
    let next = write(
        &mut repo,
        "owner",
        Some(&head),
        &[changed.clone()],
        AdoptionState::Proposed,
        "decision:new-offer",
    )
    .unwrap();
    assert_ne!(next, head);
    assert_eq!(
        write(
            &mut repo,
            "owner",
            Some(&head),
            &[changed.clone()],
            AdoptionState::Proposed,
            "decision:new-offer"
        )
        .unwrap(),
        next
    );
    assert!(
        write(
            &mut repo,
            "owner",
            Some(&head),
            &[changed.clone()],
            AdoptionState::Proposed,
            "decision:stale"
        )
        .is_err()
    );
    assert_eq!(
        resolve(&repo, "owner", &binding.semantic_ref).unwrap(),
        resolved,
        "a proposal must not deactivate the previously adopted binding"
    );
    let old = read_revision(&repo, "owner", &head).unwrap();
    assert_eq!(old.bindings[0].revision, original_binding_revision);
    assert_eq!(old.state, AdoptionState::Adopted);
    let rejected = write(
        &mut repo,
        "owner",
        Some(&next),
        &[changed.clone()],
        AdoptionState::Rejected,
        "decision:decline-offer",
    )
    .unwrap();
    assert_eq!(
        resolve(&repo, "owner", &binding.semantic_ref).unwrap(),
        resolved
    );
    let proposed = write(
        &mut repo,
        "owner",
        Some(&rejected),
        &[changed.clone()],
        AdoptionState::Proposed,
        "decision:reoffer",
    )
    .unwrap();
    let reviewed = write(
        &mut repo,
        "owner",
        Some(&proposed),
        &[changed.clone()],
        AdoptionState::Reviewed,
        "decision:review-reoffer",
    )
    .unwrap();
    write(
        &mut repo,
        "owner",
        Some(&reviewed),
        &[changed.clone()],
        AdoptionState::Adopted,
        "decision:accept-reoffer",
    )
    .unwrap();
    let replacement = resolve(&repo, "owner", &binding.semantic_ref).unwrap();
    assert_eq!(replacement[0].binding, changed);
    assert_ne!(replacement[0].revision, original_binding_revision);
    assert_eq!(
        read_revision(&repo, "owner", &head).unwrap().bindings[0].binding,
        binding
    );
}

#[test]
fn b3_b4_opaque_refs_restore_without_providers_and_rejection_preserves_facts() {
    let (mut repo, mut binding) = setup();
    binding.usage = BindingUsage::Presentation;
    binding.direction = Direction::Output;
    binding.target_ref.namespace = "presentation".into();
    binding.target_ref.capability = "location".into();
    let fact = repo
        .put_object(b"independent assertion authority".to_vec())
        .unwrap();
    let head = adopt(&mut repo, &binding);
    let record = read(&repo, "owner").unwrap().unwrap().1;
    let encoded = serde_json::to_vec(&record).unwrap();
    let restored: BindingAdoption = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(restored, record);
    for forbidden in [
        "assertion",
        "grant",
        "currently_available",
        "component",
        "input_schema",
    ] {
        let mut raw = serde_json::to_value(&binding).unwrap();
        raw[forbidden] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<SemanticBinding>(raw).is_err(),
            "{forbidden}"
        );
    }
    let rejected = write(
        &mut repo,
        "owner",
        Some(&head),
        &[binding.clone()],
        AdoptionState::Rejected,
        "decision:reject",
    )
    .unwrap();
    assert_ne!(head, rejected);
    assert!(
        resolve(&repo, "owner", &binding.semantic_ref)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.get_object(&fact).unwrap().unwrap(),
        b"independent assertion authority"
    );
    assert!(definitions::read(&repo, "owner").unwrap().is_some());
    assert_eq!(
        read_revision(&repo, "owner", &head).unwrap().bindings[0].binding,
        binding
    );
}

#[test]
fn invalid_refs_directions_provenance_and_binding_identity_are_rejected_before_writes() {
    let (mut repo, binding) = setup();
    let mut invalid = vec![];
    let mut b = binding.clone();
    b.semantic_ref.term = TermId::from_hex("ffffffffffffffffffffffffffffffff").unwrap();
    invalid.push(b);
    let mut b = binding.clone();
    b.semantic_ref.revision = "definitions-missing".into();
    invalid.push(b);
    let mut b = binding.clone();
    b.semantic_ref.namespace = "other".into();
    invalid.push(b);
    let mut b = binding.clone();
    b.target_ref.capability = " ".into();
    invalid.push(b);
    let mut b = binding.clone();
    b.provenance_ref.clear();
    invalid.push(b);
    let mut b = binding.clone();
    b.usage = BindingUsage::Observe;
    b.direction = Direction::Input;
    invalid.push(b);
    for b in invalid {
        assert!(
            write(
                &mut repo,
                "owner",
                None,
                &[b],
                AdoptionState::Proposed,
                "decision:test"
            )
            .is_err()
        );
    }
    assert!(
        write(
            &mut repo,
            "owner",
            None,
            &[binding.clone(), binding.clone()],
            AdoptionState::Proposed,
            "decision:duplicates"
        )
        .is_err()
    );
    assert!(read(&repo, "owner").unwrap().is_none());
    let head = adopt(&mut repo, &binding);
    assert!(read_revision(&repo, "other", &head).is_err());
}

#[test]
fn b9_revision_created_before_head_is_recoverable_without_duplicate_revision() {
    let (mut repo, binding) = setup();
    let head = adopt(&mut repo, &binding);
    let revision = repo.load_revision(&head).unwrap();
    let parent = revision.parent.clone().unwrap();
    adoption_fixture::publish_head(
        &repo,
        "bindings-owner",
        "fixture:rollback",
        sem_lang_repository::RepositoryRoot::Revision(parent.clone()),
    );
    let replay = write(
        &mut repo,
        "owner",
        Some(&parent),
        &[binding],
        AdoptionState::Adopted,
        "decision:adopt",
    )
    .unwrap();
    assert_eq!(replay, head);
    assert_eq!(
        repo.load_revision(&replay).unwrap().created_at,
        revision.created_at
    );
}
