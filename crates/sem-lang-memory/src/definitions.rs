//! Explicit typed definitions; persistence is shared with binding adoption.
use crate::records::{self, Record, invalid};
use sem_lang_repository::{Repository, RepositoryError, RevisionId};
use sem_lang_vocab::{VocabularyModule, VocabularyRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
const SCHEMA: &str = "sem-lang://memory/definitions/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptedDefinitions {
    pub schema: String,
    pub namespace: String,
    pub decision_ref: String,
    pub previous_revision: Option<RevisionId>,
    pub modules: Vec<VocabularyModule>,
}
impl Record for AdoptedDefinitions {
    const PREFIX: &'static str = "definitions";
    fn decision_ref(&self) -> &str {
        &self.decision_ref
    }
    fn namespace(&self) -> &str {
        &self.namespace
    }
    fn parent(&self) -> Option<&RevisionId> {
        self.previous_revision.as_ref()
    }
    fn validate<R: Repository>(&self, _repository: &R) -> Result<(), RepositoryError> {
        records::head(Self::PREFIX, &self.namespace)?;
        if self.schema != SCHEMA
            || self.decision_ref.trim().is_empty()
            || self.decision_ref.len() > 512
            || self.modules.len() > 64
        {
            return Err(invalid("invalid adopted definition identity"));
        }
        let mut registry = VocabularyRegistry::new();
        let mut terms = BTreeSet::new();
        for module in &self.modules {
            if module.vocabulary_id.trim().is_empty()
                || module.version.trim().is_empty()
                || module.terms.iter().any(|term| {
                    term.semantic_definition.is_none()
                        || term.deprecation.is_some()
                        || term.replacement.is_some()
                        || !terms.insert(term.id)
                })
                || terms.len() > 4096
            {
                return Err(invalid("missing or conflicting typed semantic definition"));
            }
            registry
                .register(module.clone())
                .map_err(|e| invalid(&e.to_string()))?;
        }
        registry
            .validate_definitions()
            .map_err(|e| invalid(&e.to_string()))
    }
}
/// Explicit adoption, never a numeric matching decision. Caller verifies decision.
pub fn adopt<R: Repository + sem_lang_repository::Backend>(
    repository: &mut R,
    namespace: &str,
    expected: Option<&RevisionId>,
    modules: &[VocabularyModule],
    decision_ref: &str,
    expected_commit_revision: &sem_lang_repository::RevisionRef,
) -> Result<RevisionId, RepositoryError> {
    records::write(
        repository,
        &AdoptedDefinitions {
            schema: SCHEMA.into(),
            namespace: namespace.into(),
            decision_ref: decision_ref.into(),
            previous_revision: expected.cloned(),
            modules: modules.to_vec(),
        },
        expected_commit_revision,
    )
}
/// Missing is absence, not a default dictionary.
pub fn read<R: Repository>(
    repository: &R,
    namespace: &str,
) -> Result<Option<(RevisionId, AdoptedDefinitions)>, RepositoryError> {
    records::read(repository, namespace)
}
/// Exact immutable adoption, independent of today's head.
pub fn read_revision<R: Repository>(
    repository: &R,
    namespace: &str,
    revision: &RevisionId,
) -> Result<AdoptedDefinitions, RepositoryError> {
    records::read_at(repository, namespace, revision)
}
