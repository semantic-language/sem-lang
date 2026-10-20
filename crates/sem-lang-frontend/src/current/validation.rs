//! Request-local validation work. No persistent state, evaluator or reader.
//! Shared graph references charge a visit each time; depth alone is insufficient.
use super::{
    CurrentStateCompiler, EvaluationBound, MAX_ITEMS, ProcessingError, ensure_bound, token,
};
use sem_lang_core::{Claim, Constraint, Value};
use std::collections::BTreeSet;

pub(super) struct Validator<'a, 'state> {
    compiler: &'a CurrentStateCompiler<'state>,
    visits: usize,
}

impl<'a, 'state> Validator<'a, 'state> {
    pub(super) fn new(compiler: &'a CurrentStateCompiler<'state>) -> Self {
        Self {
            compiler,
            visits: 0,
        }
    }
    fn validate_value(&mut self, value: &Value, depth: usize) -> Result<(), ProcessingError> {
        self.visits += 1;
        ensure_bound(
            EvaluationBound::ValueWork,
            self.visits,
            MAX_ITEMS * MAX_ITEMS,
        )?;
        ensure_bound(EvaluationBound::ExpressionDepth, depth, 32)?;
        let valid = match value {
            Value::TermRef(id) => self.compiler.known_term(*id),
            Value::TypeRef(id) => self.compiler.compiled_language.known_type(*id),
            Value::RoleRef(id) => self
                .compiler
                .registry
                .modules()
                .any(|m| m.roles.iter().any(|r| r.id == *id)),
            Value::Node(id) => {
                let node = self
                    .compiler
                    .state
                    .graph
                    .nodes
                    .get(id)
                    .ok_or(ProcessingError::InvalidReference)?;
                if node.id != *id {
                    return Err(ProcessingError::InvalidReference);
                }
                match &node.expr {
                    sem_lang_core::Expr::Apply(expr) => {
                        if !self.compiler.known_term(expr.operator) {
                            return Err(ProcessingError::InvalidReference);
                        }
                        self.validate_arguments(&expr.arguments, depth + 1)?;
                    }
                    sem_lang_core::Expr::Value(value) => self.validate_value(value, depth + 1)?,
                    sem_lang_core::Expr::Binding(binding) => {
                        if !token(&binding.variable.id)
                            || binding
                                .variable
                                .expected
                                .is_some_and(|ty| !self.compiler.compiled_language.known_type(ty))
                        {
                            return Err(ProcessingError::InvalidReference);
                        }
                        self.validate_value(&binding.value, depth + 1)?;
                    }
                    sem_lang_core::Expr::TypedHole(hole) => {
                        if !token(&hole.id)
                            || hole
                                .expected
                                .is_some_and(|ty| !self.compiler.compiled_language.known_type(ty))
                        {
                            return Err(ProcessingError::InvalidReference);
                        }
                    }
                    _ => return Err(ProcessingError::UnsupportedRepresentation),
                }
                true
            }
            Value::TypedHole(hole) => {
                token(&hole.id)
                    && hole
                        .expected
                        .is_none_or(|ty| self.compiler.compiled_language.known_type(ty))
            }
            Value::Float(value) => value.is_finite(),
            Value::Text(text) => text.len() <= 16_384,
            Value::Sequence(values) if values.len() <= MAX_ITEMS => {
                for value in values {
                    self.validate_value(value, depth + 1)?;
                }
                true
            }
            Value::Map(values) if values.len() <= MAX_ITEMS => {
                for (key, value) in values {
                    if !token(key) {
                        return Err(ProcessingError::ContractViolation);
                    }
                    self.validate_value(value, depth + 1)?;
                }
                true
            }
            Value::Nil | Value::Bool(_) | Value::Integer(_) | Value::Decimal { .. } => true,
            Value::Sequence(_) | Value::Map(_) => {
                return Err(ProcessingError::EvaluationLimitExceeded {
                    bound: EvaluationBound::ValueItems,
                    limit: MAX_ITEMS,
                    observed: match value {
                        Value::Sequence(v) => v.len(),
                        Value::Map(v) => v.len(),
                        _ => unreachable!(),
                    },
                });
            }
            _ => return Err(ProcessingError::UnsupportedRepresentation),
        };
        if valid {
            Ok(())
        } else {
            Err(ProcessingError::InvalidReference)
        }
    }

    pub(super) fn validate_claim(&mut self, claim: &Claim) -> Result<(), ProcessingError> {
        ensure_bound(EvaluationBound::Arguments, claim.arguments.len(), MAX_ITEMS)?;
        ensure_bound(
            EvaluationBound::Constraints,
            claim.constraints.len(),
            MAX_ITEMS,
        )?;
        if !self.compiler.known_term(claim.predicate)
            || claim
                .validity
                .valid_from
                .zip(claim.validity.valid_until)
                .is_some_and(|(a, b)| a >= b)
        {
            return Err(ProcessingError::ContractViolation);
        }
        for constraint in &claim.constraints {
            self.validate_constraint(constraint)?;
        }
        self.validate_arguments(&claim.arguments, 0)
    }

    fn validate_arguments(
        &mut self,
        arguments: &[sem_lang_core::Argument],
        depth: usize,
    ) -> Result<(), ProcessingError> {
        ensure_bound(EvaluationBound::ExpressionDepth, depth, 32)?;
        ensure_bound(EvaluationBound::Arguments, arguments.len(), MAX_ITEMS)?;
        let mut seen = BTreeSet::new();
        for argument in arguments {
            let role = argument.role.ok_or(ProcessingError::ContractViolation)?;
            if !seen.insert(role)
                || !self
                    .compiler
                    .registry
                    .modules()
                    .any(|m| m.roles.iter().any(|r| r.id == role))
            {
                return Err(ProcessingError::ContractViolation);
            }
            self.validate_value(&argument.value, depth)?;
        }
        Ok(())
    }

    pub(super) fn validate_constraint(
        &mut self,
        constraint: &Constraint,
    ) -> Result<(), ProcessingError> {
        match constraint {
            Constraint::TypeConstraint { value, ty } => {
                if !self.compiler.compiled_language.known_type(*ty) {
                    return Err(ProcessingError::InvalidReference);
                }
                self.validate_value(value, 0)
            }
            Constraint::ScopeConstraint { variable, type_id } => {
                if !token(&variable.id) {
                    return Err(ProcessingError::ContractViolation);
                }
                if !self.compiler.compiled_language.known_type(*type_id)
                    || variable
                        .expected
                        .is_some_and(|ty| !self.compiler.compiled_language.known_type(ty))
                {
                    return Err(ProcessingError::InvalidReference);
                }
                Ok(())
            }
            Constraint::RelationConstraint {
                value,
                role,
                target,
            } => {
                if !self
                    .compiler
                    .registry
                    .modules()
                    .any(|m| m.roles.iter().any(|r| r.id == *role))
                {
                    return Err(ProcessingError::InvalidReference);
                }
                self.validate_value(value, 0)?;
                self.validate_value(target, 0)?;
                if !matches!(value, Value::TermRef(_)) {
                    return Err(ProcessingError::UnsupportedRepresentation);
                }
                Ok(())
            }
        }
    }
}
