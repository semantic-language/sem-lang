//! Exception and evolution primitives for unknown semantics.

use sem_lang_core::{TermId, TypeId};
use sem_lang_frontend::ExceptionCase;
use sem_lang_vocab::VocabularyRegistry;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone)]
pub struct InterpretationProposal {
    pub source_exception: ExceptionCase,
    pub proposed_interpretation: String,
    pub reused_existing_terms: Vec<TermId>,
    pub proposed_new_terms: Vec<TermId>,
    pub applicability_conditions: Vec<String>,
    pub positive_examples: Vec<String>,
    pub negative_examples: Vec<String>,
    pub evidence: String,
    pub confidence: f32,
    pub compatibility_impact: String,
    pub generated_regression_tests: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionProposal {
    pub source_exception: ExceptionCase,
    pub proposed_interpretation: String,
    pub proposed_new_terms: Vec<TermId>,
    pub evidence: String,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub enum ProposalClass {
    EphemeralResolution,
    SurfaceExtension,
    GrammarExtension,
    CompositionExtension,
    SemanticExtension,
    InvalidProposal,
}

#[derive(Debug, Clone)]
pub enum InterpreterError {
    UnknownCase(String),
    MissingEvidence(String),
    RejectedByPolicy,
}

impl Display for InterpreterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCase(msg) => write!(f, "unknown exception case: {msg}"),
            Self::MissingEvidence(msg) => write!(f, "missing evidence: {msg}"),
            Self::RejectedByPolicy => write!(f, "operation rejected by policy"),
        }
    }
}

impl std::error::Error for InterpreterError {}

#[derive(Debug, Clone)]
pub enum ReviewerError {
    InvalidProposal(String),
    Incompatible(String),
}

impl Display for ReviewerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProposal(msg) => write!(f, "invalid proposal: {msg}"),
            Self::Incompatible(msg) => write!(f, "incompatible proposal: {msg}"),
        }
    }
}

impl std::error::Error for ReviewerError {}

#[derive(Debug, Clone)]
pub enum ReviewDecision {
    Accepted(ProposalClass),
    Rejected(String),
    Deferred(String),
}

impl ReviewDecision {
    pub fn accepted(&self) -> bool {
        matches!(self, Self::Accepted(_))
    }
}

pub trait ExceptionInterpreter {
    fn propose(&self, case: &ExceptionCase) -> Result<InterpretationProposal, InterpreterError>;
}

pub trait ExtensionReviewer {
    fn review(
        &self,
        proposal: &ExtensionProposal,
        registry: &VocabularyRegistry,
    ) -> Result<ReviewDecision, ReviewerError>;
}

pub struct BasicExceptionInterpreter;

impl ExceptionInterpreter for BasicExceptionInterpreter {
    fn propose(&self, case: &ExceptionCase) -> Result<InterpretationProposal, InterpreterError> {
        if case.message.trim().is_empty() {
            return Err(InterpreterError::MissingEvidence(
                "empty exception message".to_string(),
            ));
        }

        let interpreted = format!("proposed({})", case.message);
        Ok(InterpretationProposal {
            source_exception: case.clone(),
            proposed_interpretation: interpreted,
            reused_existing_terms: Vec::new(),
            proposed_new_terms: Vec::new(),
            applicability_conditions: vec!["runtime-state must be present".to_string()],
            positive_examples: vec![case.span.clone()],
            negative_examples: vec!["forbidden".to_string()],
            evidence: case.message.clone(),
            confidence: 0.75,
            compatibility_impact: "low".to_string(),
            generated_regression_tests: vec!["should reject malformed token".to_string()],
        })
    }
}

pub struct BasicExtensionReviewer;

impl ExtensionReviewer for BasicExtensionReviewer {
    fn review(
        &self,
        proposal: &ExtensionProposal,
        _registry: &VocabularyRegistry,
    ) -> Result<ReviewDecision, ReviewerError> {
        if proposal.proposed_interpretation.trim().is_empty() {
            return Err(ReviewerError::InvalidProposal(
                "empty interpretation".to_string(),
            ));
        }

        if proposal.confidence < 0.2 {
            return Err(ReviewerError::Incompatible(
                "confidence too low for review".to_string(),
            ));
        }

        let class = classify_proposal(proposal);
        // A proposal is never a language change by itself. The service must
        // keep it pending until a human or an explicitly trusted release
        // process commits a new vocabulary revision.
        Ok(ReviewDecision::Deferred(format!(
            "manual-extension-required:{class:?}"
        )))
    }
}

pub fn classify_proposal(proposal: &ExtensionProposal) -> ProposalClass {
    match &proposal.source_exception.kind {
        e if format!("{e:?}").contains("Unknown") => ProposalClass::SurfaceExtension,
        _ if proposal.proposed_interpretation.contains("grammar") => {
            ProposalClass::GrammarExtension
        }
        _ if proposal.proposed_interpretation.contains("composition") => {
            ProposalClass::CompositionExtension
        }
        _ if proposal.proposed_interpretation.contains("semantic") => {
            ProposalClass::SemanticExtension
        }
        _ => ProposalClass::EphemeralResolution,
    }
}

#[derive(Debug, Clone)]
pub struct ProposalValidator;

impl ProposalValidator {
    pub fn validate(proposal: &ExtensionProposal) -> bool {
        !proposal.source_exception.message.is_empty()
            && !proposal.proposed_interpretation.is_empty()
            && (0.0..=1.0).contains(&proposal.confidence)
    }
}

#[derive(Debug, Clone)]
pub struct ExtensionClassifier;

impl ExtensionClassifier {
    pub fn classify(proposal: &ExtensionProposal) -> ProposalClass {
        classify_proposal(proposal)
    }
}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub term_type: Option<TypeId>,
    pub term_id: Option<TermId>,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_frontend::{ExceptionCase, ExceptionKind};

    #[test]
    fn propose_and_review() {
        let case = ExceptionCase {
            kind: ExceptionKind::UnknownSurface,
            span: "仮押さえ".to_string(),
            expected_type: None,
            message: "unknown lexical unit".to_string(),
        };
        let proposal = BasicExceptionInterpreter.propose(&case).expect("proposal");
        let extension = ExtensionProposal {
            source_exception: proposal.source_exception,
            proposed_interpretation: "surface-extend".to_string(),
            proposed_new_terms: vec![],
            evidence: proposal.evidence,
            confidence: proposal.confidence,
        };
        let decision = BasicExtensionReviewer
            .review(&extension, &VocabularyRegistry::new())
            .expect("review");
        assert!(!decision.accepted());
        assert!(matches!(decision, ReviewDecision::Deferred(_)));
    }
}
