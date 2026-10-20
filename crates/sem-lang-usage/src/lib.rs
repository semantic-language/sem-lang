//! Append-oriented memory for language use and interpretation evidence.

use sem_lang_core::Expr;
use sem_lang_linguistics::{EvidenceKind, EvidenceRef, EvidenceSet};

/// Process-local evidence index for an ephemeral induction input batch.
/// Not a canonical publication ID, durable receipt, or commit revision.
pub type ObservationId = String;
pub type Timestamp = i64;

#[derive(Debug, Clone)]
pub struct SurfaceInput {
    pub text: String,
    pub locale: String,
}

#[derive(Debug, Clone, Default)]
pub struct LinguisticContext {
    pub reference_date: Option<String>,
    pub timezone: Option<String>,
    pub references: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum SourceRef {
    User,
    TestFixture(String),
    External(String),
}

#[derive(Debug, Clone)]
pub struct UsageObservation {
    pub id: ObservationId,
    pub surface: SurfaceInput,
    pub context: LinguisticContext,
    pub interpretation: Option<Expr>,
    pub language_revision: String,
    pub source: SourceRef,
    pub observed_at: Timestamp,
}

#[derive(Debug, Clone)]
pub struct InterpretationCandidate {
    pub expression: Expr,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InterpretationStatus {
    Unresolved,
    Selected,
    Corrected,
    Rejected,
}

#[derive(Debug, Clone)]
pub struct InterpretationRecord {
    pub observation: ObservationId,
    pub candidates: Vec<InterpretationCandidate>,
    pub selected: Option<Expr>,
    pub evidence: EvidenceSet,
    pub status: InterpretationStatus,
}

#[derive(Debug, Clone)]
pub struct ClarificationRecord {
    pub observation: ObservationId,
    pub question: String,
    pub answer: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfirmationRecord {
    pub observation: ObservationId,
    pub confirmed: bool,
}

#[derive(Debug, Clone)]
pub struct CorrectionRecord {
    pub observation: ObservationId,
    pub correction: String,
}

#[derive(Debug, Clone)]
pub struct ExecutionEvidence {
    pub observation: ObservationId,
    pub success: bool,
    pub result: String,
}

#[derive(Debug, Clone, Default)]
pub struct UsageMemory {
    observations: Vec<UsageObservation>,
    interpretations: Vec<InterpretationRecord>,
    pub clarifications: Vec<ClarificationRecord>,
    pub confirmations: Vec<ConfirmationRecord>,
    pub corrections: Vec<CorrectionRecord>,
    pub execution: Vec<ExecutionEvidence>,
    pub evidence: Vec<EvidenceRef>,
    next_id: u64,
}

pub type Observation = UsageObservation;
pub type Interpretation = InterpretationRecord;
pub type Evidence = ExecutionEvidence;

impl UsageMemory {
    pub fn record_observation(
        &mut self,
        surface: SurfaceInput,
        context: LinguisticContext,
        language_revision: impl Into<String>,
        source: SourceRef,
        observed_at: Timestamp,
    ) -> ObservationId {
        self.next_id += 1;
        let id = format!("obs-{:016x}", self.next_id);
        self.observations.push(UsageObservation {
            id: id.clone(),
            surface,
            context,
            interpretation: None,
            language_revision: language_revision.into(),
            source,
            observed_at,
        });
        id
    }

    pub fn observations(&self) -> &[UsageObservation] {
        &self.observations
    }

    pub fn interpretations(&self) -> &[InterpretationRecord] {
        &self.interpretations
    }

    pub fn record_interpretation(&mut self, record: InterpretationRecord) {
        if let Some(observation) = self
            .observations
            .iter_mut()
            .find(|item| item.id == record.observation)
        {
            observation.interpretation = record.selected.clone();
        }
        self.interpretations.push(record);
    }

    pub fn add_evidence(
        &mut self,
        observation: impl Into<String>,
        kind: EvidenceKind,
        detail: impl Into<String>,
    ) {
        let reference = observation.into();
        self.evidence.push(EvidenceRef {
            kind,
            reference,
            detail: detail.into(),
        });
    }

    pub fn record_execution(&mut self, evidence: ExecutionEvidence) {
        self.add_evidence(
            evidence.observation.clone(),
            if evidence.success {
                EvidenceKind::SuccessfulExecution
            } else {
                EvidenceKind::FailedExecution
            },
            evidence.result.clone(),
        );
        self.execution.push(evidence);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observations_are_append_only_and_distinct_from_interpretations() {
        let mut memory = UsageMemory::default();
        let id = memory.record_observation(
            SurfaceInput {
                text: "押さえる".to_string(),
                locale: "ja-JP".to_string(),
            },
            LinguisticContext::default(),
            "r1",
            SourceRef::User,
            1,
        );
        assert_eq!(memory.observations().len(), 1);
        assert!(memory.interpretations().is_empty());
        memory.add_evidence(id, EvidenceKind::PositiveExample, "confirmed example");
        assert_eq!(memory.evidence.len(), 1);
    }
}
