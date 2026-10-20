//! One type traversal for current lexical applicability and expression checks.
//! Text, numbers and presentation labels never manufacture semantic types.
use super::CompiledLanguage;
use sem_lang_core::{RuntimeState, TypeId, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TypeEvaluationError {
    #[error("invalid type reference")]
    InvalidReference,
    #[error("type traversal exceeded {limit} nodes (observed {observed})")]
    LimitExceeded { limit: usize, observed: usize },
}

impl CompiledLanguage {
    pub fn known_type(&self, id: TypeId) -> bool {
        self.registry
            .modules()
            .any(|m| m.types.iter().any(|t| t.id == id))
    }

    /// None means no explicit type evidence, not a representation-derived type.
    pub fn matches_type(
        &self,
        value: &Value,
        state: &RuntimeState,
        expected: TypeId,
    ) -> Result<Option<bool>, TypeEvaluationError> {
        if !self.known_type(expected) {
            return Err(TypeEvaluationError::InvalidReference);
        }
        let actual = match value {
            Value::TypeRef(id) => *id,
            Value::TermRef(id) => {
                let Some(term) = state.terms.get(id) else {
                    return Ok(None);
                };
                if term.id != *id {
                    return Err(TypeEvaluationError::InvalidReference);
                }
                term.type_id
            }
            _ => return Ok(None),
        };
        if !self.known_type(actual) {
            return Err(TypeEvaluationError::InvalidReference);
        }
        let mut pending = vec![actual];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if seen.len() > 64 {
                return Err(TypeEvaluationError::LimitExceeded {
                    limit: 64,
                    observed: seen.len(),
                });
            }
            if id == expected {
                return Ok(Some(true));
            }
            let ty = self
                .registry
                .modules()
                .flat_map(|m| &m.types)
                .find(|ty| ty.id == id)
                .ok_or(TypeEvaluationError::InvalidReference)?;
            pending.extend(ty.super_types.iter().copied());
        }
        Ok(Some(false))
    }
}
