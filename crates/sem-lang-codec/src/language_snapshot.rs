//! Immutable, domain-neutral language definitions for external consumers.
//! No application UI, permission, installation or person's state is encoded here.
use sem_lang_linguistics::{semantic_basis_vocabulary, standard_semantic_vocabulary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageTermDto {
    pub reference: String,
    pub symbol: String,
    pub layer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageSnapshotDto {
    pub schema: String,
    pub owner: String,
    pub version: String,
    pub digest: String,
    pub terms: Vec<LanguageTermDto>,
}

impl LanguageSnapshotDto {
    /// Export the exact curated basis and standard definitions, not learned candidates.
    #[must_use]
    pub fn standard() -> Self {
        let mut terms = Vec::new();
        for (layer, vocabulary) in [
            ("semantic-basis", semantic_basis_vocabulary()),
            ("standard-semantic", standard_semantic_vocabulary()),
        ] {
            terms.extend(
                vocabulary
                    .concepts
                    .into_iter()
                    .map(|(symbol, id)| LanguageTermDto {
                        reference: format!("sem-lang://concept/{id}"),
                        symbol,
                        layer: layer.into(),
                    }),
            );
        }
        terms.sort_by(|left, right| left.reference.cmp(&right.reference));
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&terms).expect("language term DTO serialization"),
        ));
        Self {
            schema: "sem-lang://wire/language-snapshot/v1".into(),
            owner: "sem-lang".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            digest,
            terms,
        }
    }

    /// Exact curated symbol lookup. This does not resolve natural-language aliases.
    #[must_use]
    pub fn term(&self, symbol: &str) -> Option<&LanguageTermDto> {
        self.terms.iter().find(|term| term.symbol == symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exported_language_is_exact_stable_and_domain_neutral() {
        let snapshot = LanguageSnapshotDto::standard();
        assert_eq!(snapshot, LanguageSnapshotDto::standard());
        assert_eq!(snapshot.version, "0.10.0");
        assert_eq!(
            snapshot.terms.len(),
            semantic_basis_vocabulary().concepts.len()
                + standard_semantic_vocabulary().concepts.len()
        );
        for (symbol, id) in standard_semantic_vocabulary().concepts {
            assert_eq!(
                snapshot.term(&symbol).unwrap().reference,
                format!("sem-lang://concept/{id}")
            );
        }
        assert!(snapshot.term("GitHub").is_none());
        assert!(snapshot.term("identity").is_none());
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_slice::<LanguageSnapshotDto>(&bytes).unwrap(),
            snapshot
        );
    }
}
