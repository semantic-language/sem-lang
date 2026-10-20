//! Facade over the single generic frontend compiler. No second semantic evaluator.
use crate::{CompileContext, Engine, SurfaceInput};
use sem_lang_frontend::current::CurrentStateCompiler;
pub use sem_lang_frontend::current::{
    ClaimConstraintResult, ConstraintStatus, EvaluationBound, Observation, ObservationScope,
    ProcessingError, ProcessingOutcome, Reason, ResolutionIssue, SemanticResolution,
};
pub use sem_lang_frontend::current::{source_structure, structural};

impl Engine {
    /// Prepare a resolved packet only after an explicit caller decision. This
    /// does not publish it; restoration::apply remains the admission/commit owner.
    pub fn structural_packet(
        &self,
        input: &structural::StructuralInput,
        packet_id: sem_lang_core::PacketId,
        claim_id: sem_lang_core::NodeId,
        decision: &str,
        observed_at: i64,
    ) -> Result<
        (
            structural::StructuralResolution,
            Option<crate::SemanticPacket>,
        ),
        ProcessingError,
    > {
        use sem_lang_core::{
            Apply, Argument, Claim, ContextDelta, Expr, Provenance, Resolution, Validity, Value,
        };
        use sem_lang_linguistics::SemanticExpr;
        use std::collections::BTreeMap;
        if decision.is_empty()
            || decision.len() > 512
            || decision.trim() != decision
            || decision.contains('\0')
        {
            return Err(ProcessingError::InvalidInput);
        }
        let result = self.resolve_structure(input)?;
        if result.reason.is_some() {
            return Ok((result, None));
        }
        let candidate = result
            .candidates
            .first()
            .ok_or(ProcessingError::ContractViolation)?;
        let expressions = match &candidate.expression {
            SemanticExpr::Conjunction(values) => values.as_slice(),
            expression => std::slice::from_ref(expression),
        };
        let (predicate, arguments) = expressions
            .iter()
            .find_map(|e| {
                if let SemanticExpr::Apply {
                    predicate,
                    arguments,
                } = e
                {
                    Some((*predicate, arguments))
                } else {
                    None
                }
            })
            .ok_or(ProcessingError::UnsupportedRepresentation)?;
        let arguments = arguments
            .iter()
            .map(|(role, expression)| {
                let value = match expression.as_ref() {
                    SemanticExpr::Literal(v) => Value::Text(v.clone()),
                    SemanticExpr::Variable(name) => candidate
                        .bindings
                        .iter()
                        .find(|b| b.variable.id == *name)
                        .map(|b| b.value.clone())
                        .ok_or(ProcessingError::InvalidReference)?,
                    _ => return Err(ProcessingError::UnsupportedRepresentation),
                };
                Ok(Argument {
                    role: Some(*role),
                    value,
                })
            })
            .collect::<Result<Vec<_>, ProcessingError>>()?;
        let packet = crate::SemanticPacket {
            id: packet_id,
            conversation_id: None,
            sender: None,
            recipient: None,
            cause: Some(decision.into()),
            correlation: None,
            act: None,
            content: Expr::Apply(Apply {
                operator: predicate,
                arguments: arguments.clone(),
            }),
            context_delta: Some(ContextDelta {
                added_claims: vec![Claim {
                    id: claim_id,
                    actor: Some(Value::Map(BTreeMap::from([
                        ("id".into(), Value::Text(input.context.subject.id.clone())),
                        (
                            "class".into(),
                            Value::Text(input.context.subject.class.clone()),
                        ),
                    ]))),
                    predicate,
                    arguments,
                    constraints: vec![],
                    validity: Validity {
                        valid_from: input.context.at,
                        valid_until: None,
                    },
                }],
                removed_claims: vec![],
                annotations: BTreeMap::from([
                    ("structural-input".into(), result.input_digest.clone()),
                    ("structural-meaning".into(), result.meaning_digest.clone()),
                ]),
            }),
            vocabulary_manifest: None,
            resolution: Resolution::Resolved,
            pragmatics: vec![],
            provenance: Some(Provenance {
                source: Some(input.reference.clone()),
                confidence: None,
                observed_at,
            }),
        };
        Ok((result, Some(packet)))
    }
    pub fn resolve_structure(
        &self,
        input: &structural::StructuralInput,
    ) -> Result<structural::StructuralResolution, ProcessingError> {
        self.current_compiler()?.resolve_structure(input)
    }
    pub fn current_compiler(&self) -> Result<CurrentStateCompiler<'_>, ProcessingError> {
        if self.restoration.is_none() {
            return Err(ProcessingError::InvalidReference);
        }
        Ok(CurrentStateCompiler::new(
            &self.compiled_language,
            &self.runtime.state,
            &self.context,
        ))
    }

    /// Scoped callers enter the same generic frontend; no cloned runtime or
    /// definition-only packet compiler is kept as a fallback.
    pub fn process_observation(
        &self,
        observation: &Observation,
        scope: &ObservationScope,
    ) -> Result<ProcessingOutcome, ProcessingError> {
        let compiler = self.current_compiler()?;
        let context = CompileContext {
            locale: observation.locale.clone(),
            subject: scope.subject.clone(),
            evidence: scope.evidence.clone(),
            at: scope.at,
            semantic_revision: compiler.semantic_revision()?,
            context_refs: vec![],
            context_scope: sem_lang_core::ContextScope::CurrentRequest,
            observation_evidence: vec![],
        };
        compiler.compile(
            &SurfaceInput::new(&observation.reference, &observation.text),
            &context,
        )
    }

    pub fn compile_surface(
        &self,
        input: &SurfaceInput,
        context: &CompileContext,
    ) -> Result<ProcessingOutcome, ProcessingError> {
        self.current_compiler()?.compile(input, context)
    }
}
