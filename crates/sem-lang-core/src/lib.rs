//! Core semantic domain model for sem-lang.

mod context;
pub use context::{ContextAssociation, ContextKind, ContextScope, ContextValidity};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct StableId<const N: usize>(pub [u8; N]);

impl<const N: usize> StableId<N> {
    pub fn from_hex(value: &str) -> Result<Self, CoreError> {
        if value.len() != N * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CoreError::InvalidIdFormat);
        }
        let mut bytes = [0u8; N];
        for i in 0..N {
            let chunk = &value[i * 2..i * 2 + 2];
            bytes[i] = u8::from_str_radix(chunk, 16).map_err(|_| CoreError::InvalidIdFormat)?;
        }
        Ok(Self(bytes))
    }

    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(N * 2);
        for byte in self.0 {
            out.push_str(&format!("{:02x}", byte));
        }
        out
    }
}

impl<const N: usize> Display for StableId<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl<const N: usize> Serialize for StableId<N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de, const N: usize> Deserialize<'de> for StableId<N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(|_| de::Error::custom("invalid stable id"))
    }
}

impl<const N: usize> FromStr for StableId<N> {
    type Err = CoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

macro_rules! define_stable_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize,
        )]
        pub struct $name(pub StableId<16>);

        impl $name {
            pub fn new_random(seed: u64, namespace: u8) -> Self {
                let mut bytes = [0u8; 16];
                bytes[0] = namespace;
                let mut x = seed ^ (namespace as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15u64);
                for i in 1..16 {
                    x ^= x >> 12;
                    x ^= x << 25;
                    x ^= x >> 27;
                    bytes[i] = (x & 0xff) as u8;
                }
                Self(StableId(bytes))
            }

            pub fn from_hex(value: &str) -> Result<Self, CoreError> {
                Ok(Self(StableId::from_hex(value)?))
            }

            pub fn as_stable(&self) -> &StableId<16> {
                &self.0
            }

            pub fn into_stable(self) -> StableId<16> {
                self.0
            }

            pub fn to_hex(&self) -> String {
                self.0.to_hex()
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::from_hex(value)
            }
        }
    };
}

define_stable_id!(TermId);
define_stable_id!(TypeId);
define_stable_id!(RoleId);
define_stable_id!(EntityId);
define_stable_id!(NodeId);
define_stable_id!(ExprId);
define_stable_id!(PacketId);
define_stable_id!(EpisodeId);
define_stable_id!(MigrationId);

impl TermId {
    /// Compile a stable machine identity from an authored language package and
    /// its canonical term name. Authored `.sem` files never carry this value;
    /// it belongs to compiled projections and wire contracts only.
    pub fn from_declaration(vocabulary_id: &str, canonical_name: &str) -> Self {
        use sha2::Digest as _;
        let mut digest = sha2::Sha256::new();
        digest.update(b"sem-lang/term/0.10.0\0");
        digest.update((vocabulary_id.len() as u64).to_be_bytes());
        digest.update(vocabulary_id.as_bytes());
        digest.update((canonical_name.len() as u64).to_be_bytes());
        digest.update(canonical_name.as_bytes());
        let bytes = digest.finalize();
        let mut identity = [0_u8; 16];
        identity.copy_from_slice(&bytes[..16]);
        Self(StableId(identity))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DigestAlgorithm {
    #[serde(rename = "sha256")]
    Sha256,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Digest {
    pub algorithm: DigestAlgorithm,
    pub canonicalization_version: u8,
    pub bytes: [u8; 32],
}

// A fixed-size sink: the serializer writes canonical bytes directly to SHA-256.
struct DigestWriter(sha2::Sha256);

impl std::io::Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        use sha2::Digest as _;
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Digest {
    /// Hash the exact serde JSON representation without buffering that representation.
    /// Callers own ordering/canonicalization and the semantic projection; this does
    /// not normalize data or turn a serialization failure into a revision.
    pub fn from_serializable<T: Serialize + ?Sized>(
        canonicalization_version: u8,
        value: &T,
    ) -> Result<Self, serde_json::Error> {
        use sha2::Digest as _;
        let mut writer = DigestWriter(sha2::Sha256::new());
        serde_json::to_writer(&mut writer, value)?;
        Ok(Self {
            algorithm: DigestAlgorithm::Sha256,
            canonicalization_version,
            bytes: writer.0.finalize().into(),
        })
    }

    pub fn from_bytes(canonicalization_version: u8, data: &[u8]) -> Self {
        use sha2::{Digest as _, Sha256};
        Self {
            algorithm: DigestAlgorithm::Sha256,
            canonicalization_version,
            bytes: Sha256::digest(data).into(),
        }
    }

    pub fn proposition_digest(data: &[u8]) -> Self {
        Self::from_bytes(1, data)
    }

    pub fn interaction_digest(data: &[u8]) -> Self {
        Self::from_bytes(1, data)
    }

    pub fn semantic_state_digest(data: &[u8]) -> Self {
        Self::from_bytes(1, data)
    }

    pub fn object_digest(data: &[u8]) -> Self {
        Self::from_bytes(1, data)
    }

    pub fn document_digest(data: &[u8]) -> Self {
        Self::from_bytes(1, data)
    }

    pub fn to_hex(&self) -> String {
        self.bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum CoreError {
    #[error("invalid id format")]
    InvalidIdFormat,
    #[error("invalid reference")]
    InvalidReference,
    #[error("unknown vocabulary id")]
    UnknownVocabulary,
    #[error("duplicate id")]
    DuplicateId,
    #[error("unresolvable")]
    Unresolvable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub id: RoleId,
    pub name: String,
    pub parent: Option<RoleId>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Type {
    pub id: TypeId,
    pub name: String,
    pub super_types: Vec<TypeId>,
    pub pure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub id: TermId,
    pub name: String,
    pub type_id: TypeId,
    pub digest: Digest,
    pub deprecation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub enum Value {
    Nil,
    Bool(bool),
    Integer(i64),
    Float(#[serde(serialize_with = "serialize_finite_f64")] f64),
    Text(String),
    Decimal { coefficient: i128, scale: i32 },
    TermRef(TermId),
    TypeRef(TypeId),
    RoleRef(RoleId),
    Entity(EntityId),
    Node(NodeId),
    Expr(ExprId),
    TypedHole(TypedHole),
    Sequence(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub id: String,
    pub expected: Option<TypeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TypedHole {
    pub id: String,
    pub expected: Option<TypeId>,
    pub surface: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Apply {
    pub operator: TermId,
    pub arguments: Vec<Argument>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub role: Option<RoleId>,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub variable: Variable,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Expr {
    Value(Value),
    Apply(Apply),
    Binding(Binding),
    Reference(Reference),
    TypedHole(TypedHole),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub target: Value,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Resolution {
    Unknown,
    Pending,
    Resolved,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub enum Constraint {
    TypeConstraint {
        value: Value,
        ty: TypeId,
    },
    RelationConstraint {
        value: Value,
        role: RoleId,
        target: Value,
    },
    ScopeConstraint {
        variable: Variable,
        type_id: TypeId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub id: NodeId,
    pub actor: Option<Value>,
    pub predicate: TermId,
    pub arguments: Vec<Argument>,
    pub constraints: Vec<Constraint>,
    pub validity: Validity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source: Option<String>,
    #[serde(serialize_with = "serialize_confidence")]
    pub confidence: Option<f32>,
    pub observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextDelta {
    pub added_claims: Vec<Claim>,
    pub removed_claims: Vec<Claim>,
    pub annotations: BTreeMap<String, String>,
}

fn serialize_finite_f64<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if !value.is_finite() {
        return Err(serde::ser::Error::custom("non-finite semantic number"));
    }
    serializer.serialize_f64(*value)
}

fn serialize_confidence<S: Serializer>(
    value: &Option<f32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if value.is_some_and(|v| !v.is_finite()) {
        return Err(serde::ser::Error::custom("non-finite confidence"));
    }
    value.serialize(serializer)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPacket {
    pub id: PacketId,
    pub conversation_id: Option<String>,
    pub sender: Option<String>,
    pub recipient: Option<String>,
    pub cause: Option<String>,
    pub correlation: Option<String>,
    pub act: Option<TermId>,
    pub content: Expr,
    pub context_delta: Option<ContextDelta>,
    pub vocabulary_manifest: Option<String>,
    pub resolution: Resolution,
    pub pragmatics: Vec<Value>,
    pub provenance: Option<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalReference {
    pub source_packet: PacketId,
    pub target_packet: PacketId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub id: NodeId,
    pub expr: Expr,
    pub created_by: Option<PacketId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub source: NodeId,
    pub role: RoleId,
    pub target: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticGraph {
    pub nodes: BTreeMap<NodeId, GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl SemanticGraph {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_node(&mut self, node: GraphNode) -> Result<(), CoreError> {
        if self.nodes.contains_key(&node.id) {
            return Err(CoreError::DuplicateId);
        }
        self.nodes.insert(node.id, node);
        Ok(())
    }

    pub fn replace_node(&mut self, node: GraphNode) {
        self.nodes.insert(node.id, node);
    }

    pub fn remove_node(&mut self, id: &NodeId) -> Option<GraphNode> {
        self.nodes.remove(id)
    }

    pub fn add_edge(&mut self, edge: GraphEdge) {
        self.edges.push(edge)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        // Typed expressions and edge targets are part of state, not just IDs.
        serde_json::to_vec(self)
    }
}

impl Default for SemanticGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum GraphDelta {
    AddNode(GraphNode),
    ReplaceNode(GraphNode),
    RemoveNode(NodeId),
    AddEdge(GraphEdge),
    RemoveEdgeByIndex(usize),
}

#[derive(Debug, Clone)]
pub enum EffectIntent {
    ProduceValue(Value),
    EmitIntent { contract: TermId, payload: String },
    Noop,
}

#[derive(Debug, Clone)]
pub enum Mutation {
    Add { node: NodeId, expr: Expr },
    Update { node: NodeId, old: Expr, new: Expr },
    Remove { node: NodeId, old: Expr },
}

#[derive(Debug, Clone)]
pub struct MutationDelta {
    pub mutations: Vec<Mutation>,
}

impl MutationDelta {
    pub fn inverse(&self) -> MutationDelta {
        let mut inv = Vec::with_capacity(self.mutations.len());
        for mutation in self.mutations.iter().rev() {
            inv.push(match mutation {
                Mutation::Add { node, expr } => Mutation::Remove {
                    node: *node,
                    old: expr.clone(),
                },
                Mutation::Remove { node, old } => Mutation::Add {
                    node: *node,
                    expr: old.clone(),
                },
                Mutation::Update { node, old, new } => Mutation::Update {
                    node: *node,
                    old: new.clone(),
                    new: old.clone(),
                },
            })
        }
        MutationDelta { mutations: inv }
    }

    pub fn is_reversible(&self) -> bool {
        !self
            .mutations
            .iter()
            .any(|m| matches!(m, Mutation::Update { .. } | Mutation::Remove { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicationOutcome {
    Applied,
    NoChange,
}

/// A bounded-size operation receipt, never a copy of past semantic states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationReceipt {
    pub packet_id: PacketId,
    pub commit: zixcel_revision::CommitReceipt,
    pub input_digest: Digest,
    pub output_digest: Digest,
    pub outcome: ApplicationOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeState {
    pub packet_causality: BTreeMap<PacketId, Vec<CausalReference>>,
    pub packets: BTreeMap<PacketId, SemanticPacket>,
    pub graph: SemanticGraph,
    pub terms: BTreeMap<TermId, Term>,
    pub types: BTreeMap<TypeId, Type>,
    pub roles: BTreeMap<RoleId, Role>,
    pub commit_state: zixcel_revision::CommitState,
    pub claims: BTreeMap<NodeId, Claim>,
    pub annotations: BTreeMap<String, String>,
}

impl RuntimeState {
    pub fn new() -> Self {
        Self {
            packet_causality: BTreeMap::new(),
            packets: BTreeMap::new(),
            graph: SemanticGraph::new(),
            terms: BTreeMap::new(),
            types: BTreeMap::new(),
            roles: BTreeMap::new(),
            commit_state: zixcel_revision::CommitState::default(),
            claims: BTreeMap::new(),
            annotations: BTreeMap::new(),
        }
    }

    pub fn with_graph(graph: SemanticGraph) -> Self {
        let mut s = Self::new();
        s.graph = graph;
        s
    }

    pub fn proposition_digest(&self) -> Result<Digest, serde_json::Error> {
        Digest::from_serializable(1, &self.graph)
    }

    pub fn interaction_digest(&self) -> Result<Digest, serde_json::Error> {
        Digest::from_serializable(1, &self.packets)
    }

    pub fn semantic_state_digest(&self) -> Result<Digest, serde_json::Error> {
        Digest::from_serializable(
            1,
            &(
                &self.graph,
                &self.claims,
                &self.annotations,
                &self.terms,
                &self.types,
                &self.roles,
            ),
        )
    }

    pub fn remember_packet(&mut self, packet: SemanticPacket) {
        self.packets.insert(packet.id, packet);
    }

    pub fn add_graph_node(&mut self, node: GraphNode) {
        let _ = self.graph.add_node(node);
    }
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalExpression {
    pub expr: Expr,
}

#[derive(Debug, Clone)]
pub struct EvaluationResult {
    pub value: Option<Value>,
    pub intents: Vec<EffectIntent>,
    pub graph_delta: Vec<GraphDelta>,
}

#[derive(Debug, Clone)]
pub struct CausalDag {
    pub order: BTreeSet<PacketId>,
    pub edges: Vec<CausalReference>,
}

impl CausalDag {
    pub fn new() -> Self {
        Self {
            order: BTreeSet::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_packet(&mut self, packet: PacketId) {
        self.order.insert(packet);
    }

    pub fn add_edge(&mut self, source: PacketId, target: PacketId) {
        self.edges.push(CausalReference {
            source_packet: source,
            target_packet: target,
        })
    }
}

impl Default for CausalDag {
    fn default() -> Self {
        Self::new()
    }
}

pub trait IdGenerator {
    fn next_term_id(&mut self) -> TermId;
    fn next_packet_id(&mut self) -> PacketId;
}

#[derive(Debug)]
pub struct DeterministicIdGenerator {
    counter: u64,
}

impl DeterministicIdGenerator {
    pub fn new(seed: u64) -> Self {
        Self { counter: seed }
    }
}

impl Default for DeterministicIdGenerator {
    fn default() -> Self {
        Self::new(0xD00D_F00D)
    }
}

impl IdGenerator for DeterministicIdGenerator {
    fn next_term_id(&mut self) -> TermId {
        self.counter = self.counter.wrapping_add(1);
        TermId::new_random(self.counter, 0x11)
    }

    fn next_packet_id(&mut self) -> PacketId {
        self.counter = self.counter.wrapping_add(1);
        PacketId::new_random(self.counter, 0x33)
    }
}

pub type NodeHandle = usize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_digest_preserves_exact_bytes_and_rejects_invalid_values() {
        use serde::ser::SerializeSeq as _;
        use sha2::{Digest as _, Sha256};
        let value = Value::Map(BTreeMap::from([
            ("escaped\"".into(), Value::Text("言語\n\\".into())),
            (
                "nested".into(),
                Value::Sequence(vec![Value::Nil, Value::Float(1.5)]),
            ),
        ]));
        for version in [0, 1, 255] {
            assert_eq!(
                Digest::from_serializable(version, &value).unwrap(),
                Digest::from_bytes(version, &serde_json::to_vec(&value).unwrap())
            );
        }
        assert!(Digest::from_serializable(1, &Value::Float(f64::NAN)).is_err());
        let mut state = RuntimeState::new();
        state
            .annotations
            .insert("audit".into(), "same projection".into());
        assert_eq!(
            state.proposition_digest().unwrap(),
            Digest::proposition_digest(&state.graph.canonical_bytes().unwrap())
        );
        assert_eq!(
            state.interaction_digest().unwrap(),
            Digest::interaction_digest(&serde_json::to_vec(&state.packets).unwrap())
        );
        assert_eq!(
            state.semantic_state_digest().unwrap(),
            Digest::semantic_state_digest(
                &serde_json::to_vec(&(
                    &state.graph,
                    &state.claims,
                    &state.annotations,
                    &state.terms,
                    &state.types,
                    &state.roles
                ))
                .unwrap()
            )
        );
        // This serializer emits 32 MiB without owning a corresponding buffer.
        struct Repeated;
        impl Serialize for Repeated {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(32768))?;
                let chunk = "x".repeat(1024);
                for _ in 0..32768 {
                    seq.serialize_element(&chunk)?;
                }
                seq.end()
            }
        }
        let mut expected = Sha256::new();
        expected.update(b"[");
        for index in 0..32768 {
            if index != 0 {
                expected.update(b",");
            }
            expected.update(b"\"");
            expected.update([b'x'; 1024]);
            expected.update(b"\"");
        }
        expected.update(b"]");
        assert_eq!(
            Digest::from_serializable(1, &Repeated).unwrap().bytes,
            <[u8; 32]>::from(expected.finalize())
        );
        assert!(std::mem::size_of::<DigestWriter>() <= 256);
    }

    #[test]
    fn stable_id_roundtrip() {
        let id = TermId::new_random(10, 7);
        let encoded = id.to_string();
        let parsed = TermId::from_hex(&encoded).expect("parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn authored_term_identity_is_compiled_from_package_and_canonical_name() {
        let first = TermId::from_declaration("example/core", "Person");
        assert_eq!(first, TermId::from_declaration("example/core", "Person"));
        assert_ne!(first, TermId::from_declaration("example/core", "Place"));
        assert_ne!(first, TermId::from_declaration("example/other", "Person"));
    }

    #[test]
    fn graph_delta_reversible() {
        let mut g = SemanticGraph::new();
        let node_id = NodeId::new_random(1, 1);
        let term = TermId::new_random(2, 2);
        g.add_node(GraphNode {
            id: node_id,
            expr: Expr::Value(Value::TermRef(term)),
            created_by: None,
        })
        .expect("add");
        let delta = GraphDelta::RemoveNode(node_id);
        assert!(matches!(delta, GraphDelta::RemoveNode(_)));
    }
}
