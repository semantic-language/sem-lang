//! Separate-process restoration using owned product source and official external dependencies.
use sem_lang::{
    Engine, PacketReducer, PacketRequest, RuntimeState, SemanticPacket, Value as SemanticValue,
    restoration::{self, RestorationInput, RevisionReference},
};
use sem_lang_context::{
    ContextEngine, ContextKind, ContextReference, ContextScope, ContextStatus, ContextStore,
    ContextSubject, PromotionOperation, PromotionRequest,
};
use sem_lang_core::{Expr, Resolution};
use sem_lang_memory::{bindings::*, definitions};
use sem_lang_repository::{HeadStore, ObjectStore};
use sem_lang_store_fs::FsRepository;
use sem_lang_vocab::VocabularyModule;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Seek},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

// Each test process is synchronous, bounded and reaped on every exit path.
struct Running {
    child: Option<Child>,
    stdout: fs::File,
    stderr: fs::File,
}

impl Running {
    fn start(root: &Path, mode: &str) -> io::Result<Self> {
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "b9_registry_cross_process_restore_and_corruption_rejection",
                "--nocapture",
            ])
            .env("SEM_BINDING_CONFORMANCE_ROOT", root)
            .env("SEM_BINDING_CONFORMANCE_MODE", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout.try_clone()?))
            .stderr(Stdio::from(stderr.try_clone()?))
            .spawn()?;
        Ok(Self {
            child: Some(child),
            stdout,
            stderr,
        })
    }

    fn finish(mut self, limit: Duration) -> io::Result<Output> {
        let deadline = Instant::now() + limit;
        loop {
            let status = self
                .child
                .as_mut()
                .ok_or_else(|| io::Error::other("child already reaped"))?
                .try_wait()?;
            if self.stdout.metadata()?.len() > 1_048_576
                || self.stderr.metadata()?.len() > 1_048_576
            {
                self.terminate()?;
                return Err(io::Error::other("registry fixture output limit"));
            }
            if let Some(status) = status {
                self.child.take();
                self.stdout.rewind()?;
                self.stderr.rewind()?;
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                self.stdout.read_to_end(&mut stdout)?;
                self.stderr.read_to_end(&mut stderr)?;
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            if Instant::now() >= deadline {
                self.terminate()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "registry fixture deadline",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn terminate(&mut self) -> io::Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        let kill = child.kill();
        let wait = child.wait();
        wait?;
        if let Err(error) = kill
            && error.kind() != io::ErrorKind::InvalidInput
        {
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

fn image(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn walk(root: &Path, at: &Path, entries: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            let directory = path.is_dir();
            entries.push((
                path.strip_prefix(root).unwrap().to_owned(),
                if directory {
                    None
                } else {
                    Some(fs::read(&path).unwrap())
                },
            ));
            if directory {
                walk(root, &path, entries);
            }
        }
    }
    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries.sort();
    entries
}

fn context_operation(
    engine: &mut ContextEngine,
    id: &str,
    scope: ContextScope,
    operation: sem_lang_context::ContextOperation,
) {
    let request = sem_lang_context::ContextRequest {
        operation_id: id.into(),
        expected_commit_revision: engine.commit_revision().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        subject_ref: ContextSubject {
            id: "person:alice".into(),
            class: "person".into(),
        },
        scope,
        operation,
    };
    engine.apply(&request).unwrap();
}

fn packet(id: u64, text: &str) -> SemanticPacket {
    SemanticPacket {
        id: sem_lang::PacketId::new_random(id, 1),
        conversation_id: None,
        sender: None,
        recipient: None,
        cause: None,
        correlation: None,
        act: None,
        content: Expr::Value(SemanticValue::Text(text.into())),
        context_delta: None,
        vocabulary_manifest: None,
        resolution: Resolution::Resolved,
        pragmatics: vec![],
        provenance: None,
    }
}
fn packet_request(state: &RuntimeState, id: &str) -> PacketRequest {
    PacketRequest {
        operation_id: id.into(),
        expected_commit_revision: PacketReducer::commit_revision(state).unwrap(),
        expected_semantic_digest: state.semantic_state_digest().unwrap(),
    }
}
fn head_receipt(repository: &FsRepository, domain: &str, id: &str) -> Value {
    json!(
        sem_lang_repository::CommitStore::new(sem_lang_repository::commit_image::BorrowedBackend(
            repository
        ))
        .receipt(domain, id)
        .unwrap()
        .unwrap()
    )
}
fn process(root: &Path, mode: &str) {
    if mode == "seed" {
        let mut repository = FsRepository::create(root).unwrap();
        let mut context = ContextEngine::new(ContextStore::default());
        for i in 0..2 {
            context_operation(
                &mut context,
                &format!("observe:london:{i}"),
                ContextScope::PersistentAgentDefault,
                sem_lang_context::ContextOperation::ObserveTimezone {
                    value: "Europe/London".into(),
                    evidence_ref: format!("evidence:london:{i}"),
                },
            );
        }
        let candidate = context.induce_defaults().remove(0);
        let request = PromotionRequest {
            subject_ref: ContextSubject {
                id: "person:alice".into(),
                class: "person".into(),
            },
            candidate_ref: candidate.reference().unwrap(),
            expected_memory_revision: context.memory_revision().unwrap(),
            expected_commit_revision: context.commit_revision().unwrap(),
            decision_ref: "owner:permanent".into(),
            operation: PromotionOperation::PromoteLongTerm,
        };
        let receipt = context.promote(candidate.clone(), &request).unwrap();
        context_operation(
            &mut context,
            "bind:tokyo",
            ContextScope::CurrentRequest,
            sem_lang_context::ContextOperation::BindTimezone {
                value: "Asia/Tokyo".into(),
            },
        );
        for i in 0..2 {
            context_operation(
                &mut context,
                &format!("observe:tokyo:{i}"),
                ContextScope::PersistentAgentDefault,
                sem_lang_context::ContextOperation::ObserveTimezone {
                    value: "Asia/Tokyo".into(),
                    evidence_ref: format!("evidence:tokyo:{i}"),
                },
            );
        }
        for (i, scope) in [
            ContextScope::CurrentRequest,
            ContextScope::Session,
            ContextScope::PersistentAgentDefault,
        ]
        .into_iter()
        .enumerate()
        {
            context_operation(
                &mut context,
                &format!("clear:{i}"),
                scope,
                sem_lang_context::ContextOperation::ClearShortTerm,
            );
        }
        let mut module: VocabularyModule = serde_json::from_str(r#"{"schema_version":"0.10.0","vocabulary_id":"example","version":"0.10.0","dependencies":[],"terms":[{"id":"42436820882964cfc19b6db3750b8d17","label":"Relation","description":"Relation","deprecation":null,"replacement":null,"semantic_definition":null}],"types":[],"roles":[],"lexical_bindings":[]}"#).unwrap();
        module.terms[0].semantic_definition =
            Some(serde_json::from_str(r#"{"Primitive":"Relation"}"#).unwrap());
        let original_module = module.clone();
        let term = module.terms[0].id;
        let revision = definitions::adopt(
            &mut repository,
            "owner",
            None,
            std::slice::from_ref(&module),
            "decision:definition",
            &sem_lang_repository::RevisionRef::default(),
        )
        .unwrap();
        let binding = SemanticBinding {
            binding_id: "residence".into(),
            semantic_ref: DefinitionRef {
                namespace: "owner".into(),
                revision: revision.clone(),
                term,
            },
            target_ref: CapabilityRef {
                namespace: "presentation".into(),
                capability: "location".into(),
                revision: None,
            },
            usage: BindingUsage::Presentation,
            direction: Direction::Output,
            constraints: vec![],
            provenance_ref: "publisher:1".into(),
        };
        assert!(
            resolve(&repository, "owner", &binding.semantic_ref)
                .unwrap()
                .is_empty()
        );
        let mut previous = None;
        let mut binding_requests = vec![];
        for state in [
            AdoptionState::Proposed,
            AdoptionState::Reviewed,
            AdoptionState::Adopted,
        ] {
            let commit_base = repository.head_revision("bindings-owner").unwrap();
            let revision = write(
                &mut repository,
                "owner",
                previous.as_ref(),
                std::slice::from_ref(&binding),
                state,
                &format!("decision:binding:{state:?}"),
                &commit_base,
            )
            .unwrap();
            binding_requests.push(json!({"parent":previous,"state":state,"base":commit_base,
                "id":format!("decision:binding:{state:?}"),"revision":revision,
                "receipt":head_receipt(&repository,"bindings-owner",&format!("decision:binding:{state:?}"))}));
            previous = Some(revision);
        }
        let fact = repository
            .put_object(b"separate provider fact".to_vec())
            .unwrap();
        let initial = RuntimeState::new();
        let original_packet = packet(1, "first meaning");
        let packet_request = packet_request(&initial, "packet:first");
        let applied = PacketReducer::reduce(&initial, &original_packet, &packet_request).unwrap();
        let packet_receipt = applied.receipt().clone();
        let runtime = applied.into_state();
        let runtime = PacketReducer::reduce(
            &runtime,
            &packet(2, "later meaning"),
            &self::packet_request(&runtime, "packet:later"),
        )
        .unwrap()
        .into_state();
        let input = restoration::checkpoint(
            &mut repository,
            &[RevisionReference {
                namespace: "owner".into(),
                revision: revision.clone(),
            }],
            &[RevisionReference {
                namespace: "owner".into(),
                revision: previous.unwrap(),
            }],
            &context.store,
            &runtime,
        )
        .unwrap();
        let expected = json!({"input": input, "candidate": candidate, "request": request,
            "receipt": receipt, "memory": context.store, "fact": fact,
            "module":original_module,"definition_revision":revision,
            "definition_receipt":head_receipt(&repository,"definitions-owner","decision:definition"),
            "binding":binding,"binding_requests":binding_requests,
            "runtime":runtime,"packet":original_packet,"packet_request":packet_request,"packet_receipt":packet_receipt});
        // Change both heads before exiting. Restoration must not follow either.
        module.terms[0].label = "Newer unrelated label".into();
        let definition_base = repository.head_revision("definitions-owner").unwrap();
        definitions::adopt(
            &mut repository,
            "owner",
            Some(&revision),
            &[module],
            "definition:newer",
            &definition_base,
        )
        .unwrap();
        let current_binding = read(&repository, "owner").unwrap().unwrap().0;
        let mut changed_binding = binding;
        changed_binding.target_ref.capability = "different".into();
        let binding_base = repository.head_revision("bindings-owner").unwrap();
        write(
            &mut repository,
            "owner",
            Some(&current_binding),
            &[changed_binding],
            AdoptionState::Proposed,
            "binding:newer",
            &binding_base,
        )
        .unwrap();
        fs::write(
            root.join("expected.json"),
            serde_json::to_vec(&expected).unwrap(),
        )
        .unwrap();
    } else {
        let mut repository =
            FsRepository::open_existing(root).expect("open existing registry fixture");
        let expected: Value =
            serde_json::from_slice(&fs::read(root.join("expected.json")).unwrap()).unwrap();
        let input: RestorationInput = serde_json::from_value(expected["input"].clone()).unwrap();
        assert_eq!(
            repository
                .get_object(&expected["fact"].as_str().unwrap().into())
                .unwrap()
                .unwrap(),
            b"separate provider fact"
        );
        let before = image(root);
        let definition_head = repository.head_revision("definitions-owner").unwrap();
        let binding_head = repository.head_revision("bindings-owner").unwrap();
        for _ in 0..100 {
            assert_eq!(
                json!(
                    definitions::adopt(
                        &mut repository,
                        "owner",
                        None,
                        &[serde_json::from_value(expected["module"].clone()).unwrap()],
                        "decision:definition",
                        &sem_lang_repository::RevisionRef::default()
                    )
                    .unwrap()
                ),
                expected["definition_revision"]
            );
            assert_eq!(
                head_receipt(&repository, "definitions-owner", "decision:definition"),
                expected["definition_receipt"]
            );
            for request in expected["binding_requests"].as_array().unwrap() {
                let parent: Option<String> =
                    serde_json::from_value(request["parent"].clone()).unwrap();
                assert_eq!(
                    json!(
                        write(
                            &mut repository,
                            "owner",
                            parent.as_ref(),
                            &[serde_json::from_value(expected["binding"].clone()).unwrap()],
                            serde_json::from_value(request["state"].clone()).unwrap(),
                            request["id"].as_str().unwrap(),
                            &serde_json::from_value(request["base"].clone()).unwrap()
                        )
                        .unwrap()
                    ),
                    request["revision"]
                );
                assert_eq!(
                    head_receipt(
                        &repository,
                        "bindings-owner",
                        request["id"].as_str().unwrap()
                    ),
                    request["receipt"]
                );
            }
        }
        assert_eq!(
            repository.head_revision("definitions-owner").unwrap(),
            definition_head
        );
        assert_eq!(
            repository.head_revision("bindings-owner").unwrap(),
            binding_head
        );
        assert_eq!(image(root), before);
        drop(repository);
        let mut engine =
            Engine::restore(FsRepository::open_existing(root).unwrap(), input.clone()).unwrap();
        let references = [ContextReference::Default(
            expected["memory"]["persistent_defaults"][0]["id"]
                .as_str()
                .unwrap()
                .into(),
        )];
        let current = engine
            .context()
            .read_current(
                &references,
                &serde_json::from_value(expected["request"]["subject_ref"].clone()).unwrap(),
                ContextScope::CurrentRequest,
                None,
            )
            .unwrap();
        assert_eq!(current.entries.len(), 1);
        assert_eq!(current.entries[0].status, ContextStatus::Applicable);
        assert_eq!(
            current.select(ContextKind::TimeZone, &[]).values,
            ["Europe/London"]
        );
        assert!(engine.context().induce_defaults().is_empty());
        assert_eq!(json!(engine.context().store), expected["memory"]);
        assert_eq!(engine.restoration_input(), Some(&input));
        assert_eq!(engine.adopted_bindings().len(), 1);
        assert_eq!(
            engine.adopted_bindings()[0].binding.target_ref.capability,
            "location"
        );
        for _ in 0..100 {
            let retry = engine
                .context_mut()
                .promote(
                    serde_json::from_value(expected["candidate"].clone()).unwrap(),
                    &serde_json::from_value(expected["request"].clone()).unwrap(),
                )
                .unwrap();
            assert_eq!(json!(retry), expected["receipt"]);
            assert_eq!(json!(engine.context().store), expected["memory"]);
        }
        let runtime: RuntimeState = serde_json::from_value(expected["runtime"].clone()).unwrap();
        let original_packet = serde_json::from_value(expected["packet"].clone()).unwrap();
        let packet_request = serde_json::from_value(expected["packet_request"].clone()).unwrap();
        for _ in 0..100 {
            let replay =
                PacketReducer::reduce(&runtime, &original_packet, &packet_request).unwrap();
            assert_eq!(json!(replay.receipt()), expected["packet_receipt"]);
            assert_eq!(json!(replay.into_state()), expected["runtime"]);
        }
    }
}

#[test]
fn b9_registry_cross_process_restore_and_corruption_rejection() {
    if let Ok(mode) = std::env::var("SEM_BINDING_CONFORMANCE_MODE") {
        if mode == "stall" {
            std::thread::sleep(Duration::from_secs(60));
            panic!("parent failed to reap stalled child");
        }
        process(
            Path::new(&std::env::var("SEM_BINDING_CONFORMANCE_ROOT").unwrap()),
            &mode,
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let invoke = |mode| {
        Running::start(directory.path(), mode)
            .unwrap()
            .finish(Duration::from_secs(20))
            .unwrap()
    };
    let absent = image(directory.path());
    let result = invoke("restore");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("open existing registry fixture"));
    assert_eq!(image(directory.path()), absent);
    for mode in ["seed", "restore"] {
        let before = image(directory.path());
        let result = invoke(mode);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty());
        if mode == "restore" {
            assert_eq!(image(directory.path()), before);
        }
    }
    let expected: Value =
        serde_json::from_slice(&fs::read(directory.path().join("expected.json")).unwrap()).unwrap();
    let digest = expected["input"]["bindings"][0]["revision"]
        .as_str()
        .unwrap()
        .strip_prefix("bindings-")
        .unwrap();
    let object = directory
        .path()
        .join(".sem-lang/objects/sha256")
        .join(&digest[..4])
        .join(&digest[4..]);
    fs::write(object, b"corrupted").unwrap();
    let corrupt = image(directory.path());
    let result = invoke("restore");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("DigestMismatch"));
    assert_eq!(image(directory.path()), corrupt);
    let stalled = Running::start(directory.path(), "stall").unwrap();
    let result = stalled.finish(Duration::from_millis(100)).unwrap_err();
    assert_eq!(result.kind(), io::ErrorKind::TimedOut);
    assert_eq!(image(directory.path()), corrupt);
}
