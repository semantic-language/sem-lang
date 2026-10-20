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
use std::collections::BTreeMap;

pub fn fixture(
    kind: TargetKind,
    runtime: &str,
) -> (
    InMemoryRepository,
    RevisionedBinding,
    ExecutionTarget,
    SemanticOperationContract,
    TypedRequest,
) {
    let mut repo = InMemoryRepository::new();
    let (binding, target, contract, request) = populate(&mut repo, kind, runtime);
    (repo, binding, target, contract, request)
}

pub fn populate<R: sem_lang_repository::Repository + sem_lang_repository::Backend>(
    repo: &mut R,
    kind: TargetKind,
    runtime: &str,
) -> (
    RevisionedBinding,
    ExecutionTarget,
    SemanticOperationContract,
    TypedRequest,
) {
    let module: VocabularyModule = serde_json::from_value(serde_json::json!({
        "schema_version":"0.10.0","vocabulary_id":"specimen","version":"0.10.0","dependencies":[],
        "terms":[{"id":"42436820882964cfc19b6db3750b8d17","label":"Identity","description":"Test only","deprecation":null,"replacement":null,"semantic_definition":{"Primitive":"Relation"}}],
        "types":[{"id":"52436820882964cfc19b6db3750b8d17","label":"Specimen scalar","description":"Test only","super_types":[],"deprecation":null}],"roles":[{"id":"62436820882964cfc19b6db3750b8d17","label":"item","description":"Test only","cardinality":"ExactlyOne"}],"lexical_bindings":[]
    })).expect("test fixture");
    let term = module.terms[0].id;
    let revision = adoption::adopt(repo, "owner", None, &[module], "decision:definition")
        .expect("test fixture");
    let ty = ExactTypeRef {
        namespace: "owner".into(),
        revision: revision.clone(),
        id: TypeId::from_hex("52436820882964cfc19b6db3750b8d17").expect("test fixture"),
    };
    let spec = TypeRequirement {
        reference: ty.clone(),
        kind: ValueKind::Text,
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
            owner: "zixcel-local-inference".into(),
            id: "runtime".into(),
            revision: runtime.into(),
        },
        contract: contract.exact_ref().expect("test fixture"),
    };
    let b = SemanticBinding {
        binding_id: "specimen".into(),
        semantic_ref: target.operation.clone(),
        target_ref: CapabilityRef {
            namespace: target.owner.clone(),
            capability: target.id.clone(),
            revision: Some(target.exact_ref().expect("test fixture").revision),
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
                repo,
                "owner",
                head.as_ref(),
                std::slice::from_ref(&b),
                state,
                decision,
            )
            .expect("test fixture"),
        );
    }
    let binding = bindings::resolve(repo, "owner", &b.semantic_ref)
        .expect("test fixture")
        .remove(0);
    let request = TypedRequest {
        contract_ref: contract.exact_ref().expect("test fixture"),
        arguments: BTreeMap::from([(
            "value".into(),
            TypedValue {
                value: Value::Text("Return one short word.".into()),
                type_ref: ty,
            },
        )]),
    };
    (binding, target, contract, request)
}
