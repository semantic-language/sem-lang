//! C1/C2 conformance: real adopted owner records, one bounded pure specimen.
// Self-contained packaged conformance fixture. Only the fixture explicitly
// selects its current CAS base; production resolution never performs adoption.
mod adoption {
    use sem_lang_memory::{
        bindings::{self, AdoptionState, SemanticBinding},
        definitions,
    };
    use sem_lang_repository::{Backend, Repository, RepositoryError, RevisionId};
    pub fn adopt<R: Repository + Backend>(
        repo: &mut R,
        ns: &str,
        parent: Option<&RevisionId>,
        modules: &[sem_lang_vocab::VocabularyModule],
        decision: &str,
    ) -> Result<RevisionId, RepositoryError> {
        let base = repo.head_revision(&format!("definitions-{ns}"))?;
        definitions::adopt(repo, ns, parent, modules, decision, &base)
    }
    pub fn write<R: Repository + Backend>(
        repo: &mut R,
        ns: &str,
        parent: Option<&RevisionId>,
        bindings: &[SemanticBinding],
        state: AdoptionState,
        decision: &str,
    ) -> Result<RevisionId, RepositoryError> {
        let base = repo.head_revision(&format!("bindings-{ns}"))?;
        bindings::write(repo, ns, parent, bindings, state, decision, &base)
    }
}
use sem_lang_core::{TypeId, Value};
use sem_lang_execution::semantic::*;
use sem_lang_memory::bindings::{self, *};
use sem_lang_store_memory::InMemoryRepository;
use sem_lang_vocab::VocabularyModule;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn fixture() -> (
    InMemoryRepository,
    RevisionedBinding,
    ExecutionTarget,
    SemanticOperationContract,
    TypedRequest,
) {
    fixture_kind(TargetKind::PureBuiltin)
}
fn fixture_kind(
    kind: TargetKind,
) -> (
    InMemoryRepository,
    RevisionedBinding,
    ExecutionTarget,
    SemanticOperationContract,
    TypedRequest,
) {
    let mut repo = InMemoryRepository::new();
    let module: VocabularyModule = serde_json::from_value(serde_json::json!({
        "schema_version":"0.10.0","vocabulary_id":"specimen","version":"0.10.0","dependencies":[],
        "terms":[{"id":"42436820882964cfc19b6db3750b8d17","label":"Identity","description":"Test only","deprecation":null,"replacement":null,"semantic_definition":{"Primitive":"Relation"}}],
        "types":[{"id":"52436820882964cfc19b6db3750b8d17","label":"Specimen scalar","description":"Test only","super_types":[],"deprecation":null}],"roles":[{"id":"62436820882964cfc19b6db3750b8d17","label":"item","description":"Test only","cardinality":"ExactlyOne"}],"lexical_bindings":[]
    })).unwrap();
    let term = module.terms[0].id;
    let revision =
        adoption::adopt(&mut repo, "owner", None, &[module], "decision:definition").unwrap();
    let ty = ExactTypeRef {
        namespace: "owner".into(),
        revision: revision.clone(),
        id: TypeId::from_hex("52436820882964cfc19b6db3750b8d17").unwrap(),
    };
    let spec = TypeRequirement {
        reference: ty.clone(),
        kind: ValueKind::Integer,
    };
    let contract = SemanticOperationContract {
        owner: "specimen".into(),
        id: "identity-contract".into(),
        inputs: BTreeMap::from([("value".into(), spec.clone())]),
        input_roles: BTreeMap::new(),
        output: spec,
    };
    let target = ExecutionTarget {
        owner: "specimen".into(),
        id: "identity-A".into(),
        label: "identity".into(),
        kind,
        operation: DefinitionRef {
            namespace: "owner".into(),
            revision,
            term,
        },
        implementation: ExactRef {
            owner: "specimen".into(),
            id: "identity-code".into(),
            revision: "implementation:1".into(),
        },
        contract: contract.exact_ref().unwrap(),
    };
    let b = SemanticBinding {
        binding_id: "specimen".into(),
        semantic_ref: target.operation.clone(),
        target_ref: CapabilityRef {
            namespace: target.owner.clone(),
            capability: target.id.clone(),
            revision: Some(target.exact_ref().unwrap().revision),
        },
        usage: BindingUsage::Action,
        direction: Direction::Input,
        constraints: vec![],
        provenance_ref: "test:declaration".into(),
    };
    let mut head = None;
    for (state, decision) in [
        (AdoptionState::Proposed, "decision:propose"),
        (AdoptionState::Reviewed, "decision:review"),
        (AdoptionState::Adopted, "decision:adopt"),
    ] {
        head = Some(
            adoption::write(
                &mut repo,
                "owner",
                head.as_ref(),
                std::slice::from_ref(&b),
                state,
                decision,
            )
            .unwrap(),
        );
    }
    let binding = bindings::resolve(&repo, "owner", &b.semantic_ref)
        .unwrap()
        .remove(0);
    let request = TypedRequest {
        contract_ref: contract.exact_ref().unwrap(),
        arguments: BTreeMap::from([(
            "value".into(),
            TypedValue {
                value: Value::Integer(2),
                type_ref: ty,
            },
        )]),
    };
    (repo, binding, target, contract, request)
}

#[test]
fn semantic_arguments_require_exact_roles_not_equal_field_names() {
    let (repo, _, _, mut contract, input) = fixture();
    let reference = contract.output.reference.clone();
    let role = ExactRoleRef {
        namespace: reference.namespace,
        revision: reference.revision,
        id: sem_lang_core::RoleId::from_hex("62436820882964cfc19b6db3750b8d17").unwrap(),
    };
    contract.input_roles.insert("value".into(), role.clone());
    let arguments = vec![SemanticArgument {
        role: role.clone(),
        value: input.arguments["value"].clone(),
    }];
    let request = bind_semantic_arguments(&repo, &contract, &arguments).unwrap();
    assert_eq!(request.arguments, input.arguments);
    assert_ne!(request.contract_ref, input.contract_ref);
    let mut wrong = arguments.clone();
    wrong[0].role.namespace = "different-owner".into();
    assert_eq!(
        bind_semantic_arguments(&repo, &contract, &wrong),
        Err(ContractError::Role)
    );
    let mut other = contract.clone();
    other.input_roles.get_mut("value").unwrap().namespace = "different-owner".into();
    assert_ne!(other.exact_ref().unwrap(), contract.exact_ref().unwrap());
    assert_eq!(
        bind_semantic_arguments(&repo, &other, &arguments),
        Err(ContractError::Role)
    );
    assert_eq!(
        bind_semantic_arguments(
            &repo,
            &contract,
            &[arguments[0].clone(), arguments[0].clone()]
        ),
        Err(ContractError::Role)
    );
    assert!(bind_semantic_arguments(&repo, &contract, &[]).is_err());
}

#[test]
fn execution_class_and_result_admissibility_are_not_effect_metadata() {
    let (_, _, target, _, _) = fixture();
    let mut identities = std::collections::BTreeSet::new();
    for (kind, class, result) in [
        (
            TargetKind::PureBuiltin,
            ExecutionClass::Pure,
            ResultDisposition::DeterministicValue,
        ),
        (
            TargetKind::PureModule,
            ExecutionClass::Pure,
            ResultDisposition::DeterministicValue,
        ),
        (
            TargetKind::InferenceRuntime,
            ExecutionClass::Inference,
            ResultDisposition::UnadoptedObservation,
        ),
        (
            TargetKind::Capability,
            ExecutionClass::Effect,
            ResultDisposition::ExternalObservation,
        ),
    ] {
        let mut t = target.clone();
        t.kind = kind;
        assert_eq!(t.execution_class(), class);
        assert_eq!(class.result_disposition(), result);
        assert!(identities.insert(t.exact_ref().unwrap().revision));
    }
}

#[test]
fn inference_preparation_fences_adoption_request_and_result_without_admission() {
    use sem_lang_repository::HeadStore;
    let (repo, binding, target, contract, request) = fixture_kind(TargetKind::InferenceRuntime);
    let before = repo.head_revision("bindings-owner").unwrap();
    let executor = InferenceExecutor::new(target.clone(), contract.clone()).unwrap();
    let plan = executor
        .prepare(&repo, "owner", &binding, &request)
        .unwrap();
    assert_eq!(plan.target_ref(), &target.exact_ref().unwrap());
    assert_eq!(plan.binding_ref().revision, binding.revision);
    let result = TypedResult {
        contract_ref: contract.exact_ref().unwrap(),
        output: request.arguments["value"].clone(),
    };
    assert_eq!(
        executor.observe(&repo, &plan, &request, &result).unwrap(),
        ResultDisposition::UnadoptedObservation
    );
    let other = InferenceExecutor::new(target.clone(), contract.clone()).unwrap();
    assert!(matches!(
        other.validate(&repo, &plan, &request),
        Err(ContractError::StalePlan)
    ));
    let mut mutated = request.clone();
    mutated.arguments.get_mut("value").unwrap().value = Value::Integer(3);
    assert!(matches!(
        executor.validate(&repo, &plan, &mutated),
        Err(ContractError::StalePlan)
    ));
    let mut invalid = result.clone();
    invalid.output.value = Value::Text("not an integer".into());
    assert!(executor.observe(&repo, &plan, &request, &invalid).is_err());
    let mut stale = binding.clone();
    stale.revision.push('x');
    assert!(executor.prepare(&repo, "owner", &stale, &request).is_err());
    for kind in [
        TargetKind::PureBuiltin,
        TargetKind::PureModule,
        TargetKind::Capability,
    ] {
        let mut rejected = target.clone();
        rejected.kind = kind;
        assert!(matches!(
            InferenceExecutor::new(rejected, contract.clone()),
            Err(ContractError::AuthorizationRequired)
        ));
    }
    assert!(PureExecutor::new(target, contract, |_| Err(ContractError::Handler)).is_err());
    assert_eq!(before, repo.head_revision("bindings-owner").unwrap());
}

#[test]
fn exact_adoption_target_plan_and_typed_output_share_one_compound_journey() {
    let (mut repo, binding, target, contract, request) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let output = contract.output.reference.clone();
    let cref = contract.exact_ref().unwrap();
    let executor = PureExecutor::new(target.clone(), contract.clone(), move |r| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(TypedResult {
            contract_ref: cref.clone(),
            output: TypedValue {
                type_ref: output.clone(),
                value: r.arguments["value"].value.clone(),
            },
        })
    })
    .unwrap();
    let plan = executor
        .prepare(&repo, "owner", &binding, &request)
        .unwrap();
    let mut bad_binding = binding.clone();
    bad_binding.revision.push('x');
    assert!(
        executor
            .prepare(&repo, "owner", &bad_binding, &request)
            .is_err()
    );
    let mut missing = binding.clone();
    missing.binding.target_ref.revision = None;
    assert!(
        executor
            .prepare(&repo, "owner", &missing, &request)
            .is_err()
    );
    assert!(
        executor
            .prepare(&repo, "not-adopted", &binding, &request)
            .is_err()
    );
    let mut other = target.clone();
    other.id = "identity-B".into();
    let other =
        PureExecutor::new(other, contract.clone(), |_| panic!("wrong target called")).unwrap();
    assert!(other.prepare(&repo, "owner", &binding, &request).is_err());
    assert!(other.invoke(&repo, &plan, &request).is_err());
    let mut replacement = target.clone();
    replacement.implementation.revision = "implementation:2".into();
    let replacement = PureExecutor::new(replacement, contract.clone(), |_| {
        panic!("replacement called")
    })
    .unwrap();
    assert!(
        replacement
            .prepare(&repo, "owner", &binding, &request)
            .is_err()
    );
    let mut changed = request.clone();
    changed.arguments.get_mut("value").unwrap().value = Value::Integer(3);
    assert!(executor.invoke(&repo, &plan, &changed).is_err());
    for kind in [
        TargetKind::PureModule,
        TargetKind::InferenceRuntime,
        TargetKind::Capability,
    ] {
        let mut t = target.clone();
        t.kind = kind;
        assert!(PureExecutor::new(t, contract.clone(), |_| panic!("non-pure called")).is_err());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        executor
            .invoke(&repo, &plan, &request)
            .unwrap()
            .output
            .value,
        Value::Integer(2)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (head, record) = bindings::read(&repo, "owner").unwrap().unwrap();
    let old = serde_json::to_vec(&record).unwrap();
    let mut optional = record.bindings[0].binding.clone();
    optional.target_ref.revision = None;
    let decoded: SemanticBinding =
        serde_json::from_slice(&serde_json::to_vec(&optional).unwrap()).unwrap();
    assert_eq!(decoded, optional);
    adoption::write(
        &mut repo,
        "owner",
        Some(&head),
        std::slice::from_ref(&binding.binding),
        AdoptionState::Rejected,
        "decision:revoke",
    )
    .unwrap();
    assert!(executor.invoke(&repo, &plan, &request).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        serde_json::to_vec(&bindings::read_revision(&repo, "owner", &head).unwrap()).unwrap(),
        old
    );
}

#[test]
fn input_and_result_negative_matrix_never_exposes_invalid_values() {
    let (repo, binding, target, contract, request) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let executor = PureExecutor::new(target.clone(), contract.clone(), move |_| {
        c.fetch_add(1, Ordering::SeqCst);
        Err(ContractError::Handler)
    })
    .unwrap();
    let mut deep = Value::Nil;
    for _ in 0..18 {
        deep = Value::Sequence(vec![deep]);
    }
    let mut negatives = vec![
        Value::Text("2".into()),
        Value::Text("x".repeat(4097)),
        Value::Sequence(vec![Value::Nil; 257]),
        Value::Map((0..65).map(|n| (n.to_string(), Value::Nil)).collect()),
        deep,
        Value::Float(f64::NAN),
        Value::Float(f64::INFINITY),
        Value::Float(f64::NEG_INFINITY),
        Value::Decimal {
            coefficient: 1,
            scale: 39,
        },
    ];
    negatives.push(Value::Sequence(vec![Value::Text("x".repeat(4096)); 32]));
    for value in negatives {
        let mut r = request.clone();
        r.arguments.get_mut("value").unwrap().value = value;
        assert!(executor.prepare(&repo, "owner", &binding, &r).is_err());
    }
    let mut wrong_type = request.clone();
    wrong_type
        .arguments
        .get_mut("value")
        .unwrap()
        .type_ref
        .revision = "missing".into();
    assert!(
        executor
            .prepare(&repo, "owner", &binding, &wrong_type)
            .is_err()
    );
    let mut wrong_contract = request.clone();
    wrong_contract.contract_ref.revision = "other".into();
    assert!(
        executor
            .prepare(&repo, "owner", &binding, &wrong_contract)
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(
        TypedRequest::decode(
            encoded
                .replace("\"Integer\":2", "\"Integer\":9223372036854775808")
                .as_bytes()
        )
        .is_err()
    );
    assert!(
        TypedRequest::decode(
            encoded
                .replace(
                    "{\"Integer\":2}",
                    "{\"Decimal\":{\"coefficient\":\"bad\",\"scale\":0}}"
                )
                .as_bytes()
        )
        .is_err()
    );
    assert!(TypedRequest::decode(&vec![b' '; 65537]).is_err());
    for value in [
        Value::Text("wrong".into()),
        Value::Text("x".repeat(4097)),
        Value::Float(f64::NAN),
        Value::Float(f64::INFINITY),
    ] {
        let c = calls.clone();
        let tr = request.arguments["value"].type_ref.clone();
        let cr = request.contract_ref.clone();
        let e = PureExecutor::new(target.clone(), contract.clone(), move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(TypedResult {
                contract_ref: cr.clone(),
                output: TypedValue {
                    type_ref: tr.clone(),
                    value: value.clone(),
                },
            })
        })
        .unwrap();
        let p = e.prepare(&repo, "owner", &binding, &request).unwrap();
        let before = calls.load(Ordering::SeqCst);
        assert!(e.invoke(&repo, &p, &request).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    }
    for fault in 0..3 {
        let c = calls.clone();
        let mut tr = request.arguments["value"].type_ref.clone();
        let mut cr = request.contract_ref.clone();
        if fault == 0 {
            tr.revision = "invalid".into();
        } else {
            cr.revision = "invalid".into();
        }
        let e = PureExecutor::new(target.clone(), contract.clone(), move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            if fault == 2 {
                return TypedResult::decode(br#"{"provider":"invalid","output":{"value":"2"}}"#);
            }
            Ok(TypedResult {
                contract_ref: cr.clone(),
                output: TypedValue {
                    type_ref: tr.clone(),
                    value: Value::Integer(2),
                },
            })
        })
        .unwrap();
        let p = e.prepare(&repo, "owner", &binding, &request).unwrap();
        let before = calls.load(Ordering::SeqCst);
        assert!(e.invoke(&repo, &p, &request).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    }
}

#[test]
fn supported_representations_and_each_bound_are_checked_without_coercion() {
    let (mut repo, binding, mut target, mut contract, mut request) = fixture();
    let mut head = bindings::read(&repo, "owner").unwrap().unwrap().0;
    let cases = [
        (ValueKind::Nil, Value::Nil),
        (ValueKind::Bool, Value::Bool(true)),
        (ValueKind::Integer, Value::Integer(i64::MAX)),
        (
            ValueKind::Decimal,
            Value::Decimal {
                coefficient: i128::MAX,
                scale: 38,
            },
        ),
        (ValueKind::Float, Value::Float(-0.0)),
        (ValueKind::Text, Value::Text("文字".into())),
        (
            ValueKind::Sequence,
            Value::Sequence(vec![Value::Integer(2)]),
        ),
        (
            ValueKind::Map,
            Value::Map(BTreeMap::from([("item".into(), Value::Bool(true))])),
        ),
        (
            ValueKind::TypeRef,
            Value::TypeRef(contract.output.reference.id),
        ),
        (
            ValueKind::TermRef,
            Value::TermRef(binding.binding.semantic_ref.term),
        ),
        (
            ValueKind::RoleRef,
            Value::RoleRef(
                sem_lang_core::RoleId::from_hex("62436820882964cfc19b6db3750b8d17").unwrap(),
            ),
        ),
    ];
    for (index, (kind, value)) in cases.into_iter().enumerate() {
        contract.inputs.get_mut("value").unwrap().kind = kind;
        contract.output.kind = kind;
        target.contract = contract.exact_ref().unwrap();
        let mut b = binding.binding.clone();
        b.target_ref.revision = Some(target.exact_ref().unwrap().revision);
        for (n, state) in [
            AdoptionState::Proposed,
            AdoptionState::Reviewed,
            AdoptionState::Adopted,
        ]
        .into_iter()
        .enumerate()
        {
            head = adoption::write(
                &mut repo,
                "owner",
                Some(&head),
                std::slice::from_ref(&b),
                state,
                &format!("decision:case:{index}:{n}"),
            )
            .unwrap();
        }
        let exact = bindings::resolve(&repo, "owner", &b.semantic_ref)
            .unwrap()
            .remove(0);
        request.contract_ref = contract.exact_ref().unwrap();
        request.arguments.get_mut("value").unwrap().value = value.clone();
        let e = PureExecutor::new(target.clone(), contract.clone(), |r: &TypedRequest| {
            Ok(TypedResult {
                contract_ref: r.contract_ref.clone(),
                output: r.arguments["value"].clone(),
            })
        })
        .unwrap();
        if exact != binding {
            assert!(
                e.prepare(&repo, "owner", &binding, &request).is_err(),
                "old adopted binding must not follow replacement"
            );
        }
        let p = e.prepare(&repo, "owner", &exact, &request).unwrap();
        assert_eq!(e.invoke(&repo, &p, &request).unwrap().output.value, value);
        if kind == ValueKind::Float {
            let mut changed = request.clone();
            changed.arguments.get_mut("value").unwrap().value = Value::Float(0.0);
            assert!(e.invoke(&repo, &p, &changed).is_err());
        }
    }
    let mut checks = vec![
        (
            Value::Text("x".repeat(MAX_TEXT_BYTES + 1)),
            ContractError::Bounds,
        ),
        (
            Value::Sequence(vec![Value::Nil; MAX_SEQUENCE + 1]),
            ContractError::Bounds,
        ),
        (
            Value::Map(
                (0..MAX_FIELDS + 1)
                    .map(|i| (i.to_string(), Value::Nil))
                    .collect(),
            ),
            ContractError::Bounds,
        ),
        (
            Value::Map(BTreeMap::from([(
                "x".repeat(MAX_KEY_BYTES + 1),
                Value::Nil,
            )])),
            ContractError::Bounds,
        ),
        (
            Value::Sequence(vec![Value::Text("x".repeat(MAX_TEXT_BYTES)); 32]),
            ContractError::Bounds,
        ),
        (
            Value::Sequence(vec![Value::Sequence(vec![Value::Nil; MAX_SEQUENCE]); 9]),
            ContractError::Bounds,
        ),
        (
            Value::Decimal {
                coefficient: 1,
                scale: -39,
            },
            ContractError::Value,
        ),
        (
            Value::TypedHole(sem_lang_core::TypedHole {
                id: "unresolved".into(),
                expected: None,
                surface: None,
            }),
            ContractError::Value,
        ),
    ];
    let mut deep = Value::Nil;
    for _ in 0..MAX_DEPTH + 1 {
        deep = Value::Sequence(vec![deep]);
    }
    checks.push((deep, ContractError::Bounds));
    for (value, error) in checks {
        let mut r = request.clone();
        r.arguments.get_mut("value").unwrap().value = value;
        assert_eq!(
            TypedRequest::decode(&serde_json::to_vec(&r).unwrap()).unwrap_err(),
            error
        );
    }
    let mut r = request.clone();
    r.arguments = (0..MAX_ARGUMENTS + 1)
        .map(|i| (i.to_string(), request.arguments["value"].clone()))
        .collect();
    assert_eq!(
        TypedRequest::decode(&serde_json::to_vec(&r).unwrap()).unwrap_err(),
        ContractError::Bounds
    );
    // Existing records with an absent revision remain readable, not executable.
    let mut optional = binding.binding.clone();
    optional.target_ref.revision = None;
    for (n, state) in [
        AdoptionState::Proposed,
        AdoptionState::Reviewed,
        AdoptionState::Adopted,
    ]
    .into_iter()
    .enumerate()
    {
        head = adoption::write(
            &mut repo,
            "owner",
            Some(&head),
            std::slice::from_ref(&optional),
            state,
            &format!("decision:optional:{n}"),
        )
        .unwrap();
    }
    let exact = bindings::resolve(&repo, "owner", &optional.semantic_ref)
        .unwrap()
        .remove(0);
    assert_eq!(
        resolve_execution_target(&repo, "owner", &exact, &target).unwrap_err(),
        ContractError::Target
    );
}
