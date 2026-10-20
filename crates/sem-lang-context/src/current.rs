//! Borrowed current context read and one evidence selector. No mirror or writes.
use crate::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ContextReference {
    Active,
    Session,
    Default(String),
    Decision(String),
    /// Exact retained owner input, not an inferred fact or vector position.
    Observation(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextStatus {
    Applicable,
    Candidate,
    Observed,
    Expired,
    OutOfScope,
    Rejected,
    Promoted,
    Replaced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum CurrentReadError {
    #[error("invalid context reference")]
    InvalidReference,
    #[error("invalid context subject")]
    InvalidSubject,
    #[error("context evaluation instant required")]
    MissingInstant,
    #[error("invalid context value or validity")]
    InvalidContext,
    #[error("context read capacity exceeded: {observed} > {limit}")]
    LimitExceeded { limit: usize, observed: usize },
}

pub struct CurrentContextEntry<'a> {
    pub reference: &'a ContextReference,
    pub kind: Option<ContextKind>,
    pub value: Option<&'a str>,
    pub authority: ContextAuthority,
    pub source: ContextSource,
    pub sensitivity: u8,
    pub scope: Option<ContextScope>,
    pub validity: Option<&'a ContextValidity>,
    pub status: ContextStatus,
}

pub struct CurrentContextRead<'a> {
    pub entries: Vec<CurrentContextEntry<'a>>,
}

#[derive(Clone, Copy)]
pub struct ContextEvidence<'a> {
    pub value: &'a str,
    pub authority: ContextAuthority,
}

pub struct ContextSelection<'a> {
    pub values: Vec<&'a str>,
    pub authority: Option<ContextAuthority>,
}
impl ContextSelection<'_> {
    pub fn conflicting(&self) -> bool {
        self.values.len() > 1
    }
}

/// Higher-ranked conflicting evidence is retained, never replaced by a weaker
/// unique default. Equal values coalesce without discarding source references.
pub fn select_evidence<'a>(
    evidence: impl IntoIterator<Item = ContextEvidence<'a>>,
) -> ContextSelection<'a> {
    let mut values = BTreeSet::new();
    let mut authority: Option<ContextAuthority> = None;
    for item in evidence {
        let rank = authority.map_or(0, ContextAuthority::rank);
        if item.authority.rank() > rank {
            values.clear();
            authority = Some(item.authority);
        }
        if item.authority.rank() >= rank {
            values.insert(item.value);
        }
    }
    ContextSelection {
        values: values.into_iter().collect(),
        authority,
    }
}

impl<'a> CurrentContextRead<'a> {
    pub fn select(&self, kind: ContextKind, extra: &[ContextEvidence<'a>]) -> ContextSelection<'a> {
        select_evidence(
            self.entries
                .iter()
                .filter_map(|entry| {
                    (entry.kind == Some(kind) && entry.status == ContextStatus::Applicable)
                        .then_some(entry.value)
                        .flatten()
                        .map(|value| ContextEvidence {
                            value,
                            authority: entry.authority,
                        })
                })
                .chain(extra.iter().copied()),
        )
    }
}

impl ContextEngine {
    /// Resolves exact references without copying candidate/context bodies.
    pub fn read_current<'a>(
        &'a self,
        references: &'a [ContextReference],
        subject: &ContextSubject,
        scope: ContextScope,
        at: Option<i64>,
    ) -> Result<CurrentContextRead<'a>, CurrentReadError> {
        if subject.id.trim().is_empty() || subject.class.trim().is_empty() {
            return Err(CurrentReadError::InvalidSubject);
        }
        if references.len() > 64 {
            return Err(CurrentReadError::LimitExceeded {
                limit: 64,
                observed: references.len(),
            });
        }
        if references.iter().collect::<BTreeSet<_>>().len() != references.len() {
            return Err(CurrentReadError::InvalidReference);
        }
        let mut entries = Vec::new();
        for reference in references {
            let mut entry = match reference {
                ContextReference::Active | ContextReference::Session => {
                    // These records have no per-record subject. Only an explicitly
                    // subject-bound engine can expose them; no anonymous owner guess.
                    if self.subject_ref.as_ref() != Some(subject) {
                        return Err(CurrentReadError::InvalidSubject);
                    }
                    let binding = match reference {
                        ContextReference::Active => self.store.active_timezone.as_ref(),
                        _ => self.store.session_timezone.as_ref(),
                    }
                    .ok_or(CurrentReadError::InvalidReference)?;
                    if &binding.subject != subject {
                        return Err(CurrentReadError::InvalidSubject);
                    }
                    CurrentContextEntry {
                        reference,
                        kind: Some(binding.kind),
                        value: Some(&binding.value),
                        source: binding.source,
                        sensitivity: binding.sensitivity,
                        authority: binding.authority,
                        scope: Some(binding.scope),
                        validity: Some(&binding.validity),
                        status: ContextStatus::Applicable,
                    }
                }
                ContextReference::Default(id) => {
                    let mut matches = self
                        .store
                        .persistent_defaults
                        .iter()
                        .filter(|v| &v.id == id);
                    let item = matches.next().ok_or(CurrentReadError::InvalidReference)?;
                    if matches.next().is_some() {
                        return Err(CurrentReadError::InvalidReference);
                    }
                    if &item.subject != subject {
                        return Err(CurrentReadError::InvalidSubject);
                    }
                    if item.revision.is_empty() || item.promotion_ref.trim().is_empty() {
                        return Err(CurrentReadError::InvalidContext);
                    }
                    CurrentContextEntry {
                        reference,
                        kind: Some(item.kind),
                        value: Some(&item.value),
                        source: ContextSource::PersistentDefault,
                        sensitivity: 0,
                        authority: ContextAuthority::PersistentDefault,
                        scope: Some(item.scope),
                        validity: Some(&item.validity),
                        status: ContextStatus::Applicable,
                    }
                }
                ContextReference::Decision(id) => {
                    let item = self
                        .store
                        .decided_candidates
                        .get(id)
                        .ok_or(CurrentReadError::InvalidReference)?;
                    if &item.subject_ref != subject {
                        return Err(CurrentReadError::InvalidSubject);
                    }
                    if item
                        .candidate
                        .reference()
                        .map_err(|_| CurrentReadError::InvalidContext)?
                        != *id
                    {
                        return Err(CurrentReadError::InvalidReference);
                    }
                    let status = match item.disposition {
                        CandidateDisposition::SelectedShortTerm => ContextStatus::Applicable,
                        CandidateDisposition::Pending => ContextStatus::Candidate,
                        // Promoted candidates are not a second source of accepted facts.
                        CandidateDisposition::Promoted => ContextStatus::Promoted,
                        CandidateDisposition::Rejected => ContextStatus::Rejected,
                        CandidateDisposition::Replaced => ContextStatus::Replaced,
                    };
                    CurrentContextEntry {
                        reference,
                        kind: Some(item.candidate.kind),
                        value: Some(&item.candidate.value),
                        source: ContextSource::ExplicitUserStatement,
                        sensitivity: 0,
                        authority: ContextAuthority::ActiveContext,
                        scope: Some(item.candidate.inferred_scope),
                        validity: Some(&item.candidate.inferred_validity),
                        status,
                    }
                }
                ContextReference::Observation(id) => {
                    self.observation(id, subject).map_err(|error| match error {
                        ObservationError::SubjectMismatch => CurrentReadError::InvalidSubject,
                        ObservationError::Missing => CurrentReadError::InvalidReference,
                        ObservationError::CapacityExceeded { limit, observed } => {
                            CurrentReadError::LimitExceeded { limit, observed }
                        }
                        _ => CurrentReadError::InvalidContext,
                    })?;
                    CurrentContextEntry {
                        reference,
                        kind: None,
                        value: None,
                        source: ContextSource::ExternalProvided,
                        sensitivity: 0,
                        authority: ContextAuthority::Inferred,
                        scope: None,
                        validity: None,
                        status: ContextStatus::Observed,
                    }
                }
            };
            if let Some(value) = entry.value
                && (value.trim().is_empty() || value.len() > 16_384 || value.contains('\0'))
            {
                return Err(CurrentReadError::InvalidContext);
            }
            if let Some(validity) = entry.validity {
                if !validity.valid() {
                    return Err(CurrentReadError::InvalidContext);
                }
                if !validity
                    .applicable(at)
                    .ok_or(CurrentReadError::MissingInstant)?
                {
                    entry.status = ContextStatus::Expired;
                } else if entry.scope.is_some_and(|value| !value.permits(scope)) {
                    entry.status = ContextStatus::OutOfScope;
                }
            }
            entries.push(entry);
        }
        Ok(CurrentContextRead { entries })
    }
}
