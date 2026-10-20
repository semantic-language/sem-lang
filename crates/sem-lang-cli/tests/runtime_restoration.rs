#[path = "../../../test-support/adoption.rs"]
mod adoption_fixture;
#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
use sem_lang::restoration::{self, RestorationError, RestorationInput, RevisionReference};
use sem_lang::{Engine, RuntimeState};
use sem_lang_context::{ContextEngine, ContextStore};
use sem_lang_memory::bindings::{
    AdoptionState, BindingUsage, CapabilityRef, DefinitionRef, Direction, SemanticBinding,
};
use sem_lang_repository::{HeadStore, Repository};
use sem_lang_store_fs::FsRepository;
use sem_lang_vocab::VocabularyModule;

#[test]
fn exact_restoration_ignores_new_heads_and_preserves_current_memory() {
    let temporary = tempfile::tempdir().unwrap();
    let mut repo = FsRepository::create(temporary.path()).unwrap();
    let mut module: VocabularyModule = serde_json::from_str(r#"{"schema_version":"0.10.0","vocabulary_id":"example","version":"0.10.0","dependencies":[],"terms":[{"id":"42436820882964cfc19b6db3750b8d17","label":"Relation","description":"Relation","deprecation":null,"replacement":null,"semantic_definition":null}],"types":[],"roles":[],"lexical_bindings":[]}"#).unwrap();
    module.terms[0].semantic_definition =
        Some(serde_json::from_str(r#"{"Primitive":"Relation"}"#).unwrap());
    let definition = adoption_fixture::adopt(
        &mut repo,
        "owner",
        None,
        &[module.clone()],
        "decision:definition",
    )
    .unwrap();
    let binding = SemanticBinding {
        binding_id: "observe".into(),
        semantic_ref: DefinitionRef {
            namespace: "owner".into(),
            revision: definition.clone(),
            term: module.terms[0].id,
        },
        target_ref: CapabilityRef {
            namespace: "provider".into(),
            capability: "observe".into(),
            revision: None,
        },
        usage: BindingUsage::Observe,
        direction: Direction::Output,
        constraints: vec![],
        provenance_ref: "publisher:1".into(),
    };
    let mut parent = None;
    for state in [
        AdoptionState::Proposed,
        AdoptionState::Reviewed,
        AdoptionState::Adopted,
    ] {
        parent = Some(
            adoption_fixture::write(
                &mut repo,
                "owner",
                parent.as_ref(),
                std::slice::from_ref(&binding),
                state,
                &format!("decision:{state:?}"),
            )
            .unwrap(),
        );
    }
    let mut context = ContextEngine::new(ContextStore::default());
    context.bind_explicit_timezone("Asia/Tokyo", sem_lang_context::ContextScope::Session);
    let mut runtime = RuntimeState::new();
    runtime
        .annotations
        .insert("accepted".into(), "original".into());
    let input = restoration::checkpoint(
        &mut repo,
        &[RevisionReference {
            namespace: "owner".into(),
            revision: definition.clone(),
        }],
        &[RevisionReference {
            namespace: "owner".into(),
            revision: parent.clone().unwrap(),
        }],
        &context.store,
        &runtime,
    )
    .unwrap();
    adoption_fixture::publish_head(
        &repo,
        "runtime-owner",
        "fixture:runtime",
        sem_lang_repository::RepositoryRoot::Snapshot(input.runtime_snapshot.clone()),
    );
    module.terms[0].label = "Newer label must not leak".into();
    adoption_fixture::adopt(
        &mut repo,
        "owner",
        Some(&definition),
        &[module],
        "decision:new",
    )
    .unwrap();
    let mut next_binding = binding;
    next_binding.target_ref.capability = "observe/newer".into();
    let replacement = adoption_fixture::write(
        &mut repo,
        "owner",
        parent.as_ref(),
        &[next_binding],
        AdoptionState::Proposed,
        "decision:replacement",
    )
    .unwrap();
    let newer_memory = ContextStore::default()
        .save_to_repository(&mut repo)
        .unwrap();
    adoption_fixture::publish_head(
        &repo,
        "memory-owner",
        "fixture:memory",
        sem_lang_repository::RepositoryRoot::Object(newer_memory),
    );
    assert_ne!(Some(replacement), parent);
    assert_ne!(
        repo.load_head("definitions-owner").unwrap(),
        Some(definition)
    );
    let engine = Engine::restore(repo, input.clone()).unwrap();
    assert_eq!(engine.restoration_input(), Some(&input));
    assert_eq!(engine.runtime().state.annotations, runtime.annotations);
    assert_eq!(engine.adopted_bindings()[0].binding.binding_id, "observe");
    assert_eq!(
        engine.adopted_bindings()[0].binding.target_ref.capability,
        "observe"
    );
    assert_eq!(
        engine.registry().modules().next().unwrap().terms[0].label,
        "Relation"
    );
    assert_eq!(
        engine
            .context()
            .store
            .session_timezone
            .as_ref()
            .unwrap()
            .value,
        "Asia/Tokyo"
    );
    drop(engine);

    // Separate processes use exactly the same restoration/application contract,
    // without reading --language or rebuilding an empty RuntimeState.
    use sem_lang_core::{Expr, PacketId, Resolution, SemanticPacket, Value};
    let packet = SemanticPacket {
        id: PacketId::new_random(1, 1),
        conversation_id: None,
        sender: None,
        recipient: None,
        cause: None,
        correlation: None,
        act: None,
        content: Expr::Value(Value::Text("observed".into())),
        context_delta: None,
        vocabulary_manifest: None,
        resolution: Resolution::Resolved,
        pragmatics: vec![],
        provenance: None,
    };
    let directory = temporary.path();
    std::fs::write(
        directory.join("input.json"),
        serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.join("packet.json"),
        sem_lang_codec::Codec::encode_packet(&packet).unwrap(),
    )
    .unwrap();
    let initial_state = Engine::restore(
        FsRepository::open_existing(directory).unwrap(),
        input.clone(),
    )
    .unwrap()
    .runtime()
    .state
    .clone();
    let request = sem_lang::restoration::CheckpointRequest {
        expected_commit_revision: FsRepository::open_existing(directory)
            .unwrap()
            .head_revision("runtime-owner")
            .unwrap(),
        packet: sem_lang::PacketRequest {
            operation_id: "cli:packet:1".into(),
            expected_commit_revision: sem_lang::PacketReducer::commit_revision(&initial_state)
                .unwrap(),
            expected_semantic_digest: initial_state.semantic_state_digest().unwrap(),
        },
    };
    std::fs::write(
        directory.join("request.json"),
        serde_json::to_vec(&request).unwrap(),
    )
    .unwrap();
    let run = |input_file: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_sem-lang-cli"))
            .current_dir(directory)
            .args([
                "--format",
                "json",
                "--language",
                "absent.sem",
                "packet",
                "apply",
                "packet.json",
                "--repo",
                ".",
                "--head",
                "runtime-owner",
                "--restoration",
                input_file,
                "--request",
                "request.json",
            ])
            .output()
            .unwrap()
    };
    let applied = run("input.json");
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let applied: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    let current: RestorationInput = serde_json::from_value(applied["restoration"].clone()).unwrap();
    std::fs::write(
        directory.join("current.json"),
        serde_json::to_vec(&current).unwrap(),
    )
    .unwrap();
    // A subsequent committed state must survive replay of the earlier operation.
    let mut later_repo = FsRepository::open_existing(directory).unwrap();
    let mut later_packet = packet.clone();
    later_packet.id = PacketId::new_random(2, 2);
    let current_state = {
        drop(later_repo);
        let e = Engine::restore(
            FsRepository::open_existing(directory).unwrap(),
            current.clone(),
        )
        .unwrap();
        e.runtime().state.clone()
    };
    later_repo = FsRepository::open_existing(directory).unwrap();
    let later_request = restoration::CheckpointRequest {
        expected_commit_revision: later_repo.head_revision("runtime-owner").unwrap(),
        packet: sem_lang::PacketRequest {
            operation_id: "cli:packet:2".into(),
            expected_commit_revision: sem_lang::PacketReducer::commit_revision(&current_state)
                .unwrap(),
            expected_semantic_digest: current_state.semantic_state_digest().unwrap(),
        },
    };
    let later = restoration::apply(
        &mut later_repo,
        &current,
        &later_packet,
        &later_request,
        "runtime-owner",
    )
    .unwrap();
    let later_revision = later_repo.head_revision("runtime-owner").unwrap();
    assert_eq!(
        later_repo.load_head("runtime-owner").unwrap(),
        Some(later.restoration.runtime_snapshot.clone())
    );
    assert!(later_repo.verify().unwrap().errors.is_empty());
    drop(later_repo);
    let canonical_file = directory.join(".sem-lang/commits.json");
    let canonical_bytes = std::fs::read(&canonical_file).unwrap();
    let canonical_mtime = std::fs::metadata(&canonical_file)
        .unwrap()
        .modified()
        .unwrap();
    for _ in 0..100 {
        let retried = run("input.json");
        assert!(
            retried.status.success(),
            "{}",
            String::from_utf8_lossy(&retried.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&retried.stdout).unwrap(),
            applied
        );
        assert!(!run("current.json").status.success());
    }
    assert_eq!(std::fs::read(&canonical_file).unwrap(), canonical_bytes);
    assert_eq!(
        std::fs::metadata(&canonical_file)
            .unwrap()
            .modified()
            .unwrap(),
        canonical_mtime
    );
    assert_eq!(
        FsRepository::open_existing(directory)
            .unwrap()
            .head_revision("runtime-owner")
            .unwrap(),
        later_revision
    );
    let retried = run("input.json");
    assert!(
        retried.status.success(),
        "{}",
        String::from_utf8_lossy(&retried.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&retried.stdout).unwrap(),
        applied
    );
    assert!(
        !run("current.json").status.success(),
        "stale head must not overwrite current state"
    );
    let engine = Engine::restore(FsRepository::open_existing(directory).unwrap(), current).unwrap();
    assert_eq!(
        engine
            .runtime()
            .state
            .annotations
            .get("accepted")
            .map(String::as_str),
        Some("original")
    );
    assert_eq!(
        engine.runtime().state.packets.get(&packet.id),
        Some(&packet)
    );
    drop(engine);

    // Each failure must remain visible. A newer valid head must not repair it.
    for kind in ["definition", "binding", "memory", "runtime"] {
        let mut bad = input.clone();
        match kind {
            "definition" => bad.definitions[0].revision = "definitions-absent".into(),
            "binding" => bad.bindings[0].revision = "bindings-absent".into(),
            "memory" => bad.memory = "absent".into(),
            _ => bad.runtime_snapshot = "runtime-absent".into(),
        }
        let error = Engine::restore(FsRepository::open_existing(directory).unwrap(), bad)
            .err()
            .unwrap();
        assert!(
            matches!(error, RestorationError::MissingReference { kind: actual, .. } if actual == kind),
            "{error}"
        );
    }
    let mut wrong = input.clone();
    wrong.bindings[0].revision = wrong.definitions[0].revision.clone();
    assert!(matches!(
        Engine::restore(FsRepository::open_existing(directory).unwrap(), wrong),
        Err(RestorationError::WrongObjectKind("binding"))
    ));
    let mut mismatch = input.clone();
    {
        let mut repo = FsRepository::open_existing(directory).unwrap();
        mismatch.memory = ContextStore::default()
            .save_to_repository(&mut repo)
            .unwrap();
    }
    assert!(matches!(
        Engine::restore(FsRepository::open_existing(directory).unwrap(), mismatch),
        Err(RestorationError::IncompatibleReferences)
    ));

    let snapshot_file = directory
        .join(".sem-lang/snapshots")
        .join(&input.runtime_snapshot);
    let original = std::fs::read(&snapshot_file).unwrap();
    let dto: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut wrong_revision = dto.clone();
    wrong_revision["revision"] = serde_json::json!("runtime-other");
    std::fs::write(&snapshot_file, serde_json::to_vec(&wrong_revision).unwrap()).unwrap();
    assert!(matches!(
        Engine::restore(
            FsRepository::open_existing(directory).unwrap(),
            input.clone()
        ),
        Err(RestorationError::RevisionMismatch("runtime"))
    ));
    let mut tampered = dto.clone();
    tampered["serialized"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(32));
    std::fs::write(&snapshot_file, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert!(matches!(
        Engine::restore(
            FsRepository::open_existing(directory).unwrap(),
            input.clone()
        ),
        Err(RestorationError::DigestMismatch { .. })
    ));
    std::fs::write(&snapshot_file, b"not json").unwrap();
    assert!(matches!(
        Engine::restore(
            FsRepository::open_existing(directory).unwrap(),
            input.clone()
        ),
        Err(RestorationError::CorruptObject {
            kind: "runtime",
            ..
        })
    ));
    std::fs::write(&snapshot_file, original).unwrap();
    let definition_file = directory
        .join(".sem-lang/revisions")
        .join(&input.definitions[0].revision);
    let original = std::fs::read(&definition_file).unwrap();
    let mut invalid_parent: serde_json::Value = serde_json::from_slice(&original).unwrap();
    invalid_parent["parent"] = serde_json::json!("definitions-unrelated");
    std::fs::write(
        &definition_file,
        serde_json::to_vec(&invalid_parent).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        Engine::restore(
            FsRepository::open_existing(directory).unwrap(),
            input.clone()
        ),
        Err(RestorationError::InvalidRevision(_))
    ));
    std::fs::write(&definition_file, original).unwrap();
    assert!(Engine::restore(FsRepository::open_existing(directory).unwrap(), input).is_ok());
}
