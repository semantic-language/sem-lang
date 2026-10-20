use super::*;

#[test]
fn declared_multiplicity_distinguishes_absence_many_ambiguity_bounds_and_correlated_tuples() {
    use sem_lang::current::ConstraintStatus;
    let cards = [
        Cardinality::Zero,
        Cardinality::ZeroOrOne,
        Cardinality::ExactlyOne,
        Cardinality::OneOrMany,
        Cardinality::ZeroOrMany,
        Cardinality::Bounded { min: 2, max: 3 },
    ];
    for card in cards {
        for count in 0..=4 {
            let mut definitions = module();
            definitions.roles[0].cardinality = card;
            let mut state = RuntimeState::new();
            let mut selected = scope();
            selected.evidence.clear();
            for i in 0..count.max(1) {
                let mut fact = claim(
                    i + 1,
                    selected.actor(),
                    &format!("target-{i}"),
                    Some("same-time"),
                );
                if count == 0 {
                    // Explicit absence is data; an absent claim is not zero.
                    fact.arguments[0].value = Value::Nil;
                }
                state.claims.insert(fact.id, fact.clone());
                selected.evidence.push(fact.id);
            }
            let current = engine(&state, definitions.clone());
            let output = current
                .process_observation(&observation("reserve this"), &selected)
                .unwrap();
            let result = match &output {
                ProcessingOutcome::Resolved(result)
                | ProcessingOutcome::NeedsResolution(result) => result,
            };
            let (min, max) = card.bounds();
            let accepted = count as usize >= min && max.is_none_or(|m| count as usize <= m);
            assert_eq!(
                matches!(output, ProcessingOutcome::Resolved(_)),
                accepted,
                "{card:?}, {count}"
            );
            let multiplicity = result
                .multiplicities
                .iter()
                .find(|m| m.variable == "target")
                .unwrap();
            assert_eq!(multiplicity.observed, count as usize);
            assert_eq!(multiplicity.minimum, min);
            assert_eq!(multiplicity.maximum, max);
            assert_eq!(
                multiplicity.status,
                if accepted {
                    ConstraintStatus::Satisfied
                } else {
                    ConstraintStatus::Unsatisfied
                }
            );
            assert!(
                result
                    .bindings
                    .iter()
                    .any(|b| b.variable.id == "datetime"
                        && b.value == Value::Text("same-time".into()))
            );
            if accepted {
                let actual = &result
                    .bindings
                    .iter()
                    .find(|b| b.variable.id == "target")
                    .unwrap()
                    .value;
                let expected: Vec<_> = (0..count)
                    .map(|i| Value::Text(format!("target-{i}")))
                    .collect();
                assert_eq!(
                    *actual,
                    if max == Some(1) && count == 1 {
                        expected[0].clone()
                    } else {
                        Value::Sequence(expected)
                    }
                );
            } else {
                let issue = result
                    .issues
                    .iter()
                    .find(|i| i.variable.as_deref() == Some("target"))
                    .unwrap();
                assert_eq!(
                    issue.reason,
                    if max == Some(1) && count > 1 {
                        Reason::Ambiguous
                    } else {
                        Reason::CardinalityViolation
                    }
                );
            }
            selected.evidence.reverse();
            assert_eq!(
                output,
                current
                    .process_observation(&observation("reserve this"), &selected)
                    .unwrap()
            );
            selected.evidence.clear();
            let ProcessingOutcome::NeedsResolution(missing) = current
                .process_observation(&observation("reserve this"), &selected)
                .unwrap()
            else {
                panic!("no evidence must not become explicit absence")
            };
            assert!(
                missing
                    .issues
                    .iter()
                    .any(|i| i.variable.as_deref() == Some("target")
                        && i.reason == Reason::MissingContext)
            );
        }
    }

    // Both many-valued slots are accepted, with their original pairing intact.
    let mut definitions = module();
    for role in &mut definitions.roles {
        role.cardinality = Cardinality::OneOrMany;
    }
    let selected = ObservationScope {
        evidence: vec![node(1), node(2)],
        ..scope()
    };
    let mut state = RuntimeState::new();
    state
        .claims
        .insert(node(1), claim(1, selected.actor(), "a", Some("b")));
    state
        .claims
        .insert(node(2), claim(2, selected.actor(), "c", Some("d")));
    let ProcessingOutcome::Resolved(result) = engine(&state, definitions)
        .process_observation(&observation("reserve this"), &selected)
        .unwrap()
    else {
        panic!("declared many")
    };
    assert_eq!(result.relations.len(), 1);
    let mut pairs: Vec<_> = result.relations[0]
        .rows
        .iter()
        .map(|row| {
            let get = |name| {
                row.iter()
                    .find(|b| b.variable.id == name)
                    .unwrap()
                    .value
                    .clone()
            };
            (get("target"), get("datetime"))
        })
        .collect();
    pairs.sort_by_cached_key(|pair| serde_json::to_vec(pair).unwrap());
    assert_eq!(
        pairs,
        vec![
            (Value::Text("a".into()), Value::Text("b".into())),
            (Value::Text("c".into()), Value::Text("d".into()))
        ]
    );
    // Invalid ranges fail adoption; they are not "unsupported" at runtime.
    let mut invalid = module();
    invalid.roles[0].cardinality = Cardinality::Bounded { min: 3, max: 2 };
    assert!(VocabularyRegistry::new().register(invalid).is_err());

    for card in [Cardinality::ExactlyOne, Cardinality::OneOrMany] {
        let mut definitions = module();
        definitions.roles[0].cardinality = card;
        let mut state = RuntimeState::new();
        let mut absent = claim(1, selected.actor(), "unused", Some("same-time"));
        absent.arguments[0].value = Value::Nil;
        state.claims.insert(node(1), absent);
        state.claims.insert(
            node(2),
            claim(2, selected.actor(), "present", Some("same-time")),
        );
        let ProcessingOutcome::NeedsResolution(result) = engine(&state, definitions)
            .process_observation(&observation("reserve this"), &selected)
            .unwrap()
        else {
            panic!("explicit absence and presence contradict each other")
        };
        assert!(result.bindings.iter().all(|b| b.variable.id != "target"));
        assert!(result.bindings.iter().any(|b| b.variable.id == "datetime"));
        let issue = result
            .issues
            .iter()
            .find(|i| i.variable.as_deref() == Some("target"))
            .unwrap();
        assert_eq!(issue.reason, Reason::ConflictingEvidence);
        assert_eq!(issue.candidates, vec![Value::Text("present".into())]);
        assert_eq!(result.relations[0].rows.len(), 2);
    }
}
