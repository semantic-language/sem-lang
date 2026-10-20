//! Product-independent meaning-to-capability references. No execution or grants.
use crate::{
    definitions,
    records::{self, Record, invalid},
};
use sem_lang_core::{Digest, TermId};
use sem_lang_repository::{Repository, RepositoryError, RevisionId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const SCHEMA: &str = "sem-lang://memory/bindings/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionRef {
    pub namespace: String,
    pub revision: RevisionId,
    pub term: TermId,
}

/// Opaque identity only. This package neither dereferences nor interprets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRef {
    pub namespace: String,
    pub capability: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingUsage {
    Resource,
    Action,
    Observe,
    Presentation,
    Provider,
}

/// Flow relative to the external capability: Input sends, Output receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
    Bidirectional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticBinding {
    pub binding_id: String,
    pub semantic_ref: DefinitionRef,
    pub target_ref: CapabilityRef,
    pub usage: BindingUsage,
    pub direction: Direction,
    /// Exact semantic conditions, not provider input schemas or authorization.
    pub constraints: Vec<DefinitionRef>,
    pub provenance_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionedBinding {
    pub revision: RevisionId,
    pub binding: SemanticBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionState {
    Proposed,
    Reviewed,
    Adopted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingAdoption {
    pub schema: String,
    pub namespace: String,
    pub previous_revision: Option<RevisionId>,
    /// Earlier accepted adoption kept effective while reviewing a replacement.
    /// A reference only: no second binding body or separate authority.
    pub adopted_revision: Option<RevisionId>,
    pub decision_ref: String,
    pub state: AdoptionState,
    pub bindings: Vec<RevisionedBinding>,
}

fn reference(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.bytes().all(|c| c.is_ascii_graphic())
}

fn validate_definition<R: Repository>(
    repository: &R,
    reference: &DefinitionRef,
) -> Result<(), RepositoryError> {
    let memory = definitions::read_revision(repository, &reference.namespace, &reference.revision)?;
    if !memory
        .modules
        .iter()
        .any(|module| module.terms.iter().any(|term| term.id == reference.term))
    {
        return Err(invalid(
            "binding definition does not exist at the exact revision",
        ));
    }
    Ok(())
}

fn revision(binding: &SemanticBinding) -> Result<String, RepositoryError> {
    let bytes = serde_json::to_vec(binding).map_err(|e| invalid(&e.to_string()))?;
    Ok(format!(
        "binding-{}",
        Digest::object_digest(&bytes).to_hex()
    ))
}

impl Record for BindingAdoption {
    const PREFIX: &'static str = "bindings";
    fn decision_ref(&self) -> &str {
        &self.decision_ref
    }
    fn namespace(&self) -> &str {
        &self.namespace
    }
    fn parent(&self) -> Option<&RevisionId> {
        self.previous_revision.as_ref()
    }
    fn validate_parent(&self, parent: Option<&Self>) -> Result<(), RepositoryError> {
        let allowed = match (parent, self.state) {
            (_, AdoptionState::Proposed) => true,
            (Some(previous), AdoptionState::Reviewed) => {
                previous.state == AdoptionState::Proposed && previous.bindings == self.bindings
            }
            (Some(previous), AdoptionState::Adopted) => {
                previous.state == AdoptionState::Reviewed && previous.bindings == self.bindings
            }
            (Some(previous), AdoptionState::Rejected) => {
                previous.state != AdoptionState::Rejected && previous.bindings == self.bindings
            }
            _ => false,
        };
        let adopted = match (parent, self.state) {
            (_, AdoptionState::Adopted) => None,
            (Some(previous), AdoptionState::Rejected)
                if previous.state == AdoptionState::Adopted =>
            {
                None
            }
            (Some(previous), _) if previous.state == AdoptionState::Adopted => {
                self.previous_revision.as_ref()
            }
            (Some(previous), _) => previous.adopted_revision.as_ref(),
            (None, _) => None,
        };
        if !allowed || self.adopted_revision.as_ref() != adopted {
            return Err(invalid("invalid binding adoption parent relation"));
        }
        Ok(())
    }
    fn validate<R: Repository>(&self, repository: &R) -> Result<(), RepositoryError> {
        records::head(Self::PREFIX, &self.namespace)?;
        if self.schema != SCHEMA
            || !reference(&self.decision_ref)
            || self.bindings.is_empty()
            || self.bindings.len() > 256
            || self
                .adopted_revision
                .as_ref()
                .is_some_and(|id| !reference(id))
            || (self.state == AdoptionState::Adopted && self.adopted_revision.is_some())
        {
            return Err(invalid("invalid binding adoption identity or limit"));
        }
        let mut ids = BTreeSet::new();
        for item in &self.bindings {
            let b = &item.binding;
            if !reference(&b.binding_id)
                || !ids.insert(&b.binding_id)
                || !reference(&b.provenance_ref)
                || !reference(&b.target_ref.namespace)
                || !reference(&b.target_ref.capability)
                || b.target_ref
                    .revision
                    .as_ref()
                    .is_some_and(|v| !reference(v))
                || b.constraints.len() > 16
                || item.revision != revision(b)?
            {
                return Err(invalid("invalid semantic binding identity"));
            }
            match (b.usage, b.direction) {
                (
                    BindingUsage::Observe | BindingUsage::Presentation,
                    Direction::Input | Direction::Bidirectional,
                )
                | (BindingUsage::Action, Direction::Output) => {
                    return Err(invalid("binding direction contradicts usage"));
                }
                _ => {}
            }
            validate_definition(repository, &b.semantic_ref)?;
            let mut constraints = BTreeSet::new();
            for condition in &b.constraints {
                if !constraints.insert((&condition.namespace, &condition.revision, condition.term))
                {
                    return Err(invalid("duplicate semantic constraint"));
                }
                validate_definition(repository, condition)?;
            }
        }
        Ok(())
    }
}

/// Explicit adoption using definition storage's immutable/CAS path. Caller
/// authenticates decision references; no permission or availability is inferred.
pub fn write<R: Repository + sem_lang_repository::Backend>(
    repository: &mut R,
    namespace: &str,
    expected: Option<&RevisionId>,
    bindings: &[SemanticBinding],
    state: AdoptionState,
    decision_ref: &str,
    expected_commit_revision: &sem_lang_repository::RevisionRef,
) -> Result<RevisionId, RepositoryError> {
    if bindings.len() > 256 {
        return Err(invalid("binding count exceeds limit"));
    }
    let bindings = bindings
        .iter()
        .map(|binding| {
            Ok(RevisionedBinding {
                revision: revision(binding)?,
                binding: binding.clone(),
            })
        })
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    // Validate the stated base, not today's head: permits exact retries after
    // acknowledgement loss, while shared storage still fences stale CAS.
    let base: Option<BindingAdoption> = expected
        .map(|id| read_revision(repository, namespace, id))
        .transpose()?;
    let allowed = match (&base, state) {
        (None, AdoptionState::Proposed) => true,
        (Some(_), AdoptionState::Proposed) => true,
        (Some(previous), AdoptionState::Reviewed) => {
            previous.state == AdoptionState::Proposed && previous.bindings == bindings
        }
        (Some(previous), AdoptionState::Adopted) => {
            previous.state == AdoptionState::Reviewed && previous.bindings == bindings
        }
        (Some(previous), AdoptionState::Rejected) => {
            previous.state != AdoptionState::Rejected && previous.bindings == bindings
        }
        _ => false,
    };
    if !allowed {
        return Err(invalid("explicit binding adoption transition required"));
    }
    let adopted_revision = match (&base, state) {
        (_, AdoptionState::Adopted) => None,
        (Some(previous), AdoptionState::Rejected) if previous.state == AdoptionState::Adopted => {
            None
        }
        (Some(previous), _) if previous.state == AdoptionState::Adopted => expected.cloned(),
        (Some(previous), _) => previous.adopted_revision.clone(),
        (None, _) => None,
    };
    if let Some(id) = &adopted_revision {
        let accepted = read_revision(repository, namespace, id)?;
        if accepted.state != AdoptionState::Adopted || accepted.adopted_revision.is_some() {
            return Err(invalid(
                "binding adoption reference is not an adopted revision",
            ));
        }
    }
    records::write(
        repository,
        &BindingAdoption {
            schema: SCHEMA.into(),
            namespace: namespace.into(),
            previous_revision: expected.cloned(),
            adopted_revision,
            decision_ref: decision_ref.into(),
            state,
            bindings,
        },
        expected_commit_revision,
    )
}

pub fn read<R: Repository>(
    repository: &R,
    namespace: &str,
) -> Result<Option<(RevisionId, BindingAdoption)>, RepositoryError> {
    records::read(repository, namespace)
}

pub fn read_revision<R: Repository>(
    repository: &R,
    namespace: &str,
    revision: &RevisionId,
) -> Result<BindingAdoption, RepositoryError> {
    records::read_at(repository, namespace, revision)
}

/// All exact adopted matches, not an arbitrary first match or a permission.
pub fn resolve<R: Repository>(
    repository: &R,
    namespace: &str,
    semantic_ref: &DefinitionRef,
) -> Result<Vec<RevisionedBinding>, RepositoryError> {
    let Some((_, mut record)) = read(repository, namespace)? else {
        return Ok(vec![]);
    };
    if record.state != AdoptionState::Adopted {
        let Some(id) = record.adopted_revision else {
            return Ok(vec![]);
        };
        record = read_revision(repository, namespace, &id)?;
        if record.state != AdoptionState::Adopted || record.adopted_revision.is_some() {
            return Err(invalid(
                "binding adoption reference is not an adopted revision",
            ));
        }
    }
    Ok(record
        .bindings
        .into_iter()
        .filter(|b| b.binding.semantic_ref == *semantic_ref)
        .collect())
}
