//! Bounded, disposable lookup over an owner-read semantic image. This is not
//! an admission, identity, equivalence, authentication or persistence authority.
//! The owner supplies exact predicate definitions; labels never select rules.

use crate::PacketReducer;
use sem_lang_core::{
    Claim, Constraint, Digest, EntityId, NodeId, PacketId, Provenance, RoleId, RuntimeState,
    TermId, TypeId, Value,
};
use sem_lang_vocab::VocabularyRegistry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zixcel_revision::RevisionRef;

const MAX_CLAIMS: usize = 4096;
const MAX_PACKETS: usize = 4096;
const MAX_RULES: usize = 64;
const MAX_ENTITIES: usize = 256;
const MAX_FACTS: usize = 64;
const MAX_TEXT: usize = 1024;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexField {
    Label,
    Identifier,
    Relation,
}

/// A projection rule, not a new semantic definition. `actor` is the subject of
/// the indexed assertion and `value_role` selects an explicit argument role.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexRule {
    pub predicate: TermId,
    pub definition_digest: Digest,
    pub value_role: RoleId,
    pub field: IndexField,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FactEvidence {
    pub claim: NodeId,
    pub claim_digest: Digest,
    pub packet: PacketId,
    pub packet_digest: Digest,
    pub provenance: Option<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndexedText {
    pub value: String,
    pub evidence: Vec<FactEvidence>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndexedRelation {
    pub target: EntityId,
    pub predicate: TermId,
    pub evidence: Vec<FactEvidence>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndexedType {
    pub type_id: TypeId,
    pub definition_digest: Digest,
    pub evidence: Vec<FactEvidence>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EntityIndexEntry {
    pub entity: EntityId,
    pub types: Vec<IndexedType>,
    pub labels: Vec<IndexedText>,
    pub identifiers: Vec<IndexedText>,
    pub relations: Vec<IndexedRelation>,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    #[error("semantic index capacity exceeded")]
    LimitExceeded,
    #[error("exact semantic definition unavailable")]
    DefinitionUnavailable,
    #[error("exact assertion provenance unavailable")]
    ProvenanceUnavailable,
    #[error("semantic entity unavailable")]
    EntityUnavailable,
    #[error("invalid semantic index input")]
    InvalidInput,
}

/// No deserializer: rebuild from the current owner image, never restore a cache
/// as canonical truth. The caller must retain the source owner/scope alongside
/// the EntityId and this exact revision when exposing references externally.
#[derive(Debug)]
pub struct EntityIndex {
    revision: RevisionRef,
    rules_digest: Digest,
    evaluated_at: i64,
    entries: BTreeMap<EntityId, EntityIndexEntry>,
}
impl EntityIndex {
    pub fn build(
        state: &RuntimeState,
        registry: &VocabularyRegistry,
        rules: &[IndexRule],
        now: i64,
    ) -> Result<Self, IndexError> {
        if state.claims.len() > MAX_CLAIMS
            || state.packets.len() > MAX_PACKETS
            || rules.len() > MAX_RULES
        {
            return Err(IndexError::LimitExceeded);
        }
        // Bound bytes/work before deriving digests or copying source strings.
        bounded_size(&(&state.claims, &state.packets))?;
        let revision =
            PacketReducer::commit_revision(state).map_err(|_| IndexError::InvalidInput)?;
        // Adopted definitions belong to the restored registry, not RuntimeState's
        // optional dictionary image. Never repair/copy that image on a read.
        let mut definitions = BTreeMap::new();
        let mut type_definitions = BTreeMap::new();
        let mut roles = std::collections::BTreeSet::new();
        let mut definition_bytes = MAX_BYTES;
        let mut count = 0usize;
        for module in registry.modules() {
            count = count
                .checked_add(1 + module.terms.len() + module.types.len() + module.roles.len())
                .ok_or(IndexError::LimitExceeded)?;
            if count > MAX_CLAIMS {
                return Err(IndexError::LimitExceeded);
            }
            charge(module, &mut definition_bytes)?;
            for term in &module.terms {
                if definitions.insert(term.id, term).is_some() {
                    return Err(IndexError::DefinitionUnavailable);
                }
            }
            for ty in &module.types {
                if type_definitions.insert(ty.id, ty).is_some() {
                    return Err(IndexError::DefinitionUnavailable);
                }
            }
            for role in &module.roles {
                if !roles.insert(role.id) {
                    return Err(IndexError::DefinitionUnavailable);
                }
            }
        }
        let mut by_term = BTreeMap::new();
        for rule in rules {
            let term = definitions
                .get(&rule.predicate)
                .ok_or(IndexError::DefinitionUnavailable)?;
            if term.id != rule.predicate
                || Digest::from_serializable(1, term).map_err(|_| IndexError::InvalidInput)?
                    != rule.definition_digest
                || term.deprecation.is_some()
                || term.semantic_definition.is_none()
                || !roles.contains(&rule.value_role)
                || by_term.insert(rule.predicate, rule).is_some()
            {
                return Err(IndexError::DefinitionUnavailable);
            }
        }
        // Match the complete current assertion, not just a reusable Claim id.
        let mut evidence: BTreeMap<NodeId, Vec<FactEvidence>> = BTreeMap::new();
        let mut evidence_bytes = MAX_BYTES;
        let mut assertions = 0;
        for (id, packet) in &state.packets {
            if *id != packet.id {
                return Err(IndexError::InvalidInput);
            }
            if let Some(delta) = &packet.context_delta {
                assertions += delta.added_claims.len();
                if assertions > MAX_CLAIMS {
                    return Err(IndexError::LimitExceeded);
                }
                let packet_digest =
                    Digest::from_serializable(1, packet).map_err(|_| IndexError::InvalidInput)?;
                for claim in &delta.added_claims {
                    if state.claims.get(&claim.id) != Some(claim) {
                        continue;
                    }
                    let refs = evidence.entry(claim.id).or_default();
                    if refs.len() >= MAX_FACTS {
                        return Err(IndexError::LimitExceeded);
                    }
                    // Account before cloning the packet's source description.
                    charge(&packet.provenance, &mut evidence_bytes)?;
                    evidence_bytes = evidence_bytes
                        .checked_sub(256)
                        .ok_or(IndexError::LimitExceeded)?;
                    refs.push(FactEvidence {
                        claim: claim.id,
                        claim_digest: Digest::from_serializable(1, claim)
                            .map_err(|_| IndexError::InvalidInput)?,
                        packet: *id,
                        packet_digest,
                        provenance: packet.provenance.clone(),
                    });
                }
            }
        }
        let mut entries = BTreeMap::new();
        let mut entry_bytes = MAX_BYTES;
        for (id, claim) in &state.claims {
            if *id != claim.id {
                return Err(IndexError::InvalidInput);
            }
            if !active(claim, now) {
                continue;
            }
            let Some(Value::Entity(entity)) = claim.actor else {
                continue;
            };
            let types: Vec<_> = claim
                .constraints
                .iter()
                .filter_map(|c| match c {
                    Constraint::TypeConstraint {
                        value: Value::Entity(subject),
                        ty,
                    } if *subject == entity => Some(*ty),
                    _ => None,
                })
                .collect();
            let rule = by_term.get(&claim.predicate);
            if types.is_empty() && rule.is_none() {
                continue;
            }
            let refs = evidence.get(id).ok_or(IndexError::ProvenanceUnavailable)?;
            if !entries.contains_key(&entity) && entries.len() >= MAX_ENTITIES {
                return Err(IndexError::LimitExceeded);
            }
            let entry = entries.entry(entity).or_insert_with(|| EntityIndexEntry {
                entity,
                types: vec![],
                labels: vec![],
                identifiers: vec![],
                relations: vec![],
            });
            for type_id in types {
                charge(refs, &mut entry_bytes)?;
                let definition = type_definitions
                    .get(&type_id)
                    .ok_or(IndexError::DefinitionUnavailable)?;
                if definition.id != type_id || definition.deprecation.is_some() {
                    return Err(IndexError::DefinitionUnavailable);
                }
                // A dictionary definition is owner input too; do not hash an
                // unbounded display/parent payload outside the scan budget.
                charge(definition, &mut entry_bytes)?;
                entry.types.push(IndexedType {
                    type_id,
                    definition_digest: Digest::from_serializable(1, definition)
                        .map_err(|_| IndexError::InvalidInput)?,
                    evidence: refs.clone(),
                });
            }
            if let Some(rule) = rule {
                for argument in claim
                    .arguments
                    .iter()
                    .filter(|a| a.role == Some(rule.value_role))
                {
                    match (&rule.field, &argument.value) {
                        (IndexField::Label | IndexField::Identifier, Value::Text(value)) => {
                            if value.len() > MAX_TEXT {
                                return Err(IndexError::LimitExceeded);
                            }
                            charge(&(refs, value), &mut entry_bytes)?;
                            let fact = IndexedText {
                                value: value.clone(),
                                evidence: refs.clone(),
                            };
                            if rule.field == IndexField::Label {
                                entry.labels.push(fact)
                            } else {
                                entry.identifiers.push(fact)
                            }
                        }
                        (IndexField::Relation, Value::Entity(target)) => {
                            charge(refs, &mut entry_bytes)?;
                            entry.relations.push(IndexedRelation {
                                target: *target,
                                predicate: claim.predicate,
                                evidence: refs.clone(),
                            });
                        }
                        _ => return Err(IndexError::InvalidInput),
                    }
                }
            }
            if entry.types.len()
                + entry.labels.len()
                + entry.identifiers.len()
                + entry.relations.len()
                > MAX_FACTS
            {
                return Err(IndexError::LimitExceeded);
            }
        }
        bounded_size(&entries)?;
        Ok(Self {
            revision,
            entries,
            evaluated_at: now,
            rules_digest: Digest::from_serializable(1, rules)
                .map_err(|_| IndexError::InvalidInput)?,
        })
    }
    pub fn revision(&self) -> &RevisionRef {
        &self.revision
    }
    pub fn rules_digest(&self) -> Digest {
        self.rules_digest
    }
    pub fn evaluated_at(&self) -> i64 {
        self.evaluated_at
    }
    pub fn entries(&self) -> &BTreeMap<EntityId, EntityIndexEntry> {
        &self.entries
    }
    pub fn get(&self, entity: EntityId) -> Result<&EntityIndexEntry, IndexError> {
        self.entries
            .get(&entity)
            .ok_or(IndexError::EntityUnavailable)
    }
    /// Search returns every matching exact Entity up to the bound; overflow is
    /// explicit. It never silently chooses or merges a name match.
    pub fn search(
        &self,
        query: &str,
        maximum: usize,
    ) -> Result<Vec<&EntityIndexEntry>, IndexError> {
        if query.len() > MAX_TEXT || maximum == 0 || maximum > MAX_ENTITIES {
            return Err(IndexError::LimitExceeded);
        }
        let mut matches = vec![];
        for entry in self.entries.values() {
            if entry
                .labels
                .iter()
                .chain(&entry.identifiers)
                .any(|fact| fact.value.contains(query))
            {
                if matches.len() == maximum {
                    return Err(IndexError::LimitExceeded);
                }
                matches.push(entry);
            }
        }
        Ok(matches)
    }
}
fn active(claim: &Claim, now: i64) -> bool {
    claim.validity.valid_from.is_none_or(|t| now >= t)
        && claim.validity.valid_until.is_none_or(|t| now < t)
}
fn bounded_size(value: &impl Serialize) -> Result<(), IndexError> {
    let mut remaining = MAX_BYTES;
    charge(value, &mut remaining)
}
fn charge(value: &impl Serialize, remaining: &mut usize) -> Result<(), IndexError> {
    struct Budget<'a>(&'a mut usize);
    impl std::io::Write for Budget<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > *self.0 {
                return Err(std::io::Error::other("index capacity"));
            }
            *self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(remaining), value).map_err(|e| {
        if e.is_io() {
            IndexError::LimitExceeded
        } else {
            IndexError::InvalidInput
        }
    })
}
