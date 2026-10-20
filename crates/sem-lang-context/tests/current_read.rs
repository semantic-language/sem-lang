#[path = "../../../test-support/context.rs"]
mod context_fixture;
use context_fixture::ContextFixture;
use sem_lang_context::*;
mod observation_fixture;

fn reference_matrix() {
    use ContextScope::*;
    let subject = ContextSubject {
        id: "owner".into(),
        class: "person".into(),
    };
    let kinds = [
        ContextKind::TimeZone,
        ContextKind::ReferenceDate,
        ContextKind::Location,
        ContextKind::Calendar,
        ContextKind::Agent,
        ContextKind::Capability,
    ];
    let scopes = [
        CurrentExpression,
        CurrentRequest,
        Conversation,
        Session,
        Task,
        Agent,
        Environment,
        PersistentAgentDefault,
        DomainSpecific,
    ];
    // Independent expectation table, not a call to the implementation's permits().
    let permitted = [1u16, 3, 7, 15, 19, 191, 64, 191, 256];
    let mut engine = ContextEngine::new(ContextStore::default()).with_subject(subject.clone());
    engine
        .store
        .persistent_defaults
        .push(PersistentContextualDefault {
            id: "exact".into(),
            subject: subject.clone(),
            kind: kinds[0],
            value: "declared-value".into(),
            scope: CurrentExpression,
            validity: ContextValidity::default(),
            evidence: vec![],
            revision: "revision:1".into(),
            promotion_ref: "decision:1".into(),
        });
    let refs = [ContextReference::Default("exact".into())];
    for kind in kinds {
        engine.store.persistent_defaults[0].kind = kind;
        for (from, supplied) in scopes.iter().enumerate() {
            engine.store.persistent_defaults[0].scope = *supplied;
            let before = engine.memory_revision().unwrap();
            for (to, requested) in scopes.iter().enumerate() {
                let read = engine
                    .read_current(&refs, &subject, *requested, Some(100))
                    .unwrap();
                let allowed = permitted[from] & (1 << to) != 0;
                assert_eq!(read.entries[0].kind, Some(kind));
                assert_eq!(
                    read.entries[0].status,
                    if allowed {
                        ContextStatus::Applicable
                    } else {
                        ContextStatus::OutOfScope
                    }
                );
                for selected in kinds {
                    assert_eq!(
                        read.select(selected, &[]).values,
                        if allowed && selected == kind {
                            vec!["declared-value"]
                        } else {
                            vec![]
                        }
                    );
                }
            }
            assert_eq!(engine.memory_revision().unwrap(), before);
        }
    }
    engine.store.persistent_defaults[0].scope = CurrentRequest;
    engine.store.persistent_defaults[0].validity = ContextValidity {
        valid_from: Some(100),
        valid_until: Some(200),
        superseded_by: None,
    };
    for (at, expected) in [
        (99, ContextStatus::Expired),
        (100, ContextStatus::Applicable),
        (199, ContextStatus::Applicable),
        (200, ContextStatus::Expired),
        (201, ContextStatus::Expired),
    ] {
        assert_eq!(
            engine
                .read_current(&refs, &subject, CurrentRequest, Some(at))
                .unwrap()
                .entries[0]
                .status,
            expected
        );
    }
    assert!(matches!(
        engine.read_current(&refs, &subject, CurrentRequest, None),
        Err(CurrentReadError::MissingInstant)
    ));
    engine.store.persistent_defaults[0].validity.superseded_by = Some("replacement".into());
    assert_eq!(
        engine
            .read_current(&refs, &subject, CurrentRequest, None)
            .unwrap()
            .entries[0]
            .status,
        ContextStatus::Expired
    );
    engine.record_timezone_confirmation("UTC");
    engine.record_timezone_confirmation("UTC");
    let candidate = engine.induce_defaults().remove(0);
    let id = candidate.reference().unwrap();
    let decision_refs = [ContextReference::Decision(id.clone())];
    for (disposition, expected) in [
        (CandidateDisposition::Pending, ContextStatus::Candidate),
        (
            CandidateDisposition::SelectedShortTerm,
            ContextStatus::Applicable,
        ),
        (CandidateDisposition::Promoted, ContextStatus::Promoted),
        (CandidateDisposition::Rejected, ContextStatus::Rejected),
        (CandidateDisposition::Replaced, ContextStatus::Replaced),
    ] {
        engine.store.decided_candidates.insert(
            id.clone(),
            DecidedCandidate {
                subject_ref: subject.clone(),
                candidate: candidate.clone(),
                disposition,
                replacement_ref: None,
            },
        );
        let read = engine
            .read_current(&decision_refs, &subject, CurrentRequest, Some(100))
            .unwrap();
        assert_eq!(read.entries[0].status, expected);
        assert_eq!(
            read.select(ContextKind::TimeZone, &[]).values,
            if expected == ContextStatus::Applicable {
                vec!["UTC"]
            } else {
                vec![]
            }
        );
    }
    engine.bind_explicit_timezone("UTC", Session);
    engine.bind_explicit_timezone("Asia/Tokyo", CurrentRequest);
    let mut input = observation_fixture::request(
        "matrix:1",
        "Unresolved text must never be accepted as a default",
    );
    input.context.subject = subject.clone();
    let observation = engine.retain_observation(input).unwrap();
    let all = [
        ContextReference::Active,
        ContextReference::Session,
        refs[0].clone(),
        decision_refs[0].clone(),
        ContextReference::Observation(observation.reference),
    ];
    for reference in all {
        let before = engine.memory_revision().unwrap();
        let references = [reference.clone()];
        assert!(
            engine
                .read_current(&references, &subject, CurrentRequest, Some(100))
                .is_ok()
        );
        for other in [
            ContextSubject {
                id: "other".into(),
                ..subject.clone()
            },
            ContextSubject {
                class: "other".into(),
                ..subject.clone()
            },
        ] {
            assert!(matches!(
                engine.read_current(&references, &other, CurrentRequest, Some(100)),
                Err(CurrentReadError::InvalidSubject)
            ));
        }
        let duplicates = [reference.clone(), reference];
        assert!(matches!(
            engine.read_current(&duplicates, &subject, CurrentRequest, Some(100)),
            Err(CurrentReadError::InvalidReference)
        ));
        assert_eq!(engine.memory_revision().unwrap(), before);
    }
    let empty = ContextEngine::new(ContextStore::default()).with_subject(subject.clone());
    for missing in [
        ContextReference::Active,
        ContextReference::Session,
        ContextReference::Default("missing".into()),
        ContextReference::Decision("missing".into()),
        ContextReference::Observation("0".repeat(64)),
    ] {
        assert!(matches!(
            empty.read_current(&[missing], &subject, CurrentRequest, Some(100)),
            Err(CurrentReadError::InvalidReference)
        ));
    }
}

#[test]
fn current_read_preserves_precedence_conflicts_expiry_subject_and_candidate_refs() {
    reference_matrix();
    let subject = ContextSubject {
        id: "person:a".into(),
        class: "person".into(),
    };
    let mut engine = ContextEngine::new(ContextStore::default()).with_subject(subject.clone());
    engine
        .store
        .persistent_defaults
        .push(PersistentContextualDefault {
            id: "default:a".into(),
            subject: subject.clone(),
            kind: ContextKind::TimeZone,
            value: "UTC".into(),
            scope: ContextScope::PersistentAgentDefault,
            validity: ContextValidity::default(),
            evidence: vec![],
            revision: "revision:a".into(),
            promotion_ref: "decision:a".into(),
        });
    let mut expired = engine.store.persistent_defaults[0].clone();
    expired.id = "expired".into();
    expired.value = "expired-value".into();
    expired.validity.valid_until = Some(100);
    engine.store.persistent_defaults.push(expired);
    engine.record_timezone_confirmation("Asia/Tokyo");
    engine.record_timezone_confirmation("Asia/Tokyo");
    let candidate = engine.induce_defaults().remove(0);
    let reference = candidate.reference().unwrap();
    engine
        .promote(
            candidate,
            &PromotionRequest {
                subject_ref: subject.clone(),
                candidate_ref: reference.clone(),
                expected_memory_revision: engine.memory_revision().unwrap(),
                expected_commit_revision: engine.commit_revision().unwrap(),
                decision_ref: "select:a".into(),
                operation: PromotionOperation::SelectShortTerm,
            },
        )
        .unwrap();
    let refs = vec![
        ContextReference::Default("default:a".into()),
        ContextReference::Default("expired".into()),
        ContextReference::Decision(reference),
    ];
    let read = engine
        .read_current(&refs, &subject, ContextScope::CurrentRequest, Some(100))
        .unwrap();
    let choice = read.select(ContextKind::TimeZone, &[]);
    assert_eq!(choice.values, vec!["Asia/Tokyo"]);
    assert_eq!(
        read.entries
            .iter()
            .filter(|e| e.status == ContextStatus::Expired)
            .count(),
        1
    );
    assert_eq!(read.entries.len(), 3);
    let explicit = ContextEvidence {
        value: "explicit",
        authority: ContextAuthority::Explicit,
    };
    assert_eq!(
        read.select(ContextKind::TimeZone, &[explicit]).values,
        vec!["explicit"]
    );
    let accepted = ContextEvidence {
        value: "long-term",
        authority: ContextAuthority::AcceptedAssertion,
    };
    assert_eq!(
        read.select(ContextKind::TimeZone, &[accepted]).values,
        vec!["Asia/Tokyo"]
    );
    let clash = [
        ContextEvidence {
            value: "A",
            authority: ContextAuthority::ActiveContext,
        },
        ContextEvidence {
            value: "B",
            authority: ContextAuthority::ActiveContext,
        },
    ];
    assert_eq!(
        read.select(ContextKind::TimeZone, &clash).values,
        vec!["A", "Asia/Tokyo", "B"]
    );
    assert!(read.select(ContextKind::TimeZone, &clash).conflicting());
    let other = ContextSubject {
        id: "person:b".into(),
        class: "person".into(),
    };
    assert!(
        engine
            .read_current(&refs, &other, ContextScope::CurrentRequest, Some(100))
            .is_err()
    );
    assert!(
        engine
            .read_current(&refs, &subject, ContextScope::CurrentRequest, None)
            .is_err()
    );
    let default_only = engine
        .read_current(
            &refs[..2],
            &subject,
            ContextScope::CurrentRequest,
            Some(100),
        )
        .unwrap();
    assert_eq!(
        default_only
            .select(ContextKind::TimeZone, &[accepted])
            .values,
        vec!["long-term"]
    );
    assert_eq!(
        default_only.select(ContextKind::TimeZone, &[]).values,
        vec!["UTC"]
    );
    let revision = engine.memory_revision().unwrap();
    let restored = ContextEngine::new(
        serde_json::from_value(serde_json::to_value(&engine.store).unwrap()).unwrap(),
    )
    .with_subject(subject.clone());
    assert_eq!(restored.memory_revision().unwrap(), revision);
    assert_eq!(
        restored
            .read_current(&refs, &subject, ContextScope::CurrentRequest, Some(100))
            .unwrap()
            .select(ContextKind::TimeZone, &[])
            .values,
        choice.values
    );

    // Pending candidates and residuals are visible references, never accepted facts.
    let mut pending = engine
        .store
        .decided_candidates
        .values()
        .next()
        .unwrap()
        .candidate
        .clone();
    pending.value = "Europe/Paris".into();
    let pending_ref = pending.reference().unwrap();
    engine
        .promote(
            pending,
            &PromotionRequest {
                subject_ref: subject.clone(),
                candidate_ref: pending_ref.clone(),
                expected_memory_revision: engine.memory_revision().unwrap(),
                expected_commit_revision: engine.commit_revision().unwrap(),
                decision_ref: "retain:b".into(),
                operation: PromotionOperation::Retain,
            },
        )
        .unwrap();
    let mut input =
        observation_fixture::request("observed:1", "unresolved observation, not an inferred time");
    input.context.subject = subject.clone();
    let observation = engine.retain_observation(input).unwrap();
    let view_refs = [
        ContextReference::Decision(pending_ref),
        ContextReference::Observation(observation.reference.clone()),
    ];
    let before = engine.memory_revision().unwrap();
    let view = engine
        .read_current(
            &view_refs,
            &subject,
            ContextScope::CurrentRequest,
            Some(100),
        )
        .unwrap();
    assert_eq!(view.entries[0].status, ContextStatus::Candidate);
    assert_eq!(view.entries[0].value, Some("Europe/Paris"));
    assert_eq!(view.entries[1].status, ContextStatus::Observed);
    assert!(view.entries[1].value.is_none());
    assert!(view.select(ContextKind::TimeZone, &[]).values.is_empty());
    assert_eq!(engine.memory_revision().unwrap(), before);
    assert!(matches!(
        engine.read_current(&view_refs, &other, ContextScope::CurrentRequest, Some(100)),
        Err(CurrentReadError::InvalidSubject)
    ));
    let out = engine
        .read_current(&refs, &subject, ContextScope::Environment, Some(100))
        .unwrap();
    assert!(out.select(ContextKind::TimeZone, &[]).values.is_empty());
    assert!(
        out.entries
            .iter()
            .any(|entry| entry.status == ContextStatus::OutOfScope)
    );
    let invalid = [ContextReference::Observation("0".repeat(64))];
    assert!(matches!(
        engine.read_current(&invalid, &subject, ContextScope::CurrentRequest, Some(100)),
        Err(CurrentReadError::InvalidReference)
    ));
    let duplicate = [
        ContextReference::Observation(observation.reference.clone()),
        ContextReference::Observation(observation.reference),
    ];
    assert!(matches!(
        engine.read_current(
            &duplicate,
            &subject,
            ContextScope::CurrentRequest,
            Some(100)
        ),
        Err(CurrentReadError::InvalidReference)
    ));
}
