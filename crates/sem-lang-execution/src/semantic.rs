//! Exact adopted semantic execution. No store, label resolver, Grant or adoption.
//! Callers hold their owner read fence throughout prepare/invoke. Handlers are
//! trusted bounded pure builtins or an explicitly supplied inference owner.
//! Inference results remain observations. Native/effect dispatch is excluded.
use sem_lang_core::{Digest, TypeId, Value};
use sem_lang_memory::{
    bindings::{self, BindingUsage, DefinitionRef, Direction, RevisionedBinding},
    definitions,
};
use sem_lang_repository::Repository;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, sync::Arc};

pub const MAX_BYTES: usize = 65_536;
pub const MAX_ARGUMENTS: usize = 32;
pub const MAX_FIELDS: usize = 64;
pub const MAX_SEQUENCE: usize = 256;
pub const MAX_DEPTH: usize = 16;
pub const MAX_NODES: usize = 2048;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_KEY_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ContractError {
    #[error("execution binding is not exact and currently adopted")]
    Binding,
    #[error("execution target or contract is not exact")]
    Target,
    #[error("execution envelope exceeds bounds")]
    Bounds,
    #[error("execution value is invalid or unresolved")]
    Value,
    #[error("execution value does not match exact type contract")]
    Type,
    #[error("semantic argument role does not match the exact execution contract")]
    Role,
    #[error("physical execution requires a different authorization boundary")]
    AuthorizationRequired,
    #[error("execution plan belongs to another executor or request")]
    StalePlan,
    #[error("pure handler failed")]
    Handler,
    #[error("execution wire is invalid")]
    Wire,
}
type Result<T> = std::result::Result<T, ContractError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactRef {
    pub owner: String,
    pub id: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactTypeRef {
    pub namespace: String,
    pub revision: String,
    pub id: TypeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactRoleRef {
    pub namespace: String,
    pub revision: String,
    pub id: sem_lang_core::RoleId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticArgument {
    pub role: ExactRoleRef,
    pub value: TypedValue,
}

/// Representation constraint on existing core::Value, not another value model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueKind {
    Nil,
    Bool,
    Integer,
    Decimal,
    Float,
    Text,
    Sequence,
    Map,
    TermRef,
    TypeRef,
    RoleRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeRequirement {
    pub reference: ExactTypeRef,
    pub kind: ValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOperationContract {
    pub owner: String,
    pub id: String,
    pub inputs: BTreeMap<String, TypeRequirement>,
    /// Explicit meaning-to-wire slots, included in this contract's exact digest.
    /// Empty is a representation-only contract, not a semantic input mapping.
    pub input_roles: BTreeMap<String, ExactRoleRef>,
    pub output: TypeRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetKind {
    PureBuiltin,
    PureModule,
    InferenceRuntime,
    Capability,
}

/// Computational class is separate from admission of its result. PureModule
/// describes an intention, not proof of native code purity or permission to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionClass {
    Pure,
    Inference,
    Effect,
}

/// Execution dispositions, not a second Claim/Proposal vocabulary. Observation
/// does not assert truth and grants no semantic admission authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResultDisposition {
    DeterministicValue,
    UnadoptedObservation,
    ExternalObservation,
}

impl ExecutionClass {
    pub fn result_disposition(self) -> ResultDisposition {
        match self {
            Self::Pure => ResultDisposition::DeterministicValue,
            Self::Inference => ResultDisposition::UnadoptedObservation,
            Self::Effect => ResultDisposition::ExternalObservation,
        }
    }
}

/// Immutable subordinate owner descriptor; no semantic binding body or process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTarget {
    pub owner: String,
    pub id: String,
    pub label: String,
    pub kind: TargetKind,
    pub operation: DefinitionRef,
    pub implementation: ExactRef,
    pub contract: ExactRef,
}

fn token(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 512 || !s.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ContractError::Target);
    }
    Ok(())
}
fn check_ref(r: &ExactRef) -> Result<()> {
    token(&r.owner)?;
    token(&r.id)?;
    token(&r.revision)
}
fn key(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > MAX_KEY_BYTES {
        Err(ContractError::Bounds)
    } else {
        Ok(())
    }
}

/// Serialize to a fixed bound, never allocate an oversized encoded envelope.
struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_BYTES - self.0.len() {
            return Err(std::io::Error::other("execution byte limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = BoundedBytes(Vec::new());
    serde_json::to_writer(&mut bytes, value).map_err(|_| ContractError::Bounds)?;
    Ok(bytes.0)
}
fn exact<T: Serialize>(owner: &str, id: &str, domain: &str, body: &T) -> Result<ExactRef> {
    token(owner)?;
    token(id)?;
    Ok(ExactRef {
        owner: owner.into(),
        id: id.into(),
        revision: format!(
            "{domain}:{}",
            Digest::object_digest(&encode(body)?).to_hex()
        ),
    })
}
impl SemanticOperationContract {
    pub fn exact_ref(&self) -> Result<ExactRef> {
        if self.inputs.len() > MAX_ARGUMENTS {
            return Err(ContractError::Bounds);
        }
        for k in self.inputs.keys() {
            key(k)?;
        }
        if !self.input_roles.is_empty() && self.input_roles.len() != self.inputs.len() {
            return Err(ContractError::Role);
        }
        let mut roles = Vec::new();
        for (argument, role) in &self.input_roles {
            if !self.inputs.contains_key(argument) || roles.contains(role) {
                return Err(ContractError::Role);
            }
            token(&role.namespace)?;
            token(&role.revision)?;
            roles.push(role.clone());
        }
        exact(&self.owner, &self.id, "contract/v1", self)
    }
}

/// Map exact semantic roles, never field names or positions, to C2 typed slots.
/// No binding adoption, operation dispatch or provider behavior is performed.
pub fn bind_semantic_arguments<R: Repository>(
    repo: &R,
    contract: &SemanticOperationContract,
    arguments: &[SemanticArgument],
) -> Result<TypedRequest> {
    let contract_ref = contract.exact_ref()?;
    if arguments.len() > MAX_ARGUMENTS {
        return Err(ContractError::Bounds);
    }
    if contract.input_roles.is_empty() || arguments.len() != contract.input_roles.len() {
        return Err(ContractError::Role);
    }
    encode(&arguments)?;
    let mut values = BTreeMap::new();
    for (name, role) in &contract.input_roles {
        let matches = arguments
            .iter()
            .filter(|a| a.role == *role)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(ContractError::Role);
        }
        let definitions = definitions::read_revision(repo, &role.namespace, &role.revision)
            .map_err(|_| ContractError::Role)?;
        if !definitions
            .modules
            .iter()
            .any(|m| m.roles.iter().any(|r| r.id == role.id))
        {
            return Err(ContractError::Role);
        }
        values.insert(name.clone(), matches[0].value.clone());
    }
    let request = TypedRequest {
        contract_ref,
        arguments: values,
    };
    request.validate(repo, contract)?;
    Ok(request)
}
impl ExecutionTarget {
    pub fn execution_class(&self) -> ExecutionClass {
        match self.kind {
            TargetKind::PureBuiltin | TargetKind::PureModule => ExecutionClass::Pure,
            TargetKind::InferenceRuntime => ExecutionClass::Inference,
            TargetKind::Capability => ExecutionClass::Effect,
        }
    }
    pub fn exact_ref(&self) -> Result<ExactRef> {
        check_ref(&self.implementation)?;
        check_ref(&self.contract)?;
        if self.label.len() > MAX_TEXT_BYTES {
            return Err(ContractError::Bounds);
        }
        token(&self.operation.namespace)?;
        token(&self.operation.revision)?;
        exact(&self.owner, &self.id, "target/v1", self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedValue {
    pub type_ref: ExactTypeRef,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedRequest {
    pub contract_ref: ExactRef,
    pub arguments: BTreeMap<String, TypedValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedResult {
    pub contract_ref: ExactRef,
    pub output: TypedValue,
}

fn inspect(value: &Value, depth: usize, nodes: &mut usize) -> Result<ValueKind> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err(ContractError::Bounds);
    }
    Ok(match value {
        Value::Nil => ValueKind::Nil,
        Value::Bool(_) => ValueKind::Bool,
        Value::Integer(_) => ValueKind::Integer,
        Value::Decimal { scale, .. } => {
            if !(-38..=38).contains(scale) {
                return Err(ContractError::Value);
            }
            ValueKind::Decimal
        }
        Value::Float(f) => {
            if !f.is_finite() {
                return Err(ContractError::Value);
            }
            ValueKind::Float
        }
        Value::Text(s) => {
            if s.len() > MAX_TEXT_BYTES {
                return Err(ContractError::Bounds);
            }
            ValueKind::Text
        }
        Value::Sequence(seq) => {
            if seq.len() > MAX_SEQUENCE {
                return Err(ContractError::Bounds);
            }
            for v in seq {
                inspect(v, depth + 1, nodes)?;
            }
            ValueKind::Sequence
        }
        Value::Map(map) => {
            if map.len() > MAX_FIELDS {
                return Err(ContractError::Bounds);
            }
            for (k, v) in map {
                key(k)?;
                inspect(v, depth + 1, nodes)?;
            }
            ValueKind::Map
        }
        Value::TermRef(_) => ValueKind::TermRef,
        Value::TypeRef(_) => ValueKind::TypeRef,
        Value::RoleRef(_) => ValueKind::RoleRef,
        _ => return Err(ContractError::Value),
    })
}
fn check_type<R: Repository>(repo: &R, typed: &TypedValue, spec: &TypeRequirement) -> Result<()> {
    if typed.type_ref != spec.reference || inspect(&typed.value, 0, &mut 0)? != spec.kind {
        return Err(ContractError::Type);
    }
    let r = &spec.reference;
    token(&r.namespace)?;
    token(&r.revision)?;
    let defs = definitions::read_revision(repo, &r.namespace, &r.revision)
        .map_err(|_| ContractError::Type)?;
    if !defs
        .modules
        .iter()
        .any(|m| m.types.iter().any(|t| t.id == r.id))
    {
        return Err(ContractError::Type);
    }
    // Ref Values are scoped to this exact definition revision, including nested refs.
    let mut pending = vec![&typed.value];
    while let Some(v) = pending.pop() {
        let valid = match v {
            Value::TermRef(id) => defs
                .modules
                .iter()
                .any(|m| m.terms.iter().any(|t| &t.id == id)),
            Value::TypeRef(id) => defs
                .modules
                .iter()
                .any(|m| m.types.iter().any(|t| &t.id == id)),
            Value::RoleRef(id) => defs
                .modules
                .iter()
                .any(|m| m.roles.iter().any(|t| &t.id == id)),
            Value::Sequence(v) => {
                pending.extend(v);
                true
            }
            Value::Map(v) => {
                pending.extend(v.values());
                true
            }
            _ => true,
        };
        if !valid {
            return Err(ContractError::Type);
        }
    }
    Ok(())
}
impl TypedRequest {
    /// Bounded wire intake. Exact owner/contract checks still happen at prepare.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(ContractError::Bounds);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ContractError::Wire)?;
        value.bounds()?;
        Ok(value)
    }
    fn bounds(&self) -> Result<()> {
        check_ref(&self.contract_ref)?;
        if self.arguments.len() > MAX_ARGUMENTS {
            return Err(ContractError::Bounds);
        }
        let mut nodes = 0;
        for (k, v) in &self.arguments {
            key(k)?;
            inspect(&v.value, 0, &mut nodes)?;
        }
        encode(self)?;
        Ok(())
    }

    /// Canonical C2 contract/type validation, shared by pure and inference paths.
    pub fn validate<R: Repository>(
        &self,
        repo: &R,
        contract: &SemanticOperationContract,
    ) -> Result<()> {
        if self.contract_ref != contract.exact_ref()? {
            return Err(ContractError::Target);
        }
        self.bounds()?;
        if self.arguments.len() != contract.inputs.len() {
            return Err(ContractError::Type);
        }
        for (k, spec) in &contract.inputs {
            check_type(
                repo,
                self.arguments.get(k).ok_or(ContractError::Type)?,
                spec,
            )?;
        }
        let r = &contract.output.reference;
        let defs = definitions::read_revision(repo, &r.namespace, &r.revision)
            .map_err(|_| ContractError::Type)?;
        if !defs
            .modules
            .iter()
            .any(|m| m.types.iter().any(|t| t.id == r.id))
        {
            return Err(ContractError::Type);
        }
        Ok(())
    }
}
impl TypedResult {
    /// Bounded JSON result decoder. Physical invocation belongs to the
    /// selected provider and is not part of this semantic contract.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(ContractError::Bounds);
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|_| ContractError::Wire)?;
        inspect(&result.output.value, 0, &mut 0)?;
        encode(&result)?;
        Ok(result)
    }

    pub fn validate<R: Repository>(
        &self,
        repo: &R,
        contract: &SemanticOperationContract,
    ) -> Result<()> {
        inspect(&self.output.value, 0, &mut 0)?;
        encode(self)?;
        if self.contract_ref != contract.exact_ref()? {
            return Err(ContractError::Target);
        }
        check_type(repo, &self.output, &contract.output)
    }
}

/// No resolver callback can manufacture adoption. Read the canonical owner now.
pub fn resolve_execution_target<R: Repository>(
    repo: &R,
    namespace: &str,
    binding: &RevisionedBinding,
    target: &ExecutionTarget,
) -> Result<ExactRef> {
    if !bindings::resolve(repo, namespace, &binding.binding.semantic_ref)
        .map_err(|_| ContractError::Binding)?
        .contains(binding)
    {
        return Err(ContractError::Binding);
    }
    let b = &binding.binding;
    if b.usage != BindingUsage::Action
        || b.direction == Direction::Output
        || !b.constraints.is_empty()
    {
        return Err(ContractError::Binding);
    }
    let exact = target.exact_ref()?;
    if b.target_ref.namespace != exact.owner
        || b.target_ref.capability != exact.id
        || b.target_ref.revision.as_ref() != Some(&exact.revision)
        || b.semantic_ref != target.operation
    {
        return Err(ContractError::Target);
    }
    Ok(exact)
}

pub struct InvocationPlan {
    allocation: Arc<()>,
    namespace: String,
    binding: RevisionedBinding,
    request: TypedRequest,
    target: ExactRef,
    contract: ExactRef,
}
pub struct PureExecutor<F> {
    allocation: Arc<()>,
    target: ExecutionTarget,
    contract: SemanticOperationContract,
    handler: F,
}
impl<F: Fn(&TypedRequest) -> Result<TypedResult>> PureExecutor<F> {
    /// Caller admits the exact builtin implementation. A descriptor is not code
    /// provenance or authorization. No module/endpoint/path resolution occurs.
    pub fn new(
        target: ExecutionTarget,
        contract: SemanticOperationContract,
        handler: F,
    ) -> Result<Self> {
        target.exact_ref()?;
        if target.kind != TargetKind::PureBuiltin {
            return Err(ContractError::AuthorizationRequired);
        }
        if target.contract != contract.exact_ref()? {
            return Err(ContractError::Target);
        }
        Ok(Self {
            allocation: Arc::new(()),
            target,
            contract,
            handler,
        })
    }
    pub fn prepare<R: Repository>(
        &self,
        repo: &R,
        namespace: &str,
        binding: &RevisionedBinding,
        request: &TypedRequest,
    ) -> Result<InvocationPlan> {
        let target = resolve_execution_target(repo, namespace, binding, &self.target)?;
        let contract = self.contract.exact_ref()?;
        request.validate(repo, &self.contract)?;
        Ok(InvocationPlan {
            allocation: self.allocation.clone(),
            namespace: namespace.into(),
            binding: binding.clone(),
            request: request.clone(),
            target,
            contract,
        })
    }
    pub fn invoke<R: Repository>(
        &self,
        repo: &R,
        plan: &InvocationPlan,
        request: &TypedRequest,
    ) -> Result<TypedResult> {
        if !Arc::ptr_eq(&self.allocation, &plan.allocation)
            || &plan.request != request
            || encode(&plan.request)? != encode(request)?
        {
            return Err(ContractError::StalePlan);
        }
        let current = self.prepare(repo, &plan.namespace, &plan.binding, request)?;
        if current.target != plan.target || current.contract != plan.contract {
            return Err(ContractError::StalePlan);
        }
        let result = (self.handler)(request)?;
        result.validate(repo, &self.contract)?;
        Ok(result)
    }
}

/// Opaque, allocation-bound preparation. Not a serialized authorization token.
/// The caller holds the existing repository read fence through owner dispatch
/// and result validation. This type never creates a memory/store authority.
pub struct InferencePlan {
    allocation: Arc<()>,
    namespace: String,
    binding: RevisionedBinding,
    request: TypedRequest,
    target: ExactRef,
}
impl InferencePlan {
    pub fn target_ref(&self) -> &ExactRef {
        &self.target
    }
    pub fn binding_ref(&self) -> ExactRef {
        ExactRef {
            owner: self.namespace.clone(),
            id: self.binding.binding.binding_id.clone(),
            revision: self.binding.revision.clone(),
        }
    }
}

pub struct InferenceExecutor {
    allocation: Arc<()>,
    target: ExecutionTarget,
    contract: SemanticOperationContract,
}
impl InferenceExecutor {
    pub fn new(target: ExecutionTarget, contract: SemanticOperationContract) -> Result<Self> {
        target.exact_ref()?;
        if target.kind != TargetKind::InferenceRuntime {
            return Err(ContractError::AuthorizationRequired);
        }
        if target.contract != contract.exact_ref()? {
            return Err(ContractError::Target);
        }
        Ok(Self {
            allocation: Arc::new(()),
            target,
            contract,
        })
    }
    pub fn target(&self) -> &ExecutionTarget {
        &self.target
    }
    pub fn contract(&self) -> &SemanticOperationContract {
        &self.contract
    }
    pub fn prepare<R: Repository>(
        &self,
        repo: &R,
        namespace: &str,
        binding: &RevisionedBinding,
        request: &TypedRequest,
    ) -> Result<InferencePlan> {
        let target = resolve_execution_target(repo, namespace, binding, &self.target)?;
        request.validate(repo, &self.contract)?;
        Ok(InferencePlan {
            allocation: self.allocation.clone(),
            namespace: namespace.into(),
            binding: binding.clone(),
            request: request.clone(),
            target,
        })
    }
    /// Must run immediately before handing a request to the physical owner.
    pub fn validate<R: Repository>(
        &self,
        repo: &R,
        plan: &InferencePlan,
        request: &TypedRequest,
    ) -> Result<()> {
        if !Arc::ptr_eq(&self.allocation, &plan.allocation)
            || encode(request)? != encode(&plan.request)?
        {
            return Err(ContractError::StalePlan);
        }
        let current = self.prepare(repo, &plan.namespace, &plan.binding, request)?;
        if current.target != plan.target {
            return Err(ContractError::StalePlan);
        }
        Ok(())
    }
    /// Valid execution output is not semantic truth. A caller must use the
    /// existing explicit observation/compilation/admission route afterwards.
    pub fn observe<R: Repository>(
        &self,
        repo: &R,
        plan: &InferencePlan,
        request: &TypedRequest,
        result: &TypedResult,
    ) -> Result<ResultDisposition> {
        self.validate(repo, plan, request)?;
        result.validate(repo, &self.contract)?;
        Ok(ResultDisposition::UnadoptedObservation)
    }
}
