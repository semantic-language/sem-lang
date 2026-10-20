//! Typed language input and explicit retained observations share one owner.
//! Retention never supplies an interpretation, promotion, identity or default.
use crate::{
    ContextEngine, ContextError, ContextReference, ContextScope, ContextStore, ContextSubject,
    MAX_CONTEXT_BYTES, SurfaceRetention,
};
use sem_lang_core::{Digest, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceInput {
    pub reference: String,
    pub text: String,
}

impl SurfaceInput {
    pub fn new(reference: &str, text: &str) -> Self {
        Self {
            reference: reference.into(),
            text: text.into(),
        }
    }
}

/// Exact revision-bound selection, shared with the frontend without an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileContext {
    pub subject: ContextSubject,
    pub evidence: Vec<NodeId>,
    pub at: Option<i64>,
    pub locale: String,
    pub semantic_revision: String,
    pub context_refs: Vec<ContextReference>,
    pub context_scope: ContextScope,
    pub observation_evidence: Vec<NodeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationRequest {
    pub input: SurfaceInput,
    pub context: CompileContext,
    pub retention: SurfaceRetention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedObservation {
    pub reference: String,
    pub input_ref: String,
    pub input_digest: String,
    pub surface: Option<String>,
    pub context: CompileContext,
    pub retention: SurfaceRetention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReceipt {
    pub reference: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ObservationError {
    #[error("invalid observation")]
    InvalidInput,
    #[error("observation identity conflicts")]
    Conflict,
    #[error("observation content does not match its reference")]
    DigestMismatch,
    #[error("observation is missing")]
    Missing,
    #[error("observation subject does not match")]
    SubjectMismatch,
    #[error("observation surface was not retained")]
    SurfaceUnavailable,
    #[error("observation capacity exceeded: {observed} > {limit}")]
    CapacityExceeded { limit: usize, observed: usize },
}

fn bound(observed: usize, limit: usize) -> Result<(), ObservationError> {
    if observed > limit {
        Err(ObservationError::CapacityExceeded { limit, observed })
    } else {
        Ok(())
    }
}
fn token(value: &str) -> Result<(), ObservationError> {
    bound(value.len(), 512)?;
    if value.is_empty() || value.trim() != value || value.contains('\0') {
        return Err(ObservationError::InvalidInput);
    }
    Ok(())
}
fn digest(value: &str) -> Result<(), ObservationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ObservationError::DigestMismatch);
    }
    Ok(())
}
fn hash(value: &impl Serialize) -> Result<String, ObservationError> {
    Digest::from_serializable(1, value)
        .map(|v| v.to_hex())
        .map_err(|_| ObservationError::InvalidInput)
}

impl CompileContext {
    fn validate_observation(&self) -> Result<(), ObservationError> {
        token(&self.subject.id)?;
        token(&self.subject.class)?;
        token(&self.locale)?;
        digest(&self.semantic_revision)?;
        bound(self.evidence.len(), 64)?;
        bound(self.context_refs.len(), 64)?;
        bound(self.observation_evidence.len(), 64)?;
        let evidence: BTreeSet<_> = self.evidence.iter().collect();
        if evidence.len() != self.evidence.len()
            || self
                .observation_evidence
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.observation_evidence.len()
            || self
                .observation_evidence
                .iter()
                .any(|id| !evidence.contains(id))
            || self.context_refs.iter().collect::<BTreeSet<_>>().len() != self.context_refs.len()
        {
            return Err(ObservationError::InvalidInput);
        }
        for reference in &self.context_refs {
            match reference {
                ContextReference::Default(id) | ContextReference::Decision(id) => token(id)?,
                ContextReference::Observation(id) => digest(id)?,
                ContextReference::Active | ContextReference::Session => {}
            }
        }
        Ok(())
    }
}

impl RetainedObservation {
    fn from_request(request: ObservationRequest) -> Result<Self, ObservationError> {
        request.context.validate_observation()?;
        token(&request.input.reference)?;
        bound(request.input.text.len(), 65_536)?;
        if request.input.text.trim().is_empty() || request.input.text.contains('\0') {
            return Err(ObservationError::InvalidInput);
        }
        let input_digest = hash(&(&request.input, &request.context))?;
        let mut record = Self {
            reference: String::new(),
            input_ref: request.input.reference,
            input_digest,
            surface: (request.retention != SurfaceRetention::DiscardAfterCompile)
                .then_some(request.input.text),
            context: request.context,
            retention: request.retention,
        };
        record.reference = record.content_ref()?;
        record.validate()?;
        Ok(record)
    }
    fn content_ref(&self) -> Result<String, ObservationError> {
        hash(&(
            &self.input_ref,
            &self.input_digest,
            &self.surface,
            &self.context,
            self.retention,
        ))
    }
    pub fn validate(&self) -> Result<(), ObservationError> {
        token(&self.input_ref)?;
        self.context.validate_observation()?;
        digest(&self.reference)?;
        digest(&self.input_digest)?;
        match (&self.surface, self.retention) {
            (None, SurfaceRetention::DiscardAfterCompile) => {}
            (Some(_), SurfaceRetention::DiscardAfterCompile) | (None, _) => {
                return Err(ObservationError::InvalidInput);
            }
            (Some(text), _) => {
                bound(text.len(), 65_536)?;
                if text.trim().is_empty() || text.contains('\0') {
                    return Err(ObservationError::InvalidInput);
                }
                if hash(&(&SurfaceInput::new(&self.input_ref, text), &self.context))?
                    != self.input_digest
                {
                    return Err(ObservationError::DigestMismatch);
                }
            }
        }
        if self.content_ref()? != self.reference {
            return Err(ObservationError::DigestMismatch);
        }
        Ok(())
    }
    /// Reconstruct only a retained input, with its immutable owner reference.
    /// Caller must explicitly select/rebase the current context after a known
    /// memory transition; no implicit current-head or context fallback occurs here.
    pub fn surface_input(&self) -> Result<SurfaceInput, ObservationError> {
        self.validate()?;
        let text = self
            .surface
            .as_deref()
            .ok_or(ObservationError::SurfaceUnavailable)?;
        Ok(SurfaceInput::new(&self.reference, text))
    }
}

impl ContextStore {
    pub fn validate_observations(&self) -> Result<(), ObservationError> {
        bound(self.short_term.len(), 128)?;
        let mut ids = BTreeSet::new();
        let mut bytes: usize = 2;
        for record in &self.short_term {
            record.validate()?;
            if !ids.insert((
                &record.context.subject.id,
                &record.context.subject.class,
                &record.input_ref,
            )) {
                return Err(ObservationError::Conflict);
            }
            bytes = bytes.saturating_add(
                serde_json::to_vec(record)
                    .map_err(|_| ObservationError::InvalidInput)?
                    .len()
                    + 1,
            );
            bound(bytes, MAX_CONTEXT_BYTES)?;
        }
        Ok(())
    }
}

impl ContextEngine {
    pub(crate) fn retain_semantics(
        &mut self,
        request: ObservationRequest,
    ) -> Result<ObservationReceipt, ContextError> {
        let record = RetainedObservation::from_request(request)?;
        self.store.validate_observations()?;
        if let Some(previous) = self.store.short_term.iter().find(|item| {
            item.input_ref == record.input_ref && item.context.subject == record.context.subject
        }) {
            if previous != &record {
                return Err(ObservationError::Conflict.into());
            }
            return Ok(ObservationReceipt {
                reference: previous.reference.clone(),
                changed: false,
            });
        }
        bound(self.store.short_term.len().saturating_add(1), 128)?;
        let added = serde_json::to_vec(&record)
            .map_err(|_| ObservationError::InvalidInput)?
            .len()
            + 1;
        bound(
            self.short_term_size_bytes()?.saturating_add(added),
            MAX_CONTEXT_BYTES,
        )?;
        let reference = record.reference.clone();
        self.store.short_term.push(record);
        Ok(ObservationReceipt {
            reference,
            changed: true,
        })
    }
    pub fn observation(
        &self,
        reference: &str,
        subject: &ContextSubject,
    ) -> Result<&RetainedObservation, ObservationError> {
        digest(reference)?;
        self.store.validate_observations()?;
        let record = self
            .store
            .short_term
            .iter()
            .find(|entry| entry.reference == reference)
            .ok_or(ObservationError::Missing)?;
        if &record.context.subject != subject {
            return Err(ObservationError::SubjectMismatch);
        }
        Ok(record)
    }
}
