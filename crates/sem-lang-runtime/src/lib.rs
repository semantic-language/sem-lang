//! Deterministic runtime for semantic expressions and packets.

pub mod entity_index;

use sem_lang_core::{
    ApplicationOutcome, ApplicationReceipt, Digest, GraphNode, Resolution, StableId,
};
use sem_lang_core::{
    CanonicalExpression, CausalDag, Constraint, EffectIntent, EvaluationResult, Expr, GraphDelta,
    NodeId, RuntimeState, SemanticGraph, SemanticPacket, TypedHole, Value,
};
use sem_lang_repository::commit_image::SnapshotBackend;
use sem_lang_vocab::VocabularyRegistry;
use serde::{Deserialize, Serialize};
use zixcel_revision::{
    CommitIntent, CommitOutcome, CommitStore, Conflict, Failure, OperationId, Rejection,
    RevisionRef,
};
const PACKET_DOMAIN: &str = "sem-lang/runtime/packet";

/// Publication fencing is supplied by the caller, never guessed from semantic data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PacketRequest {
    pub operation_id: OperationId,
    pub expected_commit_revision: RevisionRef,
    pub expected_semantic_digest: Digest,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PacketResult {
    request_digest: Digest,
    input_digest: Digest,
    output_digest: Digest,
    outcome: ApplicationOutcome,
}
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub message: String,
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Debug, Clone, Default)]
pub struct TypeChecker {
    type_hierarchy: BTreeMap<sem_lang_core::TypeId, BTreeSet<sem_lang_core::TypeId>>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            type_hierarchy: BTreeMap::new(),
        }
    }

    pub fn register_subtype(
        &mut self,
        child: sem_lang_core::TypeId,
        parent: sem_lang_core::TypeId,
    ) {
        self.type_hierarchy.entry(child).or_default().insert(parent);
    }

    pub fn is_subtype(&self, child: sem_lang_core::TypeId, parent: sem_lang_core::TypeId) -> bool {
        if child == parent {
            return true;
        }
        let mut stack = vec![child];
        let mut seen = BTreeSet::new();
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            if let Some(parents) = self.type_hierarchy.get(&current) {
                if parents.contains(&parent) {
                    return true;
                }
                stack.extend(parents.iter().copied());
            }
        }
        false
    }

    pub fn check_expr(&self, expr: &Expr, vocab: &VocabularyRegistry) -> bool {
        match expr {
            Expr::Value(value) => self.check_value(value, vocab),
            Expr::Apply(apply) => {
                self.has_term(apply.operator, vocab)
                    && apply.arguments.iter().all(|argument| {
                        argument
                            .role
                            .map(|role| self.has_role(role, vocab))
                            .unwrap_or(true)
                            && self.check_value(&argument.value, vocab)
                    })
            }
            Expr::Binding(binding) => {
                binding
                    .variable
                    .expected
                    .map(|ty| self.has_type(ty, vocab))
                    .unwrap_or(true)
                    && self.check_value(&binding.value, vocab)
            }
            Expr::Reference(reference) => self.check_value(&reference.target, vocab),
            Expr::TypedHole(hole) => hole
                .expected
                .map(|ty| self.has_type(ty, vocab))
                .unwrap_or(true),
        }
    }

    fn check_value(&self, value: &Value, vocab: &VocabularyRegistry) -> bool {
        match value {
            Value::TermRef(term) => self.has_term(*term, vocab),
            Value::TypeRef(ty) => self.has_type(*ty, vocab),
            Value::RoleRef(role) => self.has_role(*role, vocab),
            Value::Sequence(values) => values.iter().all(|value| self.check_value(value, vocab)),
            Value::Map(values) => values.values().all(|value| self.check_value(value, vocab)),
            Value::TypedHole(hole) => hole
                .expected
                .map(|ty| self.has_type(ty, vocab))
                .unwrap_or(true),
            _ => true,
        }
    }

    fn has_term(&self, term: sem_lang_core::TermId, vocab: &VocabularyRegistry) -> bool {
        vocab
            .modules()
            .any(|module| module.terms.iter().any(|definition| definition.id == term))
    }

    fn has_type(&self, ty: sem_lang_core::TypeId, vocab: &VocabularyRegistry) -> bool {
        vocab
            .modules()
            .any(|module| module.types.iter().any(|definition| definition.id == ty))
    }

    fn has_role(&self, role: sem_lang_core::RoleId, vocab: &VocabularyRegistry) -> bool {
        vocab
            .modules()
            .any(|module| module.roles.iter().any(|definition| definition.id == role))
    }
}

#[derive(Debug, Clone)]
pub struct ConstraintEvaluator;

impl ConstraintEvaluator {
    pub fn evaluate(constraints: &[Constraint], state: &RuntimeState) -> bool {
        constraints.iter().all(|constraint| match constraint {
            Constraint::TypeConstraint { value, ty } => value_matches_type(value, *ty, state),
            Constraint::RelationConstraint {
                value,
                role,
                target,
            } => state.graph.nodes.values().any(|node| match &node.expr {
                Expr::Apply(apply) => {
                    matches!(value, Value::TermRef(term) if *term == apply.operator)
                        && apply.arguments.iter().any(|argument| {
                            argument.role == Some(*role) && values_equal(&argument.value, target)
                        })
                }
                _ => false,
            }),
            Constraint::ScopeConstraint { variable, type_id } => {
                state.types.contains_key(type_id)
                    && variable
                        .expected
                        .map(|expected| expected == *type_id)
                        .unwrap_or(true)
            }
        })
    }
}

fn value_matches_type(value: &Value, ty: sem_lang_core::TypeId, state: &RuntimeState) -> bool {
    match value {
        Value::TermRef(term) => state
            .terms
            .get(term)
            .map(|definition| definition.type_id == ty)
            .unwrap_or(false),
        Value::TypeRef(value_type) => *value_type == ty,
        Value::TypedHole(hole) => hole.expected == Some(ty),
        _ => false,
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    format_value_as_key(left) == format_value_as_key(right)
}

#[derive(Debug, Clone)]
pub struct ExpressionNormalizer;

impl ExpressionNormalizer {
    pub fn normalize(expr: &Expr) -> CanonicalExpression {
        let expr = expr.clone();
        let normalized = match expr {
            Expr::Apply(mut apply) => {
                apply.arguments.sort_by(|a, b| {
                    let sa = format_value_as_key(&a.value);
                    let sb = format_value_as_key(&b.value);
                    sa.cmp(&sb)
                });
                Expr::Apply(apply)
            }
            other => other,
        };
        CanonicalExpression { expr: normalized }
    }
}

#[inline]
fn format_value_as_key(value: &Value) -> String {
    match value {
        Value::Text(v) => format!("T:{v}"),
        Value::Integer(v) => format!("I:{v}"),
        Value::Bool(v) => format!("B:{v}"),
        Value::TermRef(t) => format!("Term:{t}"),
        Value::TypeRef(t) => format!("Type:{t}"),
        Value::RoleRef(t) => format!("Role:{t}"),
        Value::Entity(e) => format!("Entity:{e}"),
        Value::Node(n) => format!("Node:{n}"),
        Value::Expr(_) => "Expr".to_string(),
        Value::Float(v) => format!("F:{v}"),
        Value::Decimal { coefficient, scale } => format!("D:{coefficient}:{scale}"),
        Value::Nil => "Nil".to_string(),
        Value::TypedHole(h) => format!("Hole:{}", h.id),
        Value::Sequence(v) => v
            .iter()
            .map(format_value_as_key)
            .collect::<Vec<_>>()
            .join(","),
        Value::Map(m) => {
            let mut pairs: Vec<_> = m
                .iter()
                .map(|(k, v)| format!("{k}:{}", format_value_as_key(v)))
                .collect();
            pairs.sort();
            pairs.join(";")
        }
    }
}

#[derive(Debug, Clone)]
pub struct PureEvaluator;

impl PureEvaluator {
    pub fn evaluate(expr: &Expr, state: &RuntimeState) -> Option<Value> {
        if let Some((value, unresolved)) = Self::evaluate_no_state(expr, state)
            && !unresolved
        {
            return Some(value);
        }
        None
    }

    fn evaluate_no_state(expr: &Expr, _state: &RuntimeState) -> Option<(Value, bool)> {
        match expr {
            Expr::Value(v) => Some((v.clone(), value_contains_hole(v))),
            Expr::TypedHole(TypedHole {
                id,
                expected: _,
                surface,
            }) => Some((
                Value::Text(format!(
                    "<hole:{}:{}>",
                    id,
                    surface.clone().unwrap_or_default()
                )),
                true,
            )),
            Expr::Binding(binding) => {
                Some((binding.value.clone(), value_contains_hole(&binding.value)))
            }
            // An expression is not an executable binding. Until exact adopted
            // dispatch exists, retain unresolvedness instead of fabricating a
            // successful result by merely repacking its arguments.
            Expr::Apply(_) => None,
            Expr::Reference(reference) => {
                let value = reference_to_value(reference);
                Some((value.clone(), value_contains_hole(&value)))
            }
        }
    }
}

fn value_contains_hole(value: &Value) -> bool {
    match value {
        Value::TypedHole(_) => true,
        Value::Sequence(values) => values.iter().any(value_contains_hole),
        Value::Map(values) => values.values().any(value_contains_hole),
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct ReferenceResolver;

impl ReferenceResolver {
    pub fn resolve_value(value: &Value, state: &RuntimeState) -> Option<Value> {
        match value {
            Value::TermRef(t) => state.terms.get(t).map(|_| value.clone()),
            Value::Entity(_) => Some(value.clone()),
            _ => Some(value.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeStateTransition {
    state: RuntimeState,
    receipt: ApplicationReceipt,
}

#[derive(Debug, serde::Serialize, thiserror::Error)]
pub enum PacketApplyError {
    #[error("packet is not resolved: {0:?}")]
    Unresolved(Resolution),
    #[error("semantic input digest is stale")]
    StaleSemanticInput,
    #[error("commit conflict: {0:?}")]
    CommitConflict(#[serde(with = "sem_lang_repository::commit_wire::Conflict")] Conflict),
    #[error(transparent)]
    CommitFailure(
        #[from]
        #[serde(with = "sem_lang_repository::commit_wire::Failure")]
        Failure,
    ),
    #[error(transparent)]
    CommitRejected(
        #[from]
        #[serde(with = "sem_lang_repository::commit_wire::Rejection")]
        Rejection,
    ),
    #[error("packet identity was reused with different data or expected revision")]
    IdentityConflict,
    #[error("claim removal does not match current state")]
    ClaimConflict,
    #[error("packet exceeds 1 MiB")]
    PacketTooLarge,
    #[error("invalid typed representation: {0}")]
    Representation(
        #[from]
        #[serde(serialize_with = "sem_lang_repository::commit_wire::display_error")]
        serde_json::Error,
    ),
}

#[derive(Debug, Clone)]
pub struct PacketReducer;

impl PacketReducer {
    pub fn commit_revision(state: &RuntimeState) -> Result<RevisionRef, PacketApplyError> {
        let backend = SnapshotBackend::new(state.commit_state.clone())?;
        let commits = CommitStore::new(&backend);
        match commits.current(PACKET_DOMAIN)? {
            None => Ok(RevisionRef::default()),
            Some(current) => Ok(commits
                .receipt(PACKET_DOMAIN, current.operation_id())?
                .ok_or(Failure::Corrupt)?
                .committed_revision),
        }
    }
    pub fn reduce(
        previous_state: &RuntimeState,
        packet: &SemanticPacket,
        request: &PacketRequest,
    ) -> Result<RuntimeStateTransition, PacketApplyError> {
        let backend = SnapshotBackend::new(previous_state.commit_state.clone())?;
        let commits = CommitStore::new(&backend);
        let request_digest = Digest::from_serializable(1, &(packet, request))?;
        if let Some(receipt) = commits.receipt(PACKET_DOMAIN, &request.operation_id)? {
            let original = commits
                .committed(&receipt.commit_ref)?
                .ok_or(Failure::Corrupt)?;
            let mut result: PacketResult =
                serde_json::from_slice(original.payload()).map_err(|_| Failure::Corrupt)?;
            result.request_digest = request_digest;
            let replay = CommitIntent {
                domain: PACKET_DOMAIN.into(),
                operation_id: request.operation_id.clone(),
                expected_revision: request.expected_commit_revision.clone(),
                parents: request
                    .expected_commit_revision
                    .commit
                    .iter()
                    .cloned()
                    .collect(),
                payload: serde_json::to_vec(&result)?,
            }
            .prepare()?;
            return match commits.commit(&replay) {
                CommitOutcome::NoChange(commit) => Ok(RuntimeStateTransition {
                    state: previous_state.clone(),
                    receipt: ApplicationReceipt {
                        packet_id: packet.id,
                        commit,
                        input_digest: result.input_digest,
                        output_digest: result.output_digest,
                        outcome: result.outcome,
                    },
                }),
                CommitOutcome::Conflict(conflict) => {
                    Err(PacketApplyError::CommitConflict(conflict))
                }
                CommitOutcome::Rejected(error) => Err(error.into()),
                CommitOutcome::Failure(error) => Err(error.into()),
                CommitOutcome::Committed(_) => Err(Failure::Corrupt.into()),
            };
        }
        if packet.resolution != Resolution::Resolved {
            return Err(PacketApplyError::Unresolved(packet.resolution));
        }
        if serde_json::to_vec(packet)?.len() > 1_048_576 {
            return Err(PacketApplyError::PacketTooLarge);
        }
        if previous_state.semantic_state_digest()? != request.expected_semantic_digest {
            return Err(PacketApplyError::StaleSemanticInput);
        }
        // Semantic source identity is independent of the canonical operation ID.
        if previous_state
            .packets
            .get(&packet.id)
            .is_some_and(|original| original != packet)
        {
            return Err(PacketApplyError::IdentityConflict);
        }
        let mut after = previous_state.clone();
        // Transport/observation IDs are not semantic identity. Scope, act and
        // adopted meaning references are, so equal text from different subjects
        // must not accidentally share a node.
        let identity = serde_json::to_vec(&(
            &packet.sender,
            &packet.recipient,
            &packet.act,
            &packet.vocabulary_manifest,
            &packet.content,
        ))?;
        let digest = Digest::proposition_digest(&identity);
        let mut id = [0; 16];
        id.copy_from_slice(&digest.bytes[..16]);
        let node_id = NodeId(StableId(id));
        if let Some(existing) = after.graph.nodes.get(&node_id) {
            if existing.expr != packet.content {
                return Err(PacketApplyError::IdentityConflict);
            }
        } else {
            after
                .graph
                .add_node(GraphNode {
                    id: node_id,
                    expr: packet.content.clone(),
                    created_by: Some(packet.id),
                })
                .map_err(|_| PacketApplyError::IdentityConflict)?;
        }
        if let Some(delta) = &packet.context_delta {
            for claim in &delta.removed_claims {
                if after.claims.get(&claim.id) != Some(claim) {
                    return Err(PacketApplyError::ClaimConflict);
                }
                after.claims.remove(&claim.id);
            }
            for claim in &delta.added_claims {
                if after
                    .claims
                    .get(&claim.id)
                    .is_some_and(|current| current != claim)
                {
                    return Err(PacketApplyError::ClaimConflict);
                }
                after.claims.insert(claim.id, claim.clone());
            }
            after.annotations.extend(delta.annotations.clone());
        }
        let before_digest = previous_state.semantic_state_digest()?;
        let after_digest = after.semantic_state_digest()?;
        let changed = before_digest != after_digest;
        let result = PacketResult {
            request_digest,
            input_digest: before_digest,
            output_digest: after_digest,
            outcome: if changed {
                ApplicationOutcome::Applied
            } else {
                ApplicationOutcome::NoChange
            },
        };
        let prepared = CommitIntent {
            domain: PACKET_DOMAIN.into(),
            operation_id: request.operation_id.clone(),
            expected_revision: request.expected_commit_revision.clone(),
            parents: request
                .expected_commit_revision
                .commit
                .iter()
                .cloned()
                .collect(),
            payload: serde_json::to_vec(&result)?,
        }
        .prepare()?;
        let commit = match commits.commit(&prepared) {
            CommitOutcome::Committed(commit) => commit,
            CommitOutcome::Conflict(conflict) => {
                return Err(PacketApplyError::CommitConflict(conflict));
            }
            CommitOutcome::Rejected(error) => return Err(error.into()),
            CommitOutcome::Failure(error) => return Err(error.into()),
            CommitOutcome::NoChange(_) => return Err(Failure::Corrupt.into()),
        };
        let receipt = ApplicationReceipt {
            packet_id: packet.id,
            commit,
            input_digest: result.input_digest,
            output_digest: result.output_digest,
            outcome: result.outcome,
        };
        after.remember_packet(packet.clone());
        after.commit_state = backend.into_state()?;
        Ok(RuntimeStateTransition {
            state: after,
            receipt,
        })
    }
}

impl RuntimeStateTransition {
    pub fn receipt(&self) -> &ApplicationReceipt {
        &self.receipt
    }

    pub fn into_state(self) -> RuntimeState {
        self.state
    }

    pub fn effect_intents(&self) -> &[EffectIntent] {
        &[]
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeEngine {
    pub type_checker: TypeChecker,
    pub state: RuntimeState,
    pub vocab: VocabularyRegistry,
}

impl RuntimeEngine {
    pub fn new(vocab: VocabularyRegistry) -> Self {
        Self {
            type_checker: TypeChecker::new(),
            state: RuntimeState::new(),
            vocab,
        }
    }

    pub fn with_state(vocab: VocabularyRegistry, state: RuntimeState) -> Self {
        Self {
            type_checker: TypeChecker::new(),
            state,
            vocab,
        }
    }

    pub fn evaluate(&self, expression: &Expr, _context: &CausalDag) -> EvaluationResult {
        let value = self
            .type_checker
            .check_expr(expression, &self.vocab)
            .then(|| PureEvaluator::evaluate(expression, &self.state))
            .flatten();
        EvaluationResult {
            value,
            intents: vec![EffectIntent::Noop],
            graph_delta: vec![],
        }
    }

    pub fn normalize(&self, expression: &Expr) -> CanonicalExpression {
        ExpressionNormalizer::normalize(expression)
    }

    pub fn apply_delta(&self, graph: &SemanticGraph, deltas: &[GraphDelta]) -> SemanticGraph {
        let mut out = graph.clone();
        for d in deltas {
            match d {
                GraphDelta::AddNode(n) => {
                    let _ = out.add_node(n.clone());
                }
                GraphDelta::ReplaceNode(n) => out.replace_node(n.clone()),
                GraphDelta::RemoveNode(i) => {
                    let _ = out.remove_node(i);
                }
                GraphDelta::AddEdge(edge) => out.add_edge(edge.clone()),
                GraphDelta::RemoveEdgeByIndex(i) => {
                    if *i < out.edges.len() {
                        let _ = out.edges.remove(*i);
                    }
                }
            }
        }
        out
    }

    pub fn reduce(
        &self,
        previous_state: &RuntimeState,
        packet: &SemanticPacket,
        request: &PacketRequest,
    ) -> Result<RuntimeStateTransition, PacketApplyError> {
        PacketReducer::reduce(previous_state, packet, request)
    }

    pub fn unify(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::TypedHole(_), _) | (_, Value::TypedHole(_)) => true,
            (Value::Text(a), Value::Text(b)) => a == b,
            (Value::Integer(a), Value::Integer(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::TermRef(a), Value::TermRef(b)) => a == b,
            (Value::TypeRef(a), Value::TypeRef(b)) => a == b,
            _ => false,
        }
    }

    pub fn packet_digests(&self) -> Result<(Digest, Digest, Digest), serde_json::Error> {
        Ok((
            self.state.proposition_digest()?,
            self.state.interaction_digest()?,
            self.state.semantic_state_digest()?,
        ))
    }

    pub fn check_constraint(&self, constraints: &[Constraint]) -> bool {
        ConstraintEvaluator::evaluate(constraints, &self.state)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReferenceGraph {
    adjacency: BTreeMap<usize, BTreeSet<usize>>,
    labels: BTreeMap<usize, String>,
}

impl ReferenceGraph {
    pub fn new() -> Self {
        Self {
            adjacency: BTreeMap::new(),
            labels: BTreeMap::new(),
        }
    }

    pub fn add_node<S: Into<String>>(&mut self, label: S) -> usize {
        let id = self.labels.len();
        self.adjacency.insert(id, BTreeSet::new());
        self.labels.insert(id, label.into());
        id
    }

    pub fn add_edge(&mut self, from: usize, to: usize) {
        self.adjacency.entry(from).or_default().insert(to);
    }

    pub fn affected_neighborhood(&self, seed: usize, max_hop: usize) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut frontier = vec![(seed, 0usize)];
        while let Some((node, depth)) = frontier.pop() {
            if !seen.insert(node) {
                continue;
            }
            if depth >= max_hop {
                continue;
            }
            if let Some(next) = self.adjacency.get(&node) {
                for n in next {
                    frontier.push((*n, depth + 1));
                }
            }
        }
        seen.into_iter().collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeArena<T> {
    values: Vec<T>,
}

impl<T> RuntimeArena<T> {
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub fn add(&mut self, value: T) -> usize {
        let idx = self.values.len();
        self.values.push(value);
        idx
    }

    pub fn get(&self, idx: usize) -> Option<&T> {
        self.values.get(idx)
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeGraph {
    pub nodes: RuntimeArena<GraphNode>,
    pub edges: Vec<sem_lang_core::GraphEdge>,
}

impl RuntimeGraph {
    pub fn from_core(graph: &SemanticGraph) -> Self {
        let mut nodes = RuntimeArena::new();
        for n in graph.nodes.values() {
            nodes.add(n.clone());
        }
        Self {
            nodes,
            edges: graph.edges.clone(),
        }
    }

    pub fn to_core(self) -> SemanticGraph {
        let mut out = SemanticGraph::new();
        for i in 0..self.nodes.values.len() {
            if let Some(node) = self.nodes.get(i) {
                let _ = out.add_node(node.clone());
            }
        }
        out.edges.extend(self.edges);
        out
    }
}

fn reference_to_value(reference: &sem_lang_core::Reference) -> Value {
    reference.target.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_core::{Argument, Expr, PacketId, RuntimeState, SemanticPacket, Value};

    #[test]
    fn normalize_is_deterministic() {
        let expr = Expr::Apply(sem_lang_core::Apply {
            operator: sem_lang_core::TermId::new_random(1, 1),
            arguments: vec![
                Argument {
                    role: None,
                    value: Value::Text("b".to_string()),
                },
                Argument {
                    role: None,
                    value: Value::Text("a".to_string()),
                },
            ],
        });
        let a = ExpressionNormalizer::normalize(&expr);
        let b = ExpressionNormalizer::normalize(&expr);
        assert_eq!(format!("{:?}", a.expr), format!("{:?}", b.expr));
    }

    #[test]
    fn reducer_appends_packet() {
        let engine = RuntimeEngine::new(VocabularyRegistry::new());
        let packet = SemanticPacket {
            id: PacketId::new_random(1, 1),
            conversation_id: None,
            sender: None,
            recipient: None,
            cause: None,
            correlation: None,
            act: None,
            content: Expr::Value(Value::Text("ok".to_string())),
            context_delta: None,
            vocabulary_manifest: None,
            resolution: Resolution::Resolved,
            pragmatics: vec![],
            provenance: None,
        };
        let state = RuntimeState::new();
        let request = PacketRequest {
            operation_id: "test:packet".into(),
            expected_commit_revision: RevisionRef::default(),
            expected_semantic_digest: state.semantic_state_digest().unwrap(),
        };
        let transition = engine.reduce(&state, &packet, &request).unwrap();
        assert_eq!(transition.into_state().packets.len(), 1);
    }

    #[test]
    fn type_checker_rejects_unknown_references() {
        let vocab = VocabularyRegistry::new();
        let checker = TypeChecker::new();
        let expr = Expr::Value(Value::TermRef(sem_lang_core::TermId::new_random(99, 9)));

        assert!(!checker.check_expr(&expr, &vocab));
    }

    #[test]
    fn type_checker_accepts_a_term_from_registered_vocabulary() {
        let mut vocab = VocabularyRegistry::new();
        let module = sem_lang_fixtures::domain::reservation_fixture_module();
        let term = module.terms[0].id;
        vocab.register(module).unwrap();

        assert!(TypeChecker::new().check_expr(&Expr::Value(Value::TermRef(term)), &vocab));
    }

    #[test]
    fn type_constraint_checks_the_runtime_term_type() {
        let term = sem_lang_core::TermId::new_random(1, 1);
        let ty = sem_lang_core::TypeId::new_random(2, 2);
        let mut state = RuntimeState::new();
        state.types.insert(
            ty,
            sem_lang_core::Type {
                id: ty,
                name: "Entity".to_string(),
                super_types: Vec::new(),
                pure: true,
            },
        );
        state.terms.insert(
            term,
            sem_lang_core::Term {
                id: term,
                name: "entity".to_string(),
                type_id: ty,
                digest: Digest::object_digest(b"entity"),
                deprecation: false,
            },
        );

        let constraint = Constraint::TypeConstraint {
            value: Value::TermRef(term),
            ty,
        };
        assert!(ConstraintEvaluator::evaluate(&[constraint], &state));
    }

    #[test]
    fn unbound_apply_cannot_masquerade_as_a_computed_argument_map() {
        let operator = sem_lang_core::TermId::new_random(1, 1);
        let expr = Expr::Apply(sem_lang_core::Apply {
            operator,
            arguments: vec![Argument {
                role: None,
                value: Value::Text("ok".to_string()),
            }],
        });

        let value = PureEvaluator::evaluate(&expr, &RuntimeState::new());
        assert_eq!(value, None);
        let literal = Value::Map(BTreeMap::from([("kept".into(), Value::Integer(5))]));
        assert_eq!(
            PureEvaluator::evaluate(&Expr::Value(literal.clone()), &RuntimeState::new()),
            Some(literal)
        );
        for operator in [operator, sem_lang_core::TermId::new_random(1, 2)] {
            let expr = Expr::Apply(sem_lang_core::Apply {
                operator,
                arguments: vec![
                    Argument {
                        role: None,
                        value: Value::Integer(2),
                    },
                    Argument {
                        role: None,
                        value: Value::Integer(3),
                    },
                ],
            });
            assert_eq!(PureEvaluator::evaluate(&expr, &RuntimeState::new()), None);
            let engine = RuntimeEngine::new(VocabularyRegistry::new());
            let result = engine.evaluate(&expr, &CausalDag::default());
            assert_eq!(result.value, None);
            assert!(result.graph_delta.is_empty());
        }
    }
}
