use super::*;
use sem_lang_core::{Constraint, Digest, Term, TypeId};

#[test]
fn senses_are_declared_index_members_for_every_locale_and_missing_entries_fail() {
    use sem_lang_language::SemanticCompileError;
    for tag in ["en", "ja-JP,en", "*"] {
        let mut definitions = module();
        definitions.lexical_bindings[0].language_tag = tag.into();
        let current = engine(&RuntimeState::new(), definitions);
        for locale in tag.split(',').map(|t| if t == "*" { "fr" } else { t }) {
            let mut language = current.compiled_language().clone();
            let first = language.compile_semantics("reserve this", locale).unwrap();
            assert_eq!(first.sense.lexeme, first.lexeme.id);
            assert!(
                language
                    .resources
                    .senses
                    .senses
                    .iter()
                    .any(|s| s.id == first.sense.id
                        && s.lexeme == first.lexeme.id
                        && s.semantic_target == term(1))
            );
            language.resources.senses.senses.reverse();
            assert_eq!(
                language
                    .compile_semantics("reserve this", locale)
                    .unwrap()
                    .sense
                    .id,
                first.sense.id
            );
            let duplicate = first.sense.clone();
            language.resources.senses.senses.push(duplicate);
            assert!(matches!(
                language.compile_semantics("reserve this", locale),
                Err(SemanticCompileError::InvalidSenseReference)
            ));
            language.resources.senses.senses.clear();
            assert!(matches!(
                language.compile_semantics("reserve this", locale),
                Err(SemanticCompileError::InvalidSenseReference)
            ));
            let mut corrupt = first.sense;
            corrupt.semantic_target = term(3);
            language.resources.senses.senses.push(corrupt);
            assert!(matches!(
                language.compile_semantics("reserve this", locale),
                Err(SemanticCompileError::InvalidSenseReference)
            ));
        }
    }
}

#[test]
fn lexical_type_applicability_uses_explicit_current_types_and_preserves_unknown_conditions() {
    let a = TypeId::new_random(111, 1);
    let child = TypeId::new_random(112, 1);
    let b = TypeId::new_random(113, 1);
    let mut definitions = module();
    definitions.types = [(a, vec![]), (child, vec![a]), (b, vec![])]
        .into_iter()
        .map(|(id, super_types)| TypeDefinition {
            id,
            label: id.to_hex(),
            super_types,
            description: String::new(),
            deprecation: None,
        })
        .collect();
    definitions.lexical_bindings[0].semantic_type_condition = Some(a);
    let selected = scope();
    let mut state = RuntimeState::new();
    state.claims.insert(
        node(1),
        claim(1, selected.actor(), "clinic", Some("morning")),
    );
    let current = engine(&state, definitions.clone());
    let ProcessingOutcome::NeedsResolution(unknown) = current
        .process_observation(&observation("reserve this"), &selected)
        .unwrap()
    else {
        panic!("absent explicit type must be unknown")
    };
    assert_eq!(unknown.bindings.len(), 2);
    assert!(unknown.expression.is_some());
    let mut independently_applicable = definitions.clone();
    let mut unconditional = independently_applicable.lexical_bindings[0].clone();
    unconditional.semantic_type_condition = None;
    unconditional.priority = 0;
    independently_applicable
        .lexical_bindings
        .push(unconditional);
    assert!(matches!(
        engine(&state, independently_applicable)
            .process_observation(&observation("reserve this"), &selected)
            .unwrap(),
        ProcessingOutcome::Resolved(_)
    ));
    assert!(
        unknown
            .issues
            .iter()
            .any(|issue| issue.reason == Reason::MissingContext
                && issue.constraint.as_ref().is_some_and(|c| c.value()
                    == &Constraint::TypeConstraint {
                        value: Value::TermRef(term(1)),
                        ty: a
                    }))
    );
    state.terms.insert(
        term(1),
        Term {
            id: term(1),
            name: "display only".into(),
            type_id: child,
            digest: Digest::object_digest(b"fixture"),
            deprecation: false,
        },
    );
    let current = engine(&state, definitions.clone());
    let before = current
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let complete = current
        .process_observation(&observation("reserve this"), &selected)
        .unwrap();
    assert!(matches!(complete, ProcessingOutcome::Resolved(_)));
    assert_eq!(
        complete,
        current
            .process_observation(&observation("reserve this"), &selected)
            .unwrap()
    );
    assert_eq!(
        before,
        current
            .current_compiler()
            .unwrap()
            .semantic_revision()
            .unwrap()
    );
    state.terms.get_mut(&term(1)).unwrap().type_id = b;
    let ProcessingOutcome::NeedsResolution(rejected) = engine(&state, definitions.clone())
        .process_observation(&observation("reserve this"), &selected)
        .unwrap()
    else {
        panic!("wrong type cannot satisfy binding")
    };
    assert!(rejected.bindings.is_empty());
    state.terms.get_mut(&term(1)).unwrap().id = term(500);
    assert_eq!(
        rejected_checkpoint(&state, definitions.clone()),
        sem_lang::restoration::SemanticReferenceError::UnknownTerm(term(500))
    );
    state.terms.get_mut(&term(1)).unwrap().id = term(1);
    state.terms.get_mut(&term(1)).unwrap().type_id = TypeId::new_random(999, 1);
    assert_eq!(
        rejected_checkpoint(&state, definitions.clone()),
        sem_lang::restoration::SemanticReferenceError::UnknownType(TypeId::new_random(999, 1))
    );

    state.terms.get_mut(&term(1)).unwrap().type_id = a;
    let mut alternative = definitions.lexical_bindings[0].clone();
    alternative.target_term = term(3);
    alternative.semantic_type_condition = Some(b);
    definitions.lexical_bindings.push(alternative);
    state.terms.insert(
        term(3),
        Term {
            id: term(3),
            name: "same display".into(),
            type_id: a,
            digest: Digest::object_digest(b"other"),
            deprecation: false,
        },
    );
    assert!(matches!(
        engine(&state, definitions.clone())
            .process_observation(&observation("reserve this"), &selected)
            .unwrap(),
        ProcessingOutcome::Resolved(_)
    ));
    for actual in [Some(b), None] {
        if let Some(ty) = actual {
            state.terms.get_mut(&term(3)).unwrap().type_id = ty;
        } else {
            state.terms.remove(&term(3));
        }
        let ProcessingOutcome::NeedsResolution(ambiguous) = engine(&state, definitions.clone())
            .process_observation(&observation("reserve this"), &selected)
            .unwrap()
        else {
            panic!("possible competing meaning cannot be discarded")
        };
        assert_eq!(ambiguous.issues[0].reason, Reason::Ambiguous);
        assert_eq!(ambiguous.issues[0].candidates.len(), 2);
    }
}
