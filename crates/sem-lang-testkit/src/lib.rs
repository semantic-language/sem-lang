//! Shared helper utilities for sem-lang tests.

use sem_lang_core::{
    Argument, Expr, GraphDelta, NodeId, RuntimeState, SemanticGraph, SemanticPacket,
};
use sem_lang_runtime::RuntimeEngine;
use sem_lang_vocab::VocabularyRegistry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct DeterministicClock {
    now: i64,
    step: i64,
}

impl DeterministicClock {
    pub fn new(start: i64, step: i64) -> Self {
        Self { now: start, step }
    }

    pub fn now(&self) -> i64 {
        self.now
    }

    pub fn tick(&mut self) -> i64 {
        self.now += self.step;
        self.now
    }

    pub fn from_unix_epoch() -> Self {
        let start = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(_) => 0,
        };
        Self {
            now: start,
            step: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicIdGenerator {
    seed: u64,
}

impl DeterministicIdGenerator {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.seed ^= self.seed << 7;
        self.seed ^= self.seed >> 9;
        self.seed ^= self.seed << 8;
        self.seed
    }
}

#[derive(Debug, Clone)]
pub struct ReferenceEngine {
    state: RuntimeState,
}

impl ReferenceEngine {
    pub fn new(state: RuntimeState) -> Self {
        Self { state }
    }

    pub fn apply_packet(
        &mut self,
        packet: SemanticPacket,
    ) -> Result<RuntimeState, sem_lang_runtime::PacketApplyError> {
        let runtime = RuntimeEngine::new(VocabularyRegistry::new());
        let request = sem_lang_runtime::PacketRequest {
            operation_id: packet.id.to_string(),
            expected_commit_revision: sem_lang_runtime::PacketReducer::commit_revision(
                &self.state,
            )?,
            expected_semantic_digest: self.state.semantic_state_digest()?,
        };
        let transition = runtime.reduce(&self.state, &packet, &request)?;
        self.state = transition.into_state();
        Ok(self.state.clone())
    }

    pub fn state(&self) -> &RuntimeState {
        &self.state
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReferenceGraph {
    adjacency: BTreeMap<NodeId, BTreeSet<NodeId>>,
}

impl ReferenceGraph {
    pub fn new() -> Self {
        Self {
            adjacency: BTreeMap::new(),
        }
    }

    pub fn connect(&mut self, from: NodeId, to: NodeId) {
        self.adjacency.entry(from).or_default().insert(to);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub data: serde_json::Value,
}

pub fn load_fixture(path: &Path) -> Fixture {
    let raw = fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    Fixture {
        name: path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("fixture")
            .to_string(),
        data,
    }
}

pub fn arbitrary_u32(seed: &mut DeterministicIdGenerator) -> u32 {
    (seed.next_u64() % 1000) as u32
}

pub fn arbitrary_type(seed: &mut DeterministicIdGenerator) -> sem_lang_core::TypeId {
    sem_lang_core::TypeId::new_random(seed.next_u64(), 0x33)
}

pub fn arbitrary_term(seed: &mut DeterministicIdGenerator) -> sem_lang_core::TermId {
    sem_lang_core::TermId::new_random(seed.next_u64(), 0x11)
}

pub fn arbitrary_expr(seed: &mut DeterministicIdGenerator, depth: usize) -> Expr {
    if depth == 0 {
        return Expr::Value(sem_lang_core::Value::Text(format!(
            "text-{}",
            seed.next_u64()
        )));
    }
    let op = arbitrary_term(seed);
    let mut args = Vec::new();
    for i in 0..2 {
        let _ = i;
        args.push(Argument {
            role: None,
            value: sem_lang_core::Value::Text(format!("a-{}", seed.next_u64())),
        });
    }
    Expr::Apply(sem_lang_core::Apply {
        operator: op,
        arguments: args,
    })
}

pub fn arbitrary_graph(seed: &mut DeterministicIdGenerator, nodes: usize) -> SemanticGraph {
    let mut graph = SemanticGraph::new();
    for idx in 0..nodes {
        let node_id = NodeId::new_random(seed.next_u64(), (idx % 255) as u8);
        let _ = graph.add_node(sem_lang_core::GraphNode {
            id: node_id,
            expr: arbitrary_expr(seed, 2),
            created_by: None,
        });
    }
    graph
}

pub fn arbitrary_delta(seed: &mut DeterministicIdGenerator, depth: usize) -> Vec<GraphDelta> {
    let mut out = Vec::new();
    for _ in 0..depth {
        let id = NodeId::new_random(seed.next_u64(), 0x44);
        out.push(GraphDelta::AddNode(sem_lang_core::GraphNode {
            id,
            expr: arbitrary_expr(seed, 1),
            created_by: None,
        }));
    }
    out
}

pub fn assert_semantic_equiv(left: &RuntimeState, right: &RuntimeState) {
    assert_eq!(
        left.proposition_digest()
            .expect("valid left state")
            .to_hex(),
        right
            .proposition_digest()
            .expect("valid right state")
            .to_hex()
    );
}

pub fn assert_invariant<T: Debug + PartialEq>(left: T, right: T) {
    assert_eq!(left, right);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_clock_and_ids() {
        let mut c = DeterministicClock::new(1, 1);
        assert_eq!(c.tick(), 2);
        let mut id = DeterministicIdGenerator::new(12);
        assert_ne!(id.next_u64(), id.next_u64());
    }

    #[test]
    fn make_graph() {
        let mut id = DeterministicIdGenerator::new(1);
        let g = arbitrary_graph(&mut id, 3);
        assert!(!g.nodes.is_empty());
    }
}
