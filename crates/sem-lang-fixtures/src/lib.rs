//! Domain vocabulary used only by integration fixtures.
pub mod domain {
    use sem_lang_core::{RoleId, TermId, TypeId};
    use sem_lang_vocab::*;

    pub fn reservation_fixture_module() -> VocabularyModule {
        let t_request = TermId::new_random(1, 1);
        let t_question = TermId::new_random(2, 1);
        let t_assertion = TermId::new_random(3, 1);
        let t_reservation = TermId::new_random(4, 1);
        let t_time = TermId::new_random(5, 1);
        let t_place = TermId::new_random(6, 1);
        let t_station = TermId::new_random(7, 1);
        let t_create = TermId::new_random(8, 1);
        let t_cancel = TermId::new_random(9, 1);
        let t_confirm = TermId::new_random(10, 1);
        let t_near = TermId::new_random(11, 1);
        let t_binding_request = TermId::new_random(12, 1);
        let t_bind = TermId::new_random(13, 1);
        let t_proposal = TermId::new_random(14, 1);
        let t_pragmatic = TermId::new_random(15, 1);
        let t_identity = TermId::new_random(16, 1);

        let operation_type = TypeId::new_random(50, 2);
        let entity_type = TypeId::new_random(51, 2);
        let value_type = TypeId::new_random(52, 2);

        let actor_role = RoleId::new_random(80, 3);
        let theme_role = RoleId::new_random(81, 3);
        let time_role = RoleId::new_random(82, 3);
        let place_role = RoleId::new_random(83, 3);

        let mut lex = vec![];
        for (term, form, kind) in [
            (t_request, "予約", ContributionKind::Predicative),
            (t_create, "作る", ContributionKind::Operational),
            (t_create, "作って", ContributionKind::Operational),
            (t_station, "駅", ContributionKind::Denotational),
            (t_place, "場所", ContributionKind::Denotational),
            (t_confirm, "確認", ContributionKind::Pragmatic),
            (t_cancel, "取消", ContributionKind::Operational),
            (t_bind, "束縛", ContributionKind::Operational),
            (t_reservation, "予約", ContributionKind::Denotational),
            (t_near, "近く", ContributionKind::Relational),
            (t_time, "明日", ContributionKind::Temporal),
            (t_pragmatic, "ください", ContributionKind::Pragmatic),
            (t_pragmatic, "お願いします", ContributionKind::Pragmatic),
            (t_pragmatic, "please", ContributionKind::Pragmatic),
        ] {
            lex.push(LexicalBinding {
                language_tag: "ja-JP,en-US".to_string(),
                surface_form: form.to_string(),
                normalized_form: form.to_string(),
                contribution_kind: kind,
                target_term: term,
                grammatical_condition: None,
                semantic_type_condition: Some(entity_type),
                priority: 10,
                valid_from: None,
                valid_until: None,
                provenance: "embedded-core".to_string(),
            })
        }

        VocabularyModule {
            schema_version: "0.10.0".to_string(),
            vocabulary_id: "core".to_string(),
            version: "0.10.0".to_string(),
            dependencies: vec![],
            terms: vec![
                TermDefinition {
                    id: t_request,
                    label: "Request".to_string(),
                    description: "Request act".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_question,
                    label: "Question".to_string(),
                    description: "Question act".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_assertion,
                    label: "Assertion".to_string(),
                    description: "Assertion act".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_reservation,
                    label: "Reservation".to_string(),
                    description: "Reservation entity".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: Some(
                        sem_lang_linguistics::SemanticDefinition::Composition(
                            sem_lang_linguistics::SemanticExpr::Conjunction(vec![
                                sem_lang_linguistics::SemanticExpr::Apply {
                                    predicate: TermId::from_hex("4275f2c5459d0cb0d8ac0705fe5b8803")
                                        .expect("fixture term id"),
                                    arguments: vec![(
                                        theme_role,
                                        Box::new(sem_lang_linguistics::SemanticExpr::Variable(
                                            "R".to_string(),
                                        )),
                                    )],
                                },
                                sem_lang_linguistics::SemanticExpr::Apply {
                                    predicate: sem_lang_linguistics::standard_term("Commitment")
                                        .expect("Standard Commitment identity"),
                                    arguments: vec![(
                                        theme_role,
                                        Box::new(sem_lang_linguistics::SemanticExpr::Variable(
                                            "R".to_string(),
                                        )),
                                    )],
                                },
                                sem_lang_linguistics::SemanticExpr::Residual(
                                    sem_lang_linguistics::SemanticResidual {
                                        source_term: t_reservation,
                                        reason:
                                            sem_lang_linguistics::ResidualReason::DomainSpecific(
                                                "ReservationSpecificSemantics".to_string(),
                                            ),
                                    },
                                ),
                            ]),
                        ),
                    ),
                },
                TermDefinition {
                    id: t_time,
                    label: "Time".to_string(),
                    description: "Time".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_place,
                    label: "Place".to_string(),
                    description: "Place".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_station,
                    label: "Station".to_string(),
                    description: "Station entity".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_create,
                    label: "Create".to_string(),
                    description: "Create operation".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_cancel,
                    label: "Cancel".to_string(),
                    description: "Cancel operation".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_tomorrow(),
                    label: "Tomorrow".to_string(),
                    description: "Relative time".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_near,
                    label: "Near".to_string(),
                    description: "Spatial relation".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_confirm,
                    label: "Confirmation".to_string(),
                    description: "Confirmation act".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_binding_request,
                    label: "BindingRequest".to_string(),
                    description: "Request a binding".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_bind,
                    label: "Bind".to_string(),
                    description: "Bind variable".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_proposal,
                    label: "Proposal".to_string(),
                    description: "Proposal".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_pragmatic,
                    label: "Politeness".to_string(),
                    description: "Pragmatic marker".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
                TermDefinition {
                    id: t_identity,
                    label: "Identity".to_string(),
                    description: "Identity primitive".to_string(),
                    deprecation: None,
                    replacement: None,
                    semantic_definition: None,
                },
            ],
            types: vec![
                TypeDefinition {
                    id: operation_type,
                    label: "Operation".to_string(),
                    super_types: vec![],
                    description: "Operation type".to_string(),
                    deprecation: None,
                },
                TypeDefinition {
                    id: entity_type,
                    label: "Entity".to_string(),
                    super_types: vec![],
                    description: "Entity type".to_string(),
                    deprecation: None,
                },
                TypeDefinition {
                    id: value_type,
                    label: "Value".to_string(),
                    super_types: vec![],
                    description: "Value type".to_string(),
                    deprecation: None,
                },
            ],
            roles: vec![
                RoleDefinition {
                    id: actor_role,
                    label: "Actor".to_string(),
                    cardinality: Cardinality::ExactlyOne,
                    description: "Actor role".to_string(),
                },
                RoleDefinition {
                    id: theme_role,
                    label: "Theme".to_string(),
                    cardinality: Cardinality::ExactlyOne,
                    description: "Theme role".to_string(),
                },
                RoleDefinition {
                    id: time_role,
                    label: "Time".to_string(),
                    cardinality: Cardinality::ZeroOrOne,
                    description: "Time role".to_string(),
                },
                RoleDefinition {
                    id: place_role,
                    label: "Place".to_string(),
                    cardinality: Cardinality::ZeroOrOne,
                    description: "Place role".to_string(),
                },
            ],
            lexical_bindings: lex,
        }
    }

    pub fn t_tomorrow() -> TermId {
        TermId::new_random(777, 2)
    }
}
