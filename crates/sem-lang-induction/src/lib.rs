//! Deterministic induction from Usage Memory into reviewable candidates.

use sem_lang_core::{Constraint, Expr, TermId, TypeId, Value};
use sem_lang_linguistics::{
    EvidenceKind, EvidenceRef, EvidenceSet, FormPattern, KnowledgeId, KnowledgeOrigin,
    LinguisticKnowledge, LinguisticKnowledgeEntry, SemanticContribution, Validity,
};
use sem_lang_usage::{ObservationId, UsageMemory};
use std::collections::BTreeMap;

pub use sem_lang_context::{
    ConservativeContextInducer, ContextDefaultCandidate, ContextInducer, ContextObservation,
};

#[derive(Debug, Clone)]
pub struct LinguisticPattern {
    pub surface: String,
    pub locale: String,
    pub construction: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KnowledgeCandidate {
    pub id: KnowledgeId,
    pub pattern: LinguisticPattern,
    pub semantic_binding: SemanticContribution,
    pub constraints: Vec<Constraint>,
    pub supporting_evidence: Vec<ObservationId>,
    pub counter_evidence: Vec<ObservationId>,
    pub confidence: f32,
}

pub type DefinitionCandidate = KnowledgeCandidate;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ConsolidationError {
    #[error("candidate has no supporting evidence")]
    NoEvidence,
    #[error("candidate confidence is below the consolidation threshold")]
    LowConfidence,
    #[error("knowledge validation failed: {0}")]
    Knowledge(String),
}

pub struct KnowledgeInducer;

pub type DefinitionInducer = KnowledgeInducer;

impl KnowledgeInducer {
    pub fn induce(memory: &UsageMemory) -> Vec<KnowledgeCandidate> {
        let mut grouped: BTreeMap<(String, String), KnowledgeCandidate> = BTreeMap::new();
        for observation in memory.observations() {
            let Some(expression) = observation.interpretation.as_ref() else {
                continue;
            };
            let Some(concept) = expression_term(expression) else {
                continue;
            };
            let key = (
                observation.surface.locale.clone(),
                observation.surface.text.clone(),
            );
            let candidate = grouped
                .entry(key.clone())
                .or_insert_with(|| KnowledgeCandidate {
                    id: KnowledgeId::from_label(&format!("{}:{}", key.0, key.1)),
                    pattern: LinguisticPattern {
                        surface: key.1.clone(),
                        locale: key.0.clone(),
                        construction: None,
                    },
                    semantic_binding: SemanticContribution {
                        concept,
                        roles: Vec::new(),
                        polarity: None,
                        modality: None,
                    },
                    constraints: Vec::new(),
                    supporting_evidence: Vec::new(),
                    counter_evidence: Vec::new(),
                    confidence: 0.0,
                });
            candidate.supporting_evidence.push(observation.id.clone());
            candidate.confidence = (candidate.confidence + 0.5).min(1.0);
        }
        grouped.into_values().collect()
    }
}

fn expression_term(expression: &Expr) -> Option<TermId> {
    match expression {
        Expr::Value(Value::TermRef(term)) => Some(*term),
        Expr::Apply(apply) => Some(apply.operator),
        _ => None,
    }
}

pub struct Generalizer;

impl Generalizer {
    pub fn by_theme_type(
        surface: impl Into<String>,
        locale: impl Into<String>,
        theme_type: TypeId,
        semantic_binding: SemanticContribution,
        evidence: Vec<ObservationId>,
    ) -> KnowledgeCandidate {
        let surface = surface.into();
        KnowledgeCandidate {
            id: KnowledgeId::from_label(&format!("{surface}:{theme_type}")),
            pattern: LinguisticPattern {
                surface,
                locale: locale.into(),
                construction: Some("verb-theme-type".to_string()),
            },
            semantic_binding,
            constraints: vec![Constraint::TypeConstraint {
                value: Value::TypeRef(theme_type),
                ty: theme_type,
            }],
            supporting_evidence: evidence,
            counter_evidence: Vec::new(),
            confidence: 0.8,
        }
    }
}

pub struct KnowledgeConsolidator;

pub type DefinitionConsolidator = KnowledgeConsolidator;

impl KnowledgeConsolidator {
    pub fn consolidate(
        knowledge: &mut LinguisticKnowledge,
        candidate: &KnowledgeCandidate,
        revision: impl Into<String>,
    ) -> Result<KnowledgeId, ConsolidationError> {
        if candidate.supporting_evidence.is_empty() {
            return Err(ConsolidationError::NoEvidence);
        }
        if candidate.confidence < 0.5 {
            return Err(ConsolidationError::LowConfidence);
        }
        let evidence = EvidenceSet {
            items: candidate
                .supporting_evidence
                .iter()
                .map(|id| EvidenceRef {
                    kind: EvidenceKind::PositiveExample,
                    reference: id.clone(),
                    detail: "induced supporting observation".to_string(),
                })
                .chain(candidate.counter_evidence.iter().map(|id| EvidenceRef {
                    kind: EvidenceKind::NegativeExample,
                    reference: id.clone(),
                    detail: "induced counter observation".to_string(),
                }))
                .collect(),
        };
        let entry = LinguisticKnowledgeEntry {
            id: candidate.id,
            origin: KnowledgeOrigin::Learned,
            form: Some(FormPattern {
                surface: candidate.pattern.surface.clone(),
                locale: Some(candidate.pattern.locale.clone()),
                construction: candidate.pattern.construction.clone(),
            }),
            syntax: None,
            semantics: candidate.semantic_binding.clone(),
            constraints: candidate.constraints.clone(),
            pragmatics: Vec::new(),
            evidence,
            validity: Validity::Active,
            confidence: candidate.confidence,
            revision: revision.into(),
        };
        knowledge
            .add_learned(entry)
            .map_err(|error| ConsolidationError::Knowledge(error.to_string()))?;
        Ok(candidate.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_generalization_does_not_collapse_into_unconditional_rule() {
        let first = Generalizer::by_theme_type(
            "押さえる",
            "ja-JP",
            TypeId::new_random(1, 1),
            SemanticContribution {
                concept: TermId::new_random(2, 1),
                roles: Vec::new(),
                polarity: None,
                modality: None,
            },
            vec!["obs-1".to_string()],
        );
        let second = Generalizer::by_theme_type(
            "押さえる",
            "ja-JP",
            TypeId::new_random(3, 1),
            SemanticContribution {
                concept: TermId::new_random(4, 1),
                roles: Vec::new(),
                polarity: None,
                modality: None,
            },
            vec!["obs-2".to_string()],
        );
        assert_ne!(first.id, second.id);
        assert_eq!(first.constraints.len(), 1);
        assert_eq!(second.constraints.len(), 1);
    }
}
