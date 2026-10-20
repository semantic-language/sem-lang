//! One current read view over existing owner state. No persisted/flattened body.
use super::*;
use sem_lang_context::{ContextAuthority, ContextEvidence, CurrentContextRead, CurrentReadError};
use sem_lang_core::{ContextAssociation, Variable};

pub(super) struct CurrentSemanticState<'a> {
    pub claims: Vec<&'a Claim>,
    pub context: CurrentContextRead<'a>,
}

impl CurrentStateCompiler<'_> {
    /// Shared admission of exact current evidence for text and structural inputs.
    pub(super) fn scoped_claims(
        &self,
        input: &CompileContext,
    ) -> Result<Vec<&Claim>, ProcessingError> {
        if !token(&input.subject.id)
            || !token(&input.subject.class)
            || !token(&input.locale)
            || input.evidence.iter().collect::<BTreeSet<_>>().len() != input.evidence.len()
        {
            return Err(ProcessingError::InvalidInput);
        }
        ensure_bound(
            EvaluationBound::EvidenceReferences,
            input.evidence.len(),
            MAX_ITEMS,
        )?;
        let actor = Value::Map(BTreeMap::from([
            ("id".into(), Value::Text(input.subject.id.clone())),
            ("class".into(), Value::Text(input.subject.class.clone())),
        ]));
        let mut validator = validation::Validator::new(self);
        let mut claims = Vec::new();
        for reference in &input.evidence {
            let claim = self
                .state
                .claims
                .get(reference)
                .ok_or(ProcessingError::InvalidReference)?;
            if claim.id != *reference || claim.actor.as_ref() != Some(&actor) {
                return Err(ProcessingError::InvalidReference);
            }
            validator.validate_claim(claim)?;
            if (claim.validity.valid_from.is_some() || claim.validity.valid_until.is_some())
                && input.at.is_none()
            {
                return Err(ProcessingError::InvalidInput);
            }
            if input.at.is_some_and(|at| {
                claim.validity.valid_from.is_some_and(|from| at < from)
                    || claim.validity.valid_until.is_some_and(|until| at >= until)
            }) {
                continue;
            }
            claims.push(claim);
        }
        if input
            .observation_evidence
            .iter()
            .any(|id| !input.evidence.contains(id))
            || input
                .observation_evidence
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != input.observation_evidence.len()
        {
            return Err(ProcessingError::InvalidReference);
        }
        claims.sort_by_key(|claim| claim.id);
        Ok(claims)
    }
    pub(super) fn read_current<'a>(
        &'a self,
        claims: Vec<&'a Claim>,
        input: &'a CompileContext,
    ) -> Result<CurrentSemanticState<'a>, ProcessingError> {
        let context = self
            .context
            .read_current(
                &input.context_refs,
                &input.subject,
                input.context_scope,
                input.at,
            )
            .map_err(|e| match e {
                CurrentReadError::InvalidReference | CurrentReadError::InvalidSubject => {
                    ProcessingError::InvalidReference
                }
                CurrentReadError::MissingInstant => ProcessingError::InvalidInput,
                CurrentReadError::InvalidContext => ProcessingError::ContractViolation,
                CurrentReadError::LimitExceeded { limit, observed } => {
                    ProcessingError::EvaluationLimitExceeded {
                        bound: EvaluationBound::ContextReferences,
                        limit,
                        observed,
                    }
                }
            })?;
        Ok(CurrentSemanticState { claims, context })
    }

    pub(super) fn resolve_contextual(
        &self,
        associations: &[(ContextAssociation, String)],
        state: &CurrentSemanticState<'_>,
        input: &CompileContext,
        result: &mut SemanticResolution,
    ) -> Result<(), ProcessingError> {
        for (association, variable) in associations {
            let applicable = association
                .validity
                .applicable(input.at)
                .ok_or(ProcessingError::InvalidInput)?
                && association.scope.permits(input.context_scope);
            let mut evidence = Vec::new();
            if applicable {
                for claim in &state.claims {
                    if claim.predicate != association.semantic_ref {
                        continue;
                    }
                    for argument in &claim.arguments {
                        if argument.role != Some(association.role) {
                            continue;
                        }
                        let Value::Text(value) = &argument.value else {
                            // ContextStore's SemanticValue is text; do not infer a
                            // semantic representation from another primitive type.
                            return Err(ProcessingError::UnsupportedRepresentation);
                        };
                        evidence.push(ContextEvidence {
                            value,
                            authority: if input.observation_evidence.contains(&claim.id) {
                                ContextAuthority::Explicit
                            } else {
                                ContextAuthority::AcceptedAssertion
                            },
                        });
                    }
                }
            }
            let selected = applicable.then(|| state.context.select(association.kind, &evidence));
            let values = selected
                .as_ref()
                .map(|s| s.values.as_slice())
                .unwrap_or(&[]);
            if values.len() == 1 {
                let value = Value::Text(values[0].into());
                if let Some(existing) = result.bindings.iter().find(|v| v.variable.id == *variable)
                {
                    if existing.value == value {
                        continue;
                    }
                    let alternatives = vec![existing.value.clone(), value];
                    result.bindings.retain(|v| v.variable.id != *variable);
                    result.issues.push(ResolutionIssue {
                        reason: Reason::ConflictingEvidence,
                        variable: Some(variable.clone()),
                        candidates: alternatives,
                        constraint: None,
                    });
                } else {
                    result.bindings.push(Binding {
                        variable: Variable {
                            id: variable.clone(),
                            expected: None,
                        },
                        value,
                    });
                }
            } else {
                result.issues.push(ResolutionIssue {
                    reason: if values.is_empty() {
                        Reason::MissingContext
                    } else {
                        Reason::ConflictingEvidence
                    },
                    variable: Some(variable.clone()),
                    candidates: values.iter().map(|s| Value::Text((*s).into())).collect(),
                    constraint: None,
                });
            }
        }
        // A later association must not resurrect a variable with an unresolved
        // or contradictory earlier requirement. No weaker fallback binding.
        result.bindings.retain(|binding| {
            !result
                .issues
                .iter()
                .any(|issue| issue.variable.as_deref() == Some(binding.variable.id.as_str()))
        });
        result
            .bindings
            .sort_by(|a, b| a.variable.id.cmp(&b.variable.id));
        result.issues.sort_by(|a, b| a.variable.cmp(&b.variable));
        Ok(())
    }
}
