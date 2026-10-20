//! Typed constraint evaluation over the exact current state. No string parser.
use super::{
    ClaimConstraintResult, ConstraintStatus, CurrentStateCompiler, EvaluationBound, MAX_ITEMS,
    ProcessingError, ensure_bound,
};
use sem_lang_core::{Binding, Claim, Constraint, TypeId, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Truth {
    True,
    False,
    Unknown,
    Conflict,
}

impl CurrentStateCompiler<'_> {
    pub(super) fn resolved_constraint_truth(
        &self,
        constraint: &Constraint,
        result: &super::SemanticResolution,
        claims: &[&Claim],
    ) -> Result<Truth, ProcessingError> {
        if let Constraint::ScopeConstraint { variable, .. } = constraint
            && let Some(relation) = result
                .relations
                .iter()
                .find(|r| r.variables.contains(&variable.id))
        {
            let mut truth = if relation.rows.is_empty() {
                Truth::Unknown
            } else {
                Truth::True
            };
            for row in &relation.rows {
                match self.constraint_truth(constraint, row, claims)? {
                    Truth::False => return Ok(Truth::False),
                    Truth::Conflict => return Ok(Truth::Conflict),
                    Truth::Unknown => truth = Truth::Unknown,
                    Truth::True => (),
                }
            }
            return Ok(truth);
        }
        self.constraint_truth(constraint, &result.bindings, claims)
    }

    /// Certain facts grow and possible facts shrink. A fact is not admitted if
    /// a still-possible premise could invalidate its declared upper bound.
    /// Both sets are bounded by the explicit reference list; neither is stored.
    pub(super) fn applicable_claims<'a>(
        &self,
        claims: &[&'a Claim],
    ) -> Result<(Vec<&'a Claim>, Vec<ClaimConstraintResult>), ProcessingError> {
        ensure_bound(
            EvaluationBound::Constraints,
            claims.iter().map(|claim| claim.constraints.len()).sum(),
            MAX_ITEMS,
        )?;
        let mut certain: Vec<_> = claims
            .iter()
            .copied()
            .filter(|claim| claim.constraints.is_empty())
            .collect();
        let mut possible = claims.to_vec();
        loop {
            let mut rejected = BTreeSet::new();
            let mut admitted = Vec::new();
            for claim in &possible {
                if certain.iter().any(|c| c.id == claim.id) {
                    continue;
                }
                let mut all_true = true;
                for constraint in &claim.constraints {
                    match self.grounded_truth(claim, constraint, &certain, &possible)? {
                        Truth::False | Truth::Conflict => {
                            rejected.insert(claim.id);
                            all_true = false;
                        }
                        Truth::Unknown => all_true = false,
                        Truth::True => (),
                    }
                }
                if all_true {
                    admitted.push(*claim);
                }
            }
            if rejected.is_empty() && admitted.is_empty() {
                break;
            }
            certain.extend(admitted);
            possible.retain(|claim| !rejected.contains(&claim.id));
        }
        let unresolved: Vec<_> = possible
            .iter()
            .copied()
            .filter(|claim| !certain.iter().any(|c| c.id == claim.id))
            .collect();
        let mut dependencies = std::collections::BTreeMap::new();
        for claim in &unresolved {
            let mut peers = BTreeSet::new();
            for constraint in &claim.constraints {
                if self.grounded_truth(claim, constraint, &certain, &possible)? != Truth::Unknown {
                    continue;
                }
                if let Constraint::RelationConstraint {
                    value: Value::TermRef(predicate),
                    role,
                    target,
                } = constraint
                {
                    let has_support =
                        self.constraint_truth(constraint, &[], &certain)? == Truth::True;
                    peers.extend(
                        unresolved
                            .iter()
                            .filter(|other| {
                                other.predicate == *predicate
                                    && other.arguments.iter().any(|a| {
                                        a.role == Some(*role) && (has_support || a.value == *target)
                                    })
                            })
                            .map(|other| other.id),
                    );
                }
            }
            dependencies.insert(claim.id, peers);
        }
        let cyclic = |root| {
            let mut pending: Vec<_> = dependencies
                .get(&root)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            let mut seen = BTreeSet::new();
            while let Some(id) = pending.pop() {
                if id == root {
                    return true;
                }
                if seen.insert(id) {
                    pending.extend(dependencies.get(&id).into_iter().flatten().copied());
                }
            }
            false
        };
        let mut results = Vec::new();
        for claim in claims {
            for constraint in &claim.constraints {
                let status = match self.grounded_truth(claim, constraint, &certain, &possible)? {
                    Truth::True => ConstraintStatus::Satisfied,
                    Truth::False => ConstraintStatus::Unsatisfied,
                    Truth::Conflict => ConstraintStatus::Conflicting,
                    Truth::Unknown if cyclic(claim.id) => ConstraintStatus::Circular,
                    Truth::Unknown => ConstraintStatus::Unresolved,
                };
                results.push(ClaimConstraintResult {
                    claim_ref: claim.id,
                    constraint: constraint
                        .clone()
                        .try_into()
                        .map_err(|_| ProcessingError::ContractViolation)?,
                    status,
                });
            }
        }
        certain.sort_by_key(|claim| claim.id);
        Ok((certain, results))
    }

    fn grounded_truth(
        &self,
        claim: &Claim,
        constraint: &Constraint,
        certain: &[&Claim],
        possible: &[&Claim],
    ) -> Result<Truth, ProcessingError> {
        if let Constraint::ScopeConstraint { variable, .. } = constraint {
            // Bindings have a typed, explicit representation already. Their
            // lexical IDs are local to this Claim's direct argument references.
            let mut bindings = claim.arguments.iter().filter_map(|argument| {
                let Value::Node(id) = &argument.value else {
                    return None;
                };
                let sem_lang_core::Expr::Binding(binding) = &self.state.graph.nodes.get(id)?.expr
                else {
                    return None;
                };
                (binding.variable.id == variable.id).then_some(binding)
            });
            let Some(first) = bindings.next() else {
                return Ok(Truth::Unknown);
            };
            if bindings.any(|binding| binding != first) {
                return Ok(Truth::Conflict);
            }
            let truth = self.constraint_truth(constraint, std::slice::from_ref(first), certain)?;
            return Ok(match first.variable.expected {
                Some(ty) if truth == Truth::True => self.matches_type(&first.value, ty)?,
                _ => truth,
            });
        }
        let truth = self.constraint_truth(constraint, &[], certain)?;
        if truth == Truth::True
            && matches!(constraint, Constraint::RelationConstraint { .. })
            && self.constraint_truth(constraint, &[], possible)? == Truth::Conflict
        {
            Ok(Truth::Unknown)
        } else {
            Ok(truth)
        }
    }

    fn matches_type(&self, value: &Value, expected: TypeId) -> Result<Truth, ProcessingError> {
        Ok(
            match self
                .compiled_language
                .matches_type(value, self.state, expected)?
            {
                Some(true) => Truth::True,
                Some(false) => Truth::False,
                None => Truth::Unknown,
            },
        )
    }

    pub(super) fn allowed_value(
        &self,
        name: &str,
        value: &Value,
        constraints: &[Constraint],
    ) -> Result<bool, ProcessingError> {
        for constraint in constraints {
            if let Constraint::ScopeConstraint { variable, type_id } = constraint {
                if variable.id != name {
                    continue;
                }
                for ty in std::iter::once(*type_id).chain(variable.expected) {
                    if self.matches_type(value, ty)? == Truth::False {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    pub(super) fn constraint_truth(
        &self,
        constraint: &Constraint,
        bindings: &[Binding],
        claims: &[&Claim],
    ) -> Result<Truth, ProcessingError> {
        match constraint {
            Constraint::TypeConstraint { value, ty } => self.matches_type(value, *ty),
            Constraint::ScopeConstraint { variable, type_id } => {
                let Some(binding) = bindings
                    .iter()
                    .find(|binding| binding.variable.id == variable.id)
                else {
                    return Ok(Truth::Unknown);
                };
                let mut outcome = Truth::True;
                for ty in std::iter::once(*type_id).chain(variable.expected) {
                    match self.matches_type(&binding.value, ty)? {
                        Truth::False => return Ok(Truth::False),
                        Truth::Conflict => return Ok(Truth::Conflict),
                        Truth::Unknown => outcome = Truth::Unknown,
                        Truth::True => (),
                    }
                }
                Ok(outcome)
            }
            Constraint::RelationConstraint {
                value,
                role,
                target,
            } => {
                let Value::TermRef(predicate) = value else {
                    return Err(ProcessingError::UnsupportedRepresentation);
                };
                let values = claims
                    .iter()
                    .filter(|claim| claim.predicate == *predicate)
                    .flat_map(|claim| &claim.arguments)
                    .filter(|a| a.role == Some(*role))
                    .map(|a| &a.value)
                    .filter(|v| !matches!(v, Value::TypedHole(_)))
                    .collect::<Vec<_>>();
                let mut distinct = Vec::new();
                for value in &values {
                    if !matches!(value, Value::Nil) && !distinct.contains(value) {
                        distinct.push(*value);
                    }
                }
                let cardinality = self
                    .registry
                    .modules()
                    .flat_map(|m| &m.roles)
                    .find(|r| r.id == *role)
                    .ok_or(ProcessingError::InvalidReference)?
                    .cardinality;
                let (_, maximum) = cardinality.bounds();
                if maximum.is_some_and(|max| distinct.len() > max)
                    || (values.contains(&&Value::Nil) && !distinct.is_empty())
                {
                    return Ok(Truth::Conflict);
                }
                // Open-world absence stays unknown. Only an explicit Nil can
                // establish absence; representation labels never supply truth.
                Ok(if values.contains(&target) {
                    Truth::True
                } else if values.contains(&&Value::Nil) {
                    Truth::False
                } else {
                    Truth::Unknown
                })
            }
        }
    }
}
