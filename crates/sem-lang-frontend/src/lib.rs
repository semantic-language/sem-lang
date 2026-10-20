//! Deterministic frontend compilers driven by compiled language resources.

use sem_lang_core::TypeId;
use serde::{Deserialize, Serialize};

pub mod current;

pub use sem_lang_context::{CompileContext, SurfaceInput};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExceptionKind {
    UnknownSurface,
    UnknownGrammar,
    UnknownComposition,
    UnknownConcept,
    UnknownOperation,
    UnsupportedPragmaticForm,
    OntologyMismatch,
    MissingReferenceTime,
    MissingTimeZone,
    MissingLocation,
    MissingReferent,
    MissingContext,
    MultipleCandidates,
    UnderspecifiedValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceptionCase {
    pub kind: ExceptionKind,
    pub span: String,
    pub expected_type: Option<TypeId>,
    pub message: String,
}

impl ExceptionCase {
    pub fn kind(&self) -> &str {
        match self.kind {
            ExceptionKind::UnknownSurface => "UnknownSurface",
            ExceptionKind::UnknownGrammar => "UnknownGrammar",
            ExceptionKind::UnknownComposition => "UnknownComposition",
            ExceptionKind::UnknownConcept => "UnknownConcept",
            ExceptionKind::UnknownOperation => "UnknownOperation",
            ExceptionKind::UnsupportedPragmaticForm => "UnsupportedPragmaticForm",
            ExceptionKind::OntologyMismatch => "OntologyMismatch",
            ExceptionKind::MissingReferenceTime => "MissingReferenceTime",
            ExceptionKind::MissingTimeZone => "MissingTimeZone",
            ExceptionKind::MissingLocation => "MissingLocation",
            ExceptionKind::MissingReferent => "MissingReferent",
            ExceptionKind::MissingContext => "MissingContext",
            ExceptionKind::MultipleCandidates => "MultipleCandidates",
            ExceptionKind::UnderspecifiedValue => "UnderspecifiedValue",
        }
    }
}
