use super::*;

#[test]
fn declared_surface_constructions_cover_input_and_keep_incomplete_syntax_unresolved() {
    let selected = scope();
    let mut state = RuntimeState::new();
    state.claims.insert(
        node(1),
        claim(1, selected.actor(), "clinic", Some("morning")),
    );
    let mut definitions = module();
    definitions.lexical_bindings[0].grammatical_condition = Some(
        serde_json::from_value(serde_json::json!({
            "Sequence":[{"Optional":{"Text":"please "}},{"Text":"reserve this"}]
        }))
        .unwrap(),
    );
    let current = engine(&state, definitions.clone());
    for surface in ["please reserve this", "reserve this"] {
        let ProcessingOutcome::Resolved(result) = current
            .process_observation(&observation(surface), &selected)
            .unwrap()
        else {
            panic!("declared syntax covers {surface}")
        };
        assert_eq!(result.bindings.len(), 2);
        assert!(result.issues.is_empty());
    }
    let ProcessingOutcome::NeedsResolution(rejected) = current
        .process_observation(&observation("never reserve this"), &selected)
        .unwrap()
    else {
        panic!("undeclared syntax")
    };
    assert!(rejected.bindings.is_empty());
    definitions.lexical_bindings[0].grammatical_condition = Some(serde_json::from_value(serde_json::json!({
        "Sequence":[{"Optional":{"Text":"please "}},{"Text":"reserve this"},{"Text":" at clinic"}]
    })).unwrap());
    let current = engine(&state, definitions.clone());
    let before = current
        .current_compiler()
        .unwrap()
        .semantic_revision()
        .unwrap();
    let partial = current
        .process_observation(&observation("please reserve this"), &selected)
        .unwrap();
    let ProcessingOutcome::NeedsResolution(result) = &partial else {
        panic!("missing syntax stays unresolved")
    };
    assert_eq!(result.bindings.len(), 2);
    assert!(
        result
            .issues
            .iter()
            .any(|i| i.reason == Reason::UnknownExpression)
    );
    assert_eq!(
        partial,
        current
            .process_observation(&observation("please reserve this"), &selected)
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
    let mut alternatives = definitions.clone();
    let mut complete = alternatives.lexical_bindings[0].clone();
    complete.priority = 0;
    complete.grammatical_condition = Some(sem_lang_linguistics::SurfacePattern::Text(
        "please reserve this".into(),
    ));
    alternatives.lexical_bindings.push(complete.clone());
    assert!(
        matches!(
            engine(&state, alternatives.clone())
                .process_observation(&observation("please reserve this"), &selected)
                .unwrap(),
            ProcessingOutcome::Resolved(_)
        ),
        "independent complete declaration supplies the same meaning"
    );
    complete.target_term = term(3);
    alternatives.lexical_bindings.push(complete);
    let ProcessingOutcome::NeedsResolution(ambiguous) = engine(&state, alternatives)
        .process_observation(&observation("please reserve this"), &selected)
        .unwrap()
    else {
        panic!("grammatical applicability is not authority to choose between meanings")
    };
    assert!(
        ambiguous
            .issues
            .iter()
            .any(|issue| issue.reason == Reason::Ambiguous)
    );
    let raw = serde_json::to_value(&definitions).unwrap();
    let mut old = raw;
    old["lexical_bindings"][0]["grammatical_condition"] = serde_json::json!("Create Request");
    assert!(serde_json::from_value::<VocabularyModule>(old).is_err());
}
