//! bounded current evaluation and formal selection validation.
//! No provider, authorization, model, history, candidate store or implicit write.
use crate::{CompileContext, SurfaceInput};
use sem_lang_context::ContextEngine;
use sem_lang_context::ContextSubject;
use sem_lang_core::RuntimeState;
use sem_lang_core::{Binding, Claim, Constraint, Digest, NodeId, TermId, Value};
use sem_lang_language::CompiledLanguage;
use sem_lang_language::SemanticCompileError;
use sem_lang_linguistics::SemanticExpr;
use sem_lang_vocab::VocabularyRegistry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ITEMS: usize = 64;
mod confirmation;
mod constraints;
mod context;
mod materialization;
mod relations;
pub mod source_structure;
pub mod structural;
mod validation;
pub use confirmation::{
    ConfirmationError, SelectionRequest, SemanticSelection, ValidatedSelection,
};
use relations::Pattern;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub reference: String,
    pub text: String,
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationScope {
    pub subject: ContextSubject,
    /// Exact current Claim references, not supplied evidence strings or packets.
    pub evidence: Vec<NodeId>,
    /// Required for dated evidence. The compiler never reads a wall clock.
    pub at: Option<i64>,
}

impl ObservationScope {
    /// Lossless representation in the existing Claim.actor Value algebra.
    /// Both identity and class must match; no display-name or role inference.
    pub fn actor(&self) -> Value {
        Value::Map(BTreeMap::from([
            ("id".into(), Value::Text(self.subject.id.clone())),
            ("class".into(), Value::Text(self.subject.class.clone())),
        ]))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    MissingContext,
    Ambiguous,
    UnknownExpression,
    ConflictingEvidence,
    CardinalityViolation,
    CircularDependency,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionIssue {
    pub reason: Reason,
    pub variable: Option<String>,
    pub candidates: Vec<Value>,
    /// Exact unevaluated/unsatisfied constraint, not a reconstructed message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<sem_lang_linguistics::SemanticConstraint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticResolution {
    pub semantic_revision: String,
    pub context_refs: Vec<(
        sem_lang_context::ContextReference,
        sem_lang_context::ContextStatus,
    )>,
    pub observation_ref: String,
    pub subject: ContextSubject,
    pub concept: Option<TermId>,
    /// The adopted expression and its explicit variable bindings form one result.
    pub expression: Option<SemanticExpr>,
    pub bindings: Vec<Binding>,
    pub issues: Vec<ResolutionIssue>,
    pub evidence_refs: Vec<NodeId>,
    pub claim_constraints: Vec<ClaimConstraintResult>,
    pub multiplicities: Vec<MultiplicityResult>,
    /// Correlated relation rows; many-valued summaries must not erase pairing.
    pub relations: Vec<RelationResult>,
    /// Input/result digest, not a mutation receipt, permission or scheduler key.
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationResult {
    pub variables: Vec<String>,
    pub rows: Vec<Vec<Binding>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultiplicityResult {
    pub variable: String,
    pub minimum: usize,
    pub maximum: Option<usize>,
    /// A lower bound while status is Unresolved, otherwise the selected count.
    pub observed: usize,
    pub status: ConstraintStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintStatus {
    Satisfied,
    Unsatisfied,
    Unresolved,
    Conflicting,
    Circular,
}

/// Unsupported/invalid representations remain ProcessingError, not uncertainty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimConstraintResult {
    pub claim_ref: NodeId,
    pub constraint: sem_lang_linguistics::SemanticConstraint,
    pub status: ConstraintStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "result", deny_unknown_fields)]
pub enum ProcessingOutcome {
    Resolved(SemanticResolution),
    NeedsResolution(SemanticResolution),
}

/// Runtime Waiting/NoChange and authorization remain the caller's responsibility.
/// These failures must not be sent to a semantic uncertainty resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum ProcessingError {
    #[error("invalid observation input")]
    InvalidInput,
    #[error("invalid current state reference")]
    InvalidReference,
    #[error("invalid adopted semantic contract")]
    ContractViolation,
    #[error("stale semantic revision")]
    StaleRevision,
    #[error("evaluation limit exceeded: {bound:?} ({observed} > {limit})")]
    EvaluationLimitExceeded {
        bound: EvaluationBound,
        limit: usize,
        observed: usize,
    },
    #[error("unsupported semantic representation")]
    UnsupportedRepresentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvaluationBound {
    Construction(sem_lang_linguistics::ConstructionBound),
    MaterializationBytes,
    EvidenceReferences,
    ContextReferences,
    Alternatives,
    PairWork,
    VariableOccurrences,
    ExpressionDepth,
    TypeTraversal,
    Constraints,
    Arguments,
    Patterns,
    ValueItems,
    ValueWork,
}

impl From<sem_lang_language::TypeEvaluationError> for ProcessingError {
    fn from(error: sem_lang_language::TypeEvaluationError) -> Self {
        match error {
            sem_lang_language::TypeEvaluationError::InvalidReference => Self::InvalidReference,
            sem_lang_language::TypeEvaluationError::LimitExceeded { limit, observed } => {
                Self::EvaluationLimitExceeded {
                    bound: EvaluationBound::TypeTraversal,
                    limit,
                    observed,
                }
            }
        }
    }
}

fn ensure_bound(
    bound: EvaluationBound,
    observed: usize,
    limit: usize,
) -> Result<(), ProcessingError> {
    if observed > limit {
        Err(ProcessingError::EvaluationLimitExceeded {
            bound,
            limit,
            observed,
        })
    } else {
        Ok(())
    }
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.trim() == value && !value.contains('\0')
}

/// Borrowed current state, not a cloned memory, repository or execution host.
pub struct CurrentStateCompiler<'a> {
    registry: &'a VocabularyRegistry,
    compiled_language: &'a CompiledLanguage,
    state: &'a RuntimeState,
    context: &'a ContextEngine,
}

impl<'a> CurrentStateCompiler<'a> {
    pub fn new(
        language: &'a CompiledLanguage,
        state: &'a RuntimeState,
        context: &'a ContextEngine,
    ) -> Self {
        Self {
            registry: language.registry(),
            compiled_language: language,
            state,
            context,
        }
    }

    /// Semantic content only: receipts, packet history and annotations are not inputs.
    pub fn semantic_revision(&self) -> Result<String, ProcessingError> {
        let memory = self
            .context
            .memory_revision()
            .map_err(|_| ProcessingError::ContractViolation)?;
        let digest = Digest::from_serializable(
            1,
            &(
                self.compiled_language.digest,
                &self.state.graph,
                &self.state.claims,
                &self.state.terms,
                &self.state.types,
                &self.state.roles,
                memory,
            ),
        )
        .map_err(|_| ProcessingError::ContractViolation)?;
        Ok(digest.to_hex())
    }

    /// One generic frontend entry. The locale, subject, evidence, instant and
    /// inspected revision are mandatory caller context, never implicit defaults.
    pub fn compile(
        &self,
        input: &SurfaceInput,
        context: &CompileContext,
    ) -> Result<ProcessingOutcome, ProcessingError> {
        if context.semantic_revision.len() != 64
            || !context
                .semantic_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(ProcessingError::InvalidInput);
        }
        if self.semantic_revision()? != context.semantic_revision {
            return Err(ProcessingError::StaleRevision);
        }
        self.evaluate(
            &Observation {
                reference: input.reference.clone(),
                text: input.text.clone(),
                locale: context.locale.clone(),
            },
            &ObservationScope {
                subject: context.subject.clone(),
                evidence: context.evidence.clone(),
                at: context.at,
            },
            context,
        )
    }

    /// Uses adopted definitions and explicitly scoped current assertions only.
    /// Does not mutate memory, synthesize missing facts, or execute capabilities.
    fn evaluate(
        &self,
        observation: &Observation,
        scope: &ObservationScope,
        context: &CompileContext,
    ) -> Result<ProcessingOutcome, ProcessingError> {
        if !token(&observation.reference)
            || !token(&observation.locale)
            || observation.text.trim().is_empty()
            || observation.text.len() > 16_384
            || observation.text.contains('\0')
            || !token(&scope.subject.id)
            || !token(&scope.subject.class)
            || scope.evidence.iter().collect::<BTreeSet<_>>().len() != scope.evidence.len()
        {
            return Err(ProcessingError::InvalidInput);
        }
        let claims = self.scoped_claims(context)?;
        let mut validator = validation::Validator::new(self);
        let (applicable_claims, claim_constraints) = self.applicable_claims(&claims)?;
        let evidence_refs = claims.iter().map(|claim| claim.id).collect();
        let read = self.read_current(applicable_claims, context)?;
        let claims = &read.claims;
        let mut result = SemanticResolution {
            semantic_revision: context.semantic_revision.clone(),
            context_refs: read
                .context
                .entries
                .iter()
                .map(|entry| (entry.reference.clone(), entry.status))
                .collect(),
            observation_ref: observation.reference.clone(),
            subject: scope.subject.clone(),
            concept: None,
            expression: None,
            bindings: vec![],
            issues: vec![],
            evidence_refs,
            claim_constraints,
            multiplicities: vec![],
            relations: vec![],
            identity: String::new(),
        };
        result.context_refs.sort_by(|a, b| a.0.cmp(&b.0));
        for condition in &result.claim_constraints {
            if matches!(
                condition.status,
                ConstraintStatus::Unresolved
                    | ConstraintStatus::Conflicting
                    | ConstraintStatus::Circular
            ) {
                result.issues.push(ResolutionIssue {
                    reason: match condition.status {
                        ConstraintStatus::Conflicting => Reason::ConflictingEvidence,
                        ConstraintStatus::Circular => Reason::CircularDependency,
                        _ => Reason::MissingContext,
                    },
                    variable: None,
                    candidates: vec![],
                    constraint: Some(condition.constraint.clone()),
                });
            }
        }
        match self.compiled_language.compile_current_semantics(
            &observation.text,
            &observation.locale,
            scope.at,
            self.state,
        ) {
            Ok(compiled) => {
                result.concept = Some(compiled.concept);
                for condition in &compiled.conditions {
                    result.issues.push(ResolutionIssue {
                        reason: Reason::MissingContext,
                        variable: None,
                        candidates: vec![],
                        constraint: Some(
                            condition
                                .clone()
                                .try_into()
                                .map_err(|_| ProcessingError::ContractViolation)?,
                        ),
                    });
                }
                let mut patterns = Vec::new();
                let mut constraints = Vec::new();
                let mut associations = Vec::new();
                self.collect_patterns(
                    &compiled.normalized.expr,
                    &mut patterns,
                    &mut result.issues,
                    &mut constraints,
                    &mut associations,
                    0,
                )?;
                for constraint in &constraints {
                    validator.validate_constraint(constraint)?;
                }
                ensure_bound(
                    EvaluationBound::VariableOccurrences,
                    patterns.iter().map(Pattern::occurrences).sum(),
                    MAX_ITEMS,
                )?;
                // A substring is not a parse of the whole observation. Until a
                // declared construction covers the rest, retain that residual.
                if !compiled.complete_surface {
                    result.issues.push(ResolutionIssue {
                        reason: Reason::UnknownExpression,
                        variable: None,
                        candidates: vec![],
                        constraint: None,
                    });
                }
                result.expression = Some(compiled.normalized.expr);
                self.resolve_contextual(&associations, &read, context, &mut result)?;
                relations::resolve(
                    &patterns,
                    claims,
                    &self.state.graph.nodes,
                    &mut result,
                    |name, value| self.allowed_value(name, value, &constraints),
                )?;
                let scoped_claims = relations::scoped_claims(
                    &patterns,
                    claims,
                    &self.state.graph.nodes,
                    &result,
                    |name, value| self.allowed_value(name, value, &constraints),
                )?;
                for constraint in constraints {
                    let truth =
                        self.resolved_constraint_truth(&constraint, &result, &scoped_claims)?;
                    if truth != constraints::Truth::True {
                        result.issues.push(ResolutionIssue {
                            reason: if matches!(
                                truth,
                                constraints::Truth::False | constraints::Truth::Conflict
                            ) {
                                Reason::ConflictingEvidence
                            } else {
                                Reason::MissingContext
                            },
                            variable: match &constraint {
                                Constraint::ScopeConstraint { variable, .. } => {
                                    Some(variable.id.clone())
                                }
                                _ => None,
                            },
                            candidates: vec![],
                            constraint: Some(
                                constraint
                                    .try_into()
                                    .map_err(|_| ProcessingError::ContractViolation)?,
                            ),
                        });
                    }
                }
            }
            Err(SemanticCompileError::UnknownSurface(_)) => result.issues.push(ResolutionIssue {
                reason: Reason::UnknownExpression,
                variable: None,
                candidates: vec![],
                constraint: None,
            }),
            Err(SemanticCompileError::AmbiguousSurface { candidates, .. }) => {
                result.issues.push(ResolutionIssue {
                    reason: Reason::Ambiguous,
                    variable: None,
                    candidates: candidates.into_iter().map(Value::TermRef).collect(),
                    constraint: None,
                })
            }
            Err(SemanticCompileError::MissingEvaluationInstant) => {
                return Err(ProcessingError::InvalidInput);
            }
            Err(SemanticCompileError::Construction(error)) => {
                return Err(match error {
                    sem_lang_linguistics::ConstructionError::Invalid => {
                        ProcessingError::ContractViolation
                    }
                    sem_lang_linguistics::ConstructionError::LimitExceeded {
                        bound,
                        limit,
                        observed,
                    } => ProcessingError::EvaluationLimitExceeded {
                        bound: EvaluationBound::Construction(bound),
                        limit,
                        observed,
                    },
                });
            }
            Err(SemanticCompileError::TypeEvaluation(error)) => return Err(error.into()),
            Err(_) => return Err(ProcessingError::ContractViolation),
        }
        let digest = Digest::from_serializable(
            1,
            &(
                self.compiled_language.digest,
                observation,
                scope.subject.clone(),
                scope.at,
                context.context_scope,
                context.evidence.iter().collect::<BTreeSet<_>>(),
                context.observation_evidence.iter().collect::<BTreeSet<_>>(),
                &result,
            ),
        )
        .map_err(|_| ProcessingError::ContractViolation)?;
        result.identity = digest.to_hex();
        Ok(if result.issues.is_empty() {
            ProcessingOutcome::Resolved(result)
        } else {
            ProcessingOutcome::NeedsResolution(result)
        })
    }

    fn known_term(&self, id: TermId) -> bool {
        self.registry
            .modules()
            .any(|module| module.terms.iter().any(|term| term.id == id))
    }

    fn relation_pattern(
        &self,
        expression: &SemanticExpr,
        depth: usize,
    ) -> Result<Pattern, ProcessingError> {
        ensure_bound(EvaluationBound::ExpressionDepth, depth, 32)?;
        if let SemanticExpr::Defined { expansion, .. } = expression {
            return self.relation_pattern(expansion, depth + 1);
        }
        let SemanticExpr::Apply {
            predicate,
            arguments,
        } = expression
        else {
            return Err(ProcessingError::UnsupportedRepresentation);
        };
        ensure_bound(EvaluationBound::Arguments, arguments.len(), MAX_ITEMS)?;
        let mut pattern = Pattern {
            predicate: *predicate,
            variables: vec![],
            guards: vec![],
            nested: vec![],
            cardinalities: BTreeMap::new(),
        };
        for (role, expression) in arguments {
            let role = *role;
            if !self
                .registry
                .modules()
                .any(|module| module.roles.iter().any(|item| item.id == role))
            {
                return Err(ProcessingError::InvalidReference);
            }
            match expression.as_ref() {
                SemanticExpr::Variable(variable) => {
                    if !token(variable) {
                        return Err(ProcessingError::ContractViolation);
                    }
                    let cardinality = self
                        .registry
                        .modules()
                        .flat_map(|m| &m.roles)
                        .find(|r| r.id == role)
                        .ok_or(ProcessingError::InvalidReference)?
                        .cardinality;
                    pattern
                        .cardinalities
                        .entry(variable.clone())
                        .or_default()
                        .push(cardinality);
                    pattern.variables.push((role, variable.clone()));
                }
                SemanticExpr::Literal(value) => {
                    pattern.guards.push((role, Value::Text(value.clone())))
                }
                SemanticExpr::Concept(value) | SemanticExpr::Primitive(value) => {
                    pattern.guards.push((role, Value::TermRef(*value)))
                }
                SemanticExpr::Apply { .. } | SemanticExpr::Defined { .. } => {
                    pattern
                        .nested
                        .push((role, self.relation_pattern(expression, depth + 1)?));
                }
                _ => return Err(ProcessingError::UnsupportedRepresentation),
            }
        }
        ensure_bound(EvaluationBound::Patterns, pattern.count(), MAX_ITEMS)?;
        Ok(pattern)
    }

    fn collect_patterns(
        &self,
        expression: &SemanticExpr,
        patterns: &mut Vec<Pattern>,
        issues: &mut Vec<ResolutionIssue>,
        constraints: &mut Vec<Constraint>,
        associations: &mut Vec<(sem_lang_core::ContextAssociation, String)>,
        depth: usize,
    ) -> Result<(), ProcessingError> {
        ensure_bound(EvaluationBound::ExpressionDepth, depth, 32)?;
        match expression {
            SemanticExpr::Defined { expansion, .. } => self.collect_patterns(
                expansion,
                patterns,
                issues,
                constraints,
                associations,
                depth + 1,
            )?,
            SemanticExpr::Conjunction(expressions) => {
                for expression in expressions {
                    self.collect_patterns(
                        expression,
                        patterns,
                        issues,
                        constraints,
                        associations,
                        depth + 1,
                    )?;
                }
            }
            SemanticExpr::Apply { .. } => {
                let pattern = self.relation_pattern(expression, depth)?;
                patterns.push(pattern);
            }
            SemanticExpr::Primitive(_) | SemanticExpr::Concept(_) | SemanticExpr::Literal(_) => (),
            SemanticExpr::Residual(residual) => issues.push(ResolutionIssue {
                reason: Reason::UnknownExpression,
                variable: None,
                candidates: vec![Value::TermRef(residual.source_term)],
                constraint: None,
            }),
            SemanticExpr::Variable(variable) => issues.push(ResolutionIssue {
                reason: Reason::MissingContext,
                variable: Some(variable.clone()),
                candidates: vec![],
                constraint: None,
            }),
            SemanticExpr::Constraint(value) => {
                ensure_bound(
                    EvaluationBound::Constraints,
                    constraints.len() + 1,
                    MAX_ITEMS,
                )?;
                constraints.push(value.value().clone());
            }
            SemanticExpr::Contextual {
                association,
                variable,
            } => {
                if !self.known_term(association.semantic_ref)
                    || !self
                        .registry
                        .modules()
                        .any(|m| m.roles.iter().any(|r| r.id == association.role))
                    || !token(variable)
                    || !association.validity.valid()
                {
                    return Err(ProcessingError::InvalidReference);
                }
                ensure_bound(
                    EvaluationBound::ContextReferences,
                    associations.len() + 1,
                    MAX_ITEMS,
                )?;
                associations.push((association.clone(), variable.clone()));
            }
        }
        ensure_bound(
            EvaluationBound::Patterns,
            patterns.iter().map(Pattern::count).sum(),
            MAX_ITEMS,
        )?;
        Ok(())
    }
}
