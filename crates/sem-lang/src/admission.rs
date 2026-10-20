//! Definition-backed reference admission at the semantic persistence boundary.
//! Instance/transport/provenance identifiers and text are deliberately opaque.
use sem_lang_core::{
    Argument, Claim, Constraint, Expr, RoleId, RuntimeState, SemanticPacket, TermId, TypeId, Value,
};
use sem_lang_vocab::VocabularyRegistry;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
pub enum SemanticReferenceError {
    #[error("unadopted semantic term: {0}")]
    UnknownTerm(TermId),
    #[error("unadopted semantic type: {0}")]
    UnknownType(TypeId),
    #[error("unadopted semantic role: {0}")]
    UnknownRole(RoleId),
    #[error("semantic reference traversal capacity exceeded")]
    CapacityExceeded,
}
type Result<T = ()> = std::result::Result<T, SemanticReferenceError>;

pub(crate) struct Admission {
    terms: BTreeSet<TermId>,
    types: BTreeSet<TypeId>,
    roles: BTreeSet<RoleId>,
    remaining: usize,
}
impl Admission {
    pub(crate) fn new(registry: &VocabularyRegistry) -> Self {
        let mut value = Self {
            terms: BTreeSet::new(),
            types: BTreeSet::new(),
            roles: BTreeSet::new(),
            remaining: 65_536,
        };
        for module in registry.modules() {
            value.terms.extend(
                module
                    .terms
                    .iter()
                    .filter(|t| t.semantic_definition.is_some())
                    .map(|t| t.id),
            );
            value.types.extend(module.types.iter().map(|t| t.id));
            value.roles.extend(module.roles.iter().map(|r| r.id));
        }
        value
    }
    fn visit(&mut self, depth: usize) -> Result {
        if depth > 128 || self.remaining == 0 {
            return Err(SemanticReferenceError::CapacityExceeded);
        }
        self.remaining -= 1;
        Ok(())
    }
    fn term(&self, id: TermId) -> Result {
        if self.terms.contains(&id) {
            Ok(())
        } else {
            Err(SemanticReferenceError::UnknownTerm(id))
        }
    }
    fn ty(&self, id: TypeId) -> Result {
        if self.types.contains(&id) {
            Ok(())
        } else {
            Err(SemanticReferenceError::UnknownType(id))
        }
    }
    fn role(&self, id: RoleId) -> Result {
        if self.roles.contains(&id) {
            Ok(())
        } else {
            Err(SemanticReferenceError::UnknownRole(id))
        }
    }
    fn optional_type(&self, id: Option<TypeId>) -> Result {
        id.map_or(Ok(()), |id| self.ty(id))
    }
    fn value(&mut self, value: &Value, depth: usize) -> Result {
        self.visit(depth)?;
        match value {
            Value::TermRef(id) => self.term(*id),
            Value::TypeRef(id) => self.ty(*id),
            Value::RoleRef(id) => self.role(*id),
            Value::TypedHole(h) => self.optional_type(h.expected),
            Value::Sequence(values) => {
                for value in values {
                    self.value(value, depth + 1)?;
                }
                Ok(())
            }
            Value::Map(values) => {
                for value in values.values() {
                    self.value(value, depth + 1)?;
                }
                Ok(())
            }
            Value::Nil
            | Value::Bool(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::Text(_)
            | Value::Decimal { .. }
            | Value::Entity(_)
            | Value::Node(_)
            | Value::Expr(_) => Ok(()),
        }
    }
    fn argument(&mut self, arg: &Argument) -> Result {
        if let Some(role) = arg.role {
            self.role(role)?;
        }
        self.value(&arg.value, 0)
    }
    fn expr(&mut self, expr: &Expr) -> Result {
        self.visit(0)?;
        match expr {
            Expr::Value(value) => self.value(value, 0),
            Expr::Apply(value) => {
                self.term(value.operator)?;
                for arg in &value.arguments {
                    self.argument(arg)?;
                }
                Ok(())
            }
            Expr::Binding(value) => {
                self.optional_type(value.variable.expected)?;
                self.value(&value.value, 0)
            }
            Expr::Reference(value) => self.value(&value.target, 0),
            Expr::TypedHole(value) => self.optional_type(value.expected),
        }
    }
    fn claim(&mut self, claim: &Claim) -> Result {
        self.visit(0)?;
        self.term(claim.predicate)?;
        if let Some(actor) = &claim.actor {
            self.value(actor, 0)?;
        }
        for arg in &claim.arguments {
            self.argument(arg)?;
        }
        for constraint in &claim.constraints {
            self.visit(0)?;
            match constraint {
                Constraint::TypeConstraint { value, ty } => {
                    self.ty(*ty)?;
                    self.value(value, 0)?;
                }
                Constraint::RelationConstraint {
                    value,
                    role,
                    target,
                } => {
                    self.role(*role)?;
                    self.value(value, 0)?;
                    self.value(target, 0)?;
                }
                Constraint::ScopeConstraint { variable, type_id } => {
                    self.optional_type(variable.expected)?;
                    self.ty(*type_id)?;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn packet(&mut self, packet: &SemanticPacket) -> Result {
        self.visit(0)?;
        if let Some(act) = packet.act {
            self.term(act)?;
        }
        self.expr(&packet.content)?;
        if let Some(delta) = &packet.context_delta {
            for claim in delta.added_claims.iter().chain(&delta.removed_claims) {
                self.claim(claim)?;
            }
        }
        for value in &packet.pragmatics {
            self.value(value, 0)?;
        }
        Ok(())
    }
    pub(crate) fn state(&mut self, state: &RuntimeState) -> Result {
        for packet in state.packets.values() {
            self.packet(packet)?;
        }
        for node in state.graph.nodes.values() {
            self.expr(&node.expr)?;
        }
        for edge in &state.graph.edges {
            self.role(edge.role)?;
            self.value(&edge.target, 0)?;
        }
        for claim in state.claims.values() {
            self.claim(claim)?;
        }
        for (id, term) in &state.terms {
            self.visit(0)?;
            self.term(*id)?;
            self.term(term.id)?;
            self.ty(term.type_id)?;
        }
        for (id, ty) in &state.types {
            self.visit(0)?;
            self.ty(*id)?;
            self.ty(ty.id)?;
            for parent in &ty.super_types {
                self.ty(*parent)?;
            }
        }
        for (id, role) in &state.roles {
            self.visit(0)?;
            self.role(*id)?;
            self.role(role.id)?;
            if let Some(parent) = role.parent {
                self.role(parent)?;
            }
        }
        Ok(())
    }
}
