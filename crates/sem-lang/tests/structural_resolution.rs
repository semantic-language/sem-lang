//! Structural owner acceptance: no lexical/source-name interpretation or implicit adoption.
use sem_lang::current::structural::{
    SourceConstraint, SourceNode, SourceShape, StructuralBudget, StructuralInput,
};
use sem_lang::current::{ProcessingError, Reason};
use sem_lang::{CompileContext, Engine, InMemoryRepository, RuntimeState};
use sem_lang_context::{ContextScope, ContextStore, ContextSubject};
use sem_lang_core::{RoleId, TermId, Value};
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition, SemanticExpr};
use sem_lang_vocab::*;

fn term(n: u64) -> TermId {
    TermId::new_random(n, 42)
}
fn role(n: u64) -> RoleId {
    RoleId::new_random(n, 43)
}
fn engine(ambiguous: bool) -> Engine {
    prepared(ambiguous, |_, _| {})
}
fn prepared(
    ambiguous: bool,
    mutate: impl FnOnce(&mut VocabularyModule, &mut RuntimeState),
) -> Engine {
    let mut terms = vec![TermDefinition {
        id: term(1),
        label: "opaque".into(),
        description: String::new(),
        deprecation: None,
        replacement: None,
        semantic_definition: Some(SemanticDefinition::Primitive(PrimitiveSemantic::Relation)),
    }];
    for n in 2..=if ambiguous { 3 } else { 2 } {
        terms.push(TermDefinition {
            id: term(n),
            label: format!("unused-{n}"),
            description: String::new(),
            deprecation: None,
            replacement: None,
            semantic_definition: Some(SemanticDefinition::Composition(SemanticExpr::Apply {
                predicate: term(1),
                arguments: vec![
                    (role(1), Box::new(SemanticExpr::Literal("@".into()))),
                    (role(2), Box::new(SemanticExpr::Variable("address".into()))),
                ],
            })),
        });
    }
    let mut module = VocabularyModule {
        schema_version: "0.10.0".into(),
        vocabulary_id: "synthetic".into(),
        version: "0.10.0".into(),
        dependencies: vec![],
        terms,
        types: vec![],
        roles: (1..=2)
            .map(|n| RoleDefinition {
                id: role(n),
                label: format!("opaque-role-{n}"),
                description: String::new(),
                cardinality: Cardinality::ExactlyOne,
            })
            .collect(),
        lexical_bindings: vec![],
    };
    let mut state = RuntimeState::new();
    mutate(&mut module, &mut state);
    let mut repository = InMemoryRepository::default();
    let revision = sem_lang_memory::definitions::adopt(
        &mut repository,
        "synthetic",
        None,
        &[module],
        "explicit-decision",
        &sem_lang_repository::RevisionRef::default(),
    )
    .unwrap();
    let refs = sem_lang::restoration::checkpoint(
        &mut repository,
        &[sem_lang::restoration::RevisionReference {
            namespace: "synthetic".into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        &state,
    )
    .unwrap();
    Engine::restore(repository, refs).unwrap()
}
fn input(engine: &Engine) -> StructuralInput {
    StructuralInput {
        reference: "source-record".into(),
        root: "root".into(),
        nodes: vec![
            SourceNode {
                identity: "root".into(),
                locator: "alpha.x1".into(),
                shape: SourceShape::Record(vec!["value".into(), "kind".into()]),
            },
            SourceNode {
                identity: "value".into(),
                locator: "alpha.x1.value".into(),
                shape: SourceShape::Value(Value::Text("a@example.test".into())),
            },
            SourceNode {
                identity: "kind".into(),
                locator: "alpha.x1.evidence".into(),
                shape: SourceShape::Value(Value::Text("@".into())),
            },
        ],
        constraints: vec![],
        context: CompileContext {
            subject: ContextSubject {
                id: "user".into(),
                class: "person".into(),
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
        },
        budget: StructuralBudget::default(),
    }
}

#[test]
fn exact_structure_locator_identity_order_independence_and_read_only_replay() {
    let e = engine(false);
    let a = input(&e);
    let before = serde_json::to_value(&e.runtime().state).unwrap();
    let one = e.resolve_structure(&a).unwrap();
    assert_eq!(one.reason, None);
    assert_eq!(one.candidates.len(), 1);
    assert_eq!(one.candidates[0].concept, term(2));
    assert_eq!(
        one.candidates[0].bindings[0].value,
        Value::Text("a@example.test".into())
    );
    assert_eq!(one, e.resolve_structure(&a).unwrap());
    let mut b = a.clone();
    b.nodes.reverse();
    for n in &mut b.nodes {
        n.locator = format!("beta.y9.{}", n.locator);
        n.identity = format!("new-{}", n.identity);
        if let SourceShape::Record(children) = &mut n.shape {
            children.reverse();
            for child in children {
                *child = format!("new-{child}");
            }
        }
    }
    b.root = "new-root".into();
    b.reference = "other-source".into();
    let two = e.resolve_structure(&b).unwrap();
    assert_eq!(one.meaning_digest, two.meaning_digest);
    assert_ne!(one.input_digest, two.input_digest);
    assert_eq!(one.candidates, two.candidates);
    assert_eq!(before, serde_json::to_value(&e.runtime().state).unwrap());
}

#[test]
fn ambiguous_missing_conflicting_evidence_and_unadopted_names_are_not_truth() {
    let e = engine(true);
    let a = input(&e);
    let result = e.resolve_structure(&a).unwrap();
    assert_eq!(result.reason, Some(Reason::Ambiguous));
    assert_eq!(result.candidates.len(), 2);
    let e = engine(false);
    let mut a = input(&e);
    a.nodes[2].shape = SourceShape::Value(Value::Text("unknown-shape".into()));
    assert_eq!(
        e.resolve_structure(&a).unwrap().reason,
        Some(Reason::UnknownExpression)
    );
    let mut a = input(&e);
    a.constraints = vec![SourceConstraint::Equal("value".into(), "kind".into())];
    assert_eq!(
        e.resolve_structure(&a).unwrap().reason,
        Some(Reason::ConflictingEvidence)
    );
    let mut a = input(&e);
    a.nodes[1].shape = SourceShape::Unknown;
    assert_eq!(
        e.resolve_structure(&a).unwrap().reason,
        Some(Reason::MissingContext)
    );
}

#[test]
fn exact_revision_bounds_and_semantic_reference_injection_fail_without_mutation() {
    let e = engine(false);
    let before = serde_json::to_value(&e.runtime().state).unwrap();
    for dimension in 0..5 {
        let mut a = input(&e);
        match dimension {
            0 => a.budget.max_nodes = 0,
            1 => a.budget.max_edges = 0,
            2 => a.budget.max_candidates = 0,
            3 => a.budget.max_steps = 0,
            _ => a.budget.max_depth = 0,
        };
        assert!(
            matches!(
                e.resolve_structure(&a),
                Err(ProcessingError::EvaluationLimitExceeded { .. })
            ),
            "dimension {dimension}"
        );
    }
    let mut a = input(&e);
    a.context.semantic_revision = "0".repeat(64);
    assert_eq!(e.resolve_structure(&a), Err(ProcessingError::StaleRevision));
    let mut a = input(&e);
    a.nodes[1].shape = SourceShape::Value(Value::TermRef(term(2)));
    assert_eq!(e.resolve_structure(&a), Err(ProcessingError::InvalidInput));
    assert_eq!(before, serde_json::to_value(&e.runtime().state).unwrap());
}

#[test]
fn contextual_constraints_use_exact_subject_scoped_claims_and_keep_conflicts() {
    use sem_lang_core::{Argument, Claim, Constraint, NodeId, Validity};
    let make = |conflict: bool| {
        prepared(false, |m, s| {
            let original = m.terms[1].semantic_definition.take().unwrap();
            let SemanticDefinition::Composition(pattern) = original else {
                panic!()
            };
            m.terms[1].semantic_definition = Some(SemanticDefinition::Composition(
                SemanticExpr::Conjunction(vec![
                    pattern,
                    SemanticExpr::Constraint(
                        Constraint::RelationConstraint {
                            value: Value::TermRef(term(1)),
                            role: role(1),
                            target: Value::Text("yes".into()),
                        }
                        .try_into()
                        .unwrap(),
                    ),
                ]),
            ));
            for n in 1..=if conflict { 2 } else { 1 } {
                let id = NodeId::new_random(n, 55);
                s.claims.insert(
                    id,
                    Claim {
                        id,
                        actor: Some(Value::Map(std::collections::BTreeMap::from([
                            ("id".into(), Value::Text("user".into())),
                            ("class".into(), Value::Text("person".into())),
                        ]))),
                        predicate: term(1),
                        arguments: vec![Argument {
                            role: Some(role(1)),
                            value: Value::Text(if n == 1 { "yes" } else { "no" }.into()),
                        }],
                        constraints: vec![],
                        validity: Validity {
                            valid_from: Some(10),
                            valid_until: Some(20),
                        },
                    },
                );
            }
        })
    };
    let e = make(false);
    let mut a = input(&e);
    assert_eq!(
        e.resolve_structure(&a).unwrap().reason,
        Some(Reason::MissingContext)
    );
    a.context.evidence.push(NodeId::new_random(1, 55));
    assert_eq!(e.resolve_structure(&a), Err(ProcessingError::InvalidInput));
    a.context.at = Some(15);
    assert_eq!(e.resolve_structure(&a).unwrap().reason, None);
    a.context.subject.id = "other".into();
    assert_eq!(
        e.resolve_structure(&a),
        Err(ProcessingError::InvalidReference)
    );
    let e = make(true);
    let mut a = input(&e);
    a.context.at = Some(15);
    a.context.evidence = vec![NodeId::new_random(1, 55), NodeId::new_random(2, 55)];
    assert_eq!(
        e.resolve_structure(&a).unwrap().reason,
        Some(Reason::ConflictingEvidence)
    );
}

#[test]
fn role_permutations_are_ambiguous_and_malformed_source_graphs_are_not_repaired() {
    let e = prepared(false, |m, _| {
        if let Some(SemanticDefinition::Composition(SemanticExpr::Apply { arguments, .. })) =
            &mut m.terms[1].semantic_definition
        {
            *arguments[0].1 = SemanticExpr::Variable("other".into());
        }
    });
    let a = input(&e);
    let r = e.resolve_structure(&a).unwrap();
    assert_eq!(r.reason, Some(Reason::Ambiguous));
    assert_eq!(r.candidates.len(), 2);
    let e = engine(false);
    let mut a = input(&e);
    a.nodes[0].shape = SourceShape::Record(vec!["root".into()]);
    assert_eq!(e.resolve_structure(&a), Err(ProcessingError::InvalidInput));
    let mut a = input(&e);
    a.nodes[0].shape = SourceShape::Record(vec!["missing".into()]);
    assert_eq!(
        e.resolve_structure(&a),
        Err(ProcessingError::InvalidReference)
    );
    let mut a = input(&e);
    a.budget.max_bytes = 1;
    assert!(matches!(
        e.resolve_structure(&a),
        Err(ProcessingError::EvaluationLimitExceeded { .. })
    ));
}

#[cfg(feature = "fs-store")]
#[test]
fn explicit_packet_admission_persists_exact_values_and_replays_after_restart() {
    use sem_lang::{FsRepository, PacketRequest, restoration};
    use sem_lang_core::{NodeId, PacketId};
    use sem_lang_repository::RevisionRef;
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let path = std::env::temp_dir().join(format!("structural-owner-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let scratch = Scratch(path);
    let module = engine(false)
        .compiled_language()
        .registry()
        .modules()
        .cloned()
        .collect::<Vec<_>>();
    let mut repo = FsRepository::create(&scratch.0).unwrap();
    let revision = sem_lang_memory::definitions::adopt(
        &mut repo,
        "synthetic",
        None,
        &module,
        "explicit-decision",
        &RevisionRef::default(),
    )
    .unwrap();
    let refs = restoration::checkpoint(
        &mut repo,
        &[restoration::RevisionReference {
            namespace: "synthetic".into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )
    .unwrap();
    use sem_lang_repository::{HeadPublication, HeadStore, RepositoryRoot, head_intent};
    repo.publish_head(
        &head_intent(
            "structural",
            "initialize",
            &RevisionRef::default(),
            &HeadPublication {
                root: RepositoryRoot::Snapshot(refs.runtime_snapshot.clone()),
                request_digest: RuntimeState::new().semantic_state_digest().unwrap(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    let e = Engine::restore(repo, refs.clone()).unwrap();
    let source = input(&e);
    let (_, packet) = e
        .structural_packet(
            &source,
            PacketId::new_random(1, 99),
            NodeId::new_random(2, 99),
            "decision:adopt",
            15,
        )
        .unwrap();
    let packet = packet.unwrap();
    assert!(e.runtime().state.claims.is_empty());
    drop(e);
    let mut repo = FsRepository::open_existing(&scratch.0).unwrap();
    let request = restoration::CheckpointRequest {
        packet: PacketRequest {
            operation_id: "source-acceptance".into(),
            expected_commit_revision: RevisionRef::default(),
            expected_semantic_digest: RuntimeState::new().semantic_state_digest().unwrap(),
        },
        expected_commit_revision: repo.head_revision("structural").unwrap(),
    };
    let first = restoration::apply(&mut repo, &refs, &packet, &request, "structural").unwrap();
    let expected = serde_json::to_value(&first).unwrap();
    drop(repo);
    let mut repo = FsRepository::open_existing(&scratch.0).unwrap();
    let replay = restoration::apply(&mut repo, &refs, &packet, &request, "structural").unwrap();
    assert_eq!(expected, serde_json::to_value(&replay).unwrap());
    let mut conflict = packet.clone();
    conflict.context_delta.as_mut().unwrap().added_claims[0].arguments[1].value =
        Value::Text("altered".into());
    assert!(restoration::apply(&mut repo, &refs, &conflict, &request, "structural").is_err());
    drop(repo);
    let restored = Engine::restore(
        FsRepository::open_existing(&scratch.0).unwrap(),
        first.restoration,
    )
    .unwrap();
    let claim = restored
        .runtime()
        .state
        .claims
        .get(&NodeId::new_random(2, 99))
        .unwrap();
    assert_eq!(claim.predicate, term(1));
    assert_eq!(
        claim.arguments[1].value,
        Value::Text("a@example.test".into())
    );
    assert_eq!(
        claim.actor,
        packet.context_delta.unwrap().added_claims[0].actor
    );
    drop(restored);
}
