//! Compiler-derived identities for the language-independent semantic basis.
//!
//! These names are the compiler bootstrap index for the bundled
//! `sem-lang/standard/semantic` source. The standard package verifies that its
//! complete term set is identical to this index. Hexadecimal `TermId` values
//! never belong in authored `.sem` source.

use sem_lang_core::TermId;

pub const STANDARD_SEMANTIC_OWNER: &str = "sem-lang/standard/semantic";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredSemanticTerm {
    pub canonical_label: &'static str,
}

impl DeclaredSemanticTerm {
    pub fn term_id(self) -> TermId {
        TermId::from_declaration(STANDARD_SEMANTIC_OWNER, self.canonical_label)
    }
}

pub const SEMANTIC_BASIS: &[DeclaredSemanticTerm] = &[
    term("Value"),
    term("Type"),
    term("Predicate"),
    term("Relation"),
    term("Role"),
    term("Proposition"),
    term("Reference"),
    term("Variable"),
    term("Constraint"),
    term("Composition"),
    term("Quantification"),
    term("Polarity"),
    term("Modality"),
    term("Temporal"),
    term("Spatial"),
    term("Causality"),
];

pub const STANDARD_SEMANTIC: &[DeclaredSemanticTerm] = &[
    term("Entity"),
    term("State"),
    term("Event"),
    term("Action"),
    term("Agent"),
    term("Change"),
    term("Intent"),
    term("Plan"),
    term("Commitment"),
    term("Possession"),
    term("Existence"),
    term("Identity"),
    term("Part"),
    term("Set"),
    term("Time"),
    term("Place"),
    term("Tomorrow"),
    term("Near"),
    term("Create"),
    term("Update"),
    term("Delete"),
    term("Infer"),
    term("Assertion"),
    term("Question"),
    term("Request"),
    term("Proposal"),
    term("Report"),
    term("Correction"),
    term("Confirmation"),
    term("Rejection"),
    term("Delegation"),
    term("BindingRequest"),
    term("Bind"),
    term("Cancel"),
];

pub const fn term(canonical_label: &'static str) -> DeclaredSemanticTerm {
    DeclaredSemanticTerm { canonical_label }
}

pub fn standard_term(canonical_label: &str) -> Option<TermId> {
    SEMANTIC_BASIS
        .iter()
        .chain(STANDARD_SEMANTIC)
        .find(|entry| entry.canonical_label == canonical_label)
        .copied()
        .map(DeclaredSemanticTerm::term_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_derived_from_the_standard_package_and_canonical_name() {
        let entity = STANDARD_SEMANTIC
            .iter()
            .find(|entry| entry.canonical_label == "Entity")
            .unwrap();
        assert_eq!(
            entity.term_id(),
            TermId::from_declaration(STANDARD_SEMANTIC_OWNER, "Entity")
        );
    }

    #[test]
    fn declared_ids_are_unique_and_valid() {
        let mut ids = std::collections::BTreeSet::new();
        for entry in SEMANTIC_BASIS.iter().chain(STANDARD_SEMANTIC) {
            assert!(ids.insert(entry.term_id()));
        }
    }
}
