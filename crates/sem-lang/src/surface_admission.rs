//! Explicit admission preparation for an already interpreted surface observation.
//! This shares current compilation and the existing packet reducer/commit path;
//! it neither dispatches inference nor publishes or retains observations.
use crate::{
    CompileContext, Engine, SurfaceInput,
    current::{ProcessingError, ProcessingOutcome},
};
use sem_lang_core::{
    Apply, Argument, Claim, ContextDelta, Expr, NodeId, PacketId, Provenance, Resolution,
    SemanticPacket, Validity, Value,
};
use sem_lang_linguistics::SemanticExpr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceAdmission {
    pub input: SurfaceInput,
    pub context: CompileContext,
    pub packet_id: PacketId,
    pub claim_id: NodeId,
    pub decision_ref: String,
    pub observed_at: i64,
    /// Owner-neutral exact evidence identities, not semantic assertions.
    pub evidence_refs: Vec<crate::semantic_execution::ExactRef>,
}

type Application<'a> = (
    sem_lang_core::TermId,
    &'a [(sem_lang_core::RoleId, Box<SemanticExpr>)],
);
fn application(expr: &SemanticExpr, depth: usize) -> Result<Application<'_>, ProcessingError> {
    if depth > 32 {
        return Err(ProcessingError::ContractViolation);
    }
    match expr {
        SemanticExpr::Defined { expansion, .. } => application(expansion, depth + 1),
        SemanticExpr::Apply {
            predicate,
            arguments,
        } => Ok((*predicate, arguments)),
        _ => Err(ProcessingError::UnsupportedRepresentation),
    }
}

impl Engine {
    pub fn surface_packet(
        &self,
        request: &SurfaceAdmission,
    ) -> Result<(ProcessingOutcome, Option<SemanticPacket>), ProcessingError> {
        if request.decision_ref.is_empty()
            || request.decision_ref.len() > 512
            || request.decision_ref.trim() != request.decision_ref
            || request.decision_ref.contains('\0')
            || request.evidence_refs.len() > 16
            || request.evidence_refs.iter().any(|r| {
                [&r.owner, &r.id, &r.revision]
                    .iter()
                    .any(|s| s.is_empty() || s.len() > 512 || s.contains('\0') || s.trim() != **s)
            })
        {
            return Err(ProcessingError::InvalidInput);
        }
        let outcome = self.compile_surface(&request.input, &request.context)?;
        let ProcessingOutcome::Resolved(resolution) = &outcome else {
            return Ok((outcome, None));
        };
        let (predicate, values) = application(
            resolution
                .expression
                .as_ref()
                .ok_or(ProcessingError::UnsupportedRepresentation)?,
            0,
        )?;
        let arguments = values
            .iter()
            .map(|(role, value)| {
                let value = match value.as_ref() {
                    SemanticExpr::Literal(text) => Value::Text(text.clone()),
                    SemanticExpr::Variable(name) => resolution
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
        let evidence = serde_json::to_string(&request.evidence_refs)
            .map_err(|_| ProcessingError::InvalidInput)?;
        let packet = SemanticPacket {
            id: request.packet_id,
            conversation_id: None,
            sender: None,
            recipient: None,
            cause: Some(request.decision_ref.clone()),
            correlation: Some(request.input.reference.clone()),
            act: None,
            content: Expr::Apply(Apply {
                operator: predicate,
                arguments: arguments.clone(),
            }),
            context_delta: Some(ContextDelta {
                added_claims: vec![Claim {
                    id: request.claim_id,
                    actor: Some(Value::Map(BTreeMap::from([
                        ("id".into(), Value::Text(request.context.subject.id.clone())),
                        (
                            "class".into(),
                            Value::Text(request.context.subject.class.clone()),
                        ),
                    ]))),
                    predicate,
                    arguments,
                    constraints: vec![],
                    validity: Validity {
                        valid_from: request.context.at,
                        valid_until: None,
                    },
                }],
                removed_claims: vec![],
                annotations: BTreeMap::from([
                    ("observation-resolution".into(), resolution.identity.clone()),
                    ("observation-evidence".into(), evidence),
                ]),
            }),
            vocabulary_manifest: None,
            resolution: Resolution::Resolved,
            pragmatics: vec![],
            provenance: Some(Provenance {
                source: Some(request.input.reference.clone()),
                confidence: None,
                observed_at: request.observed_at,
            }),
        };
        Ok((outcome, Some(packet)))
    }
}
