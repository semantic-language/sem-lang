//! Curated Standard definitions. Distribution is not adoption or execution.
use sem_lang_core::{RoleId, TypeId};
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition, standard_term};
use sem_lang_vocab::*;

/// One provider-neutral inference operation. Runtime, model, truth and adoption
/// are not part of this definition. Consumers explicitly adopt the returned
/// module through the normal definitions lifecycle before binding execution.
pub fn inference() -> VocabularyModule {
    let term = standard_term("Infer").expect("Infer has an immutable Standard identity");
    VocabularyModule {
        schema_version: "0.10.0".into(),
        vocabulary_id: "sem-lang.standard.inference".into(),
        version: "0.10.0".into(),
        dependencies: vec![],
        terms: vec![TermDefinition {
            id: term, label: "Infer".into(),
            description: "Derive a non-authoritative candidate or observation from supplied content. The result is neither asserted true nor adopted by this operation.".into(),
            deprecation: None, replacement: None,
            semantic_definition: Some(SemanticDefinition::Primitive(PrimitiveSemantic::Relation)),
        }],
        roles: vec![RoleDefinition {
            id: RoleId::new_random(0x73656d696e666572, 1),
            label: "Content".into(), cardinality: Cardinality::ExactlyOne,
            description: "Explicit input from which a non-authoritative candidate is derived.".into(),
        }],
        types: vec![TypeDefinition {
            id: TypeId::new_random(0x73656d696e666572, 2), label: "Text".into(),
            description: "Textual content representation; not proof of meaning or truth.".into(),
            super_types: vec![], deprecation: None,
        }],
        lexical_bindings: vec![LexicalBinding {
            language_tag: "en".into(), surface_form: "infer".into(), normalized_form: "infer".into(),
            contribution_kind: ContributionKind::Predicative, target_term: term,
            grammatical_condition: None, semantic_type_condition: None, priority: 1,
            valid_from: None, valid_until: None,
            provenance: "sem-lang Standard definition".into(),
        }],
    }
}
