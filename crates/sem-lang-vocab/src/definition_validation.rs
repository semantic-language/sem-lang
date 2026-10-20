//! Referential validation for explicitly adopted typed definitions.
use crate::{VocabularyError, VocabularyRegistry};
use sem_lang_core::{Constraint, RoleId, TermId, TypeId, Value};
use sem_lang_linguistics::{SemanticDefinition, SemanticExpr};
use std::collections::BTreeSet;

struct References {
    terms: BTreeSet<TermId>,
    types: BTreeSet<TypeId>,
    roles: BTreeSet<RoleId>,
}

impl References {
    fn expression(&self, expression: &SemanticExpr, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match expression {
            SemanticExpr::Primitive(id) | SemanticExpr::Concept(id) => self.terms.contains(id),
            SemanticExpr::Apply {
                predicate,
                arguments,
            } => {
                let names: BTreeSet<_> = arguments.iter().map(|(name, _)| name).collect();
                self.terms.contains(predicate)
                    && names.len() == arguments.len()
                    && arguments.iter().all(|(role, value)| {
                        self.roles.contains(role) && self.expression(value, depth + 1)
                    })
            }
            SemanticExpr::Conjunction(values) => {
                values.iter().all(|value| self.expression(value, depth + 1))
            }
            SemanticExpr::Defined { term, expansion } => {
                self.terms.contains(term) && self.expression(expansion, depth + 1)
            }
            SemanticExpr::Residual(value) => self.terms.contains(&value.source_term),
            SemanticExpr::Constraint(value) => self.constraint(value.value()),
            SemanticExpr::Contextual {
                association,
                variable,
            } => {
                self.terms.contains(&association.semantic_ref)
                    && self.roles.contains(&association.role)
                    && association.validity.valid()
                    && !variable.trim().is_empty()
                    && variable.len() <= 512
            }
            SemanticExpr::Variable(_) | SemanticExpr::Literal(_) => true,
        }
    }

    fn value(&self, value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match value {
            Value::TermRef(id) => self.terms.contains(id),
            Value::TypeRef(id) => self.types.contains(id),
            Value::RoleRef(id) => self.roles.contains(id),
            Value::Float(value) => value.is_finite(),
            Value::TypedHole(value) => value.expected.is_none_or(|id| self.types.contains(&id)),
            Value::Sequence(values) => values.iter().all(|value| self.value(value, depth + 1)),
            Value::Map(values) => values.values().all(|value| self.value(value, depth + 1)),
            _ => true,
        }
    }

    fn constraint(&self, value: &Constraint) -> bool {
        match value {
            Constraint::TypeConstraint { value, ty } => {
                self.types.contains(ty) && self.value(value, 0)
            }
            Constraint::RelationConstraint {
                value,
                role,
                target,
            } => self.roles.contains(role) && self.value(value, 0) && self.value(target, 0),
            Constraint::ScopeConstraint { variable, type_id } => {
                self.types.contains(type_id)
                    && variable.expected.is_none_or(|id| self.types.contains(&id))
            }
        }
    }
}

impl VocabularyRegistry {
    /// Checks structure and references, not the truth of a curator's definition
    /// or application authority. This operation never invents missing meanings.
    pub fn validate_definitions(&self) -> Result<(), VocabularyError> {
        for module in self.modules() {
            let visible = self.visible_modules(module)?;
            let refs = References {
                terms: visible
                    .iter()
                    .flat_map(|module| module.terms.iter().map(|term| term.id))
                    .collect(),
                types: visible
                    .iter()
                    .flat_map(|module| module.types.iter().map(|ty| ty.id))
                    .collect(),
                roles: visible
                    .iter()
                    .flat_map(|module| module.roles.iter().map(|role| role.id))
                    .collect(),
            };
            for term in &module.terms {
                let valid = match &term.semantic_definition {
                    Some(SemanticDefinition::Primitive(_) | SemanticDefinition::Opaque { .. }) => {
                        true
                    }
                    Some(SemanticDefinition::Composition(expression)) => {
                        refs.expression(expression, 0)
                    }
                    Some(SemanticDefinition::Alias(id)) => refs.terms.contains(id),
                    Some(SemanticDefinition::Refined { base, constraints }) => {
                        refs.terms.contains(base)
                            && constraints
                                .iter()
                                .all(|constraint| refs.constraint(constraint))
                    }
                    None => false,
                };
                if !valid {
                    return Err(VocabularyError::InvalidFormat(format!(
                        "unresolved definition reference: {}",
                        term.id
                    )));
                }
            }
        }
        Ok(())
    }
}
