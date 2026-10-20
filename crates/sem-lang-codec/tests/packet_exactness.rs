use sem_lang_codec::Codec;
use sem_lang_core::*;
use std::collections::BTreeMap;

fn packet() -> SemanticPacket {
    let claim = Claim {
        id: NodeId::new_random(1, 1),
        actor: Some(Value::Entity(EntityId::new_random(2, 2))),
        predicate: TermId::new_random(3, 3),
        arguments: vec![Argument {
            role: Some(RoleId::new_random(4, 4)),
            value: Value::Bool(false),
        }],
        constraints: vec![Constraint::ScopeConstraint {
            variable: Variable {
                id: "x".into(),
                expected: None,
            },
            type_id: TypeId::new_random(5, 5),
        }],
        validity: Validity {
            valid_from: Some(0),
            valid_until: Some(99),
        },
    };
    SemanticPacket {
        id: PacketId::new_random(6, 6),
        conversation_id: Some("interaction-1".into()),
        sender: Some("subject-1".into()),
        recipient: Some("subject-2".into()),
        cause: Some("cause-1".into()),
        correlation: Some("correlation-1".into()),
        act: Some(TermId::new_random(7, 7)),
        content: Expr::Apply(Apply {
            operator: claim.predicate,
            arguments: claim.arguments.clone(),
        }),
        context_delta: Some(ContextDelta {
            added_claims: vec![claim.clone()],
            removed_claims: vec![claim],
            annotations: BTreeMap::from([("source".into(), "external-1".into())]),
        }),
        vocabulary_manifest: Some("definition-revision-1".into()),
        resolution: Resolution::Resolved,
        pragmatics: vec![Value::Map(BTreeMap::from([(
            "confidence".into(),
            Value::Float(0.125),
        )]))],
        provenance: Some(Provenance {
            source: Some("provider-1".into()),
            confidence: Some(0.75),
            observed_at: 123,
        }),
    }
}

#[test]
fn all_packet_fields_and_expression_variants_survive_roundtrip() {
    let mut p = packet();
    let values = vec![
        Value::Nil,
        Value::Bool(false),
        Value::Integer(i64::MIN),
        Value::Float(0.1),
        Value::Text("日本語 / English".into()),
        Value::Decimal {
            coefficient: i128::MAX,
            scale: -3,
        },
        Value::TermRef(TermId::new_random(8, 8)),
        Value::TypeRef(TypeId::new_random(9, 9)),
        Value::RoleRef(RoleId::new_random(10, 10)),
        Value::Entity(EntityId::new_random(11, 11)),
        Value::Node(NodeId::new_random(12, 12)),
        Value::Expr(ExprId::new_random(13, 13)),
        Value::TypedHole(TypedHole {
            id: "hole".into(),
            expected: Some(TypeId::new_random(14, 14)),
            surface: Some("unknown".into()),
        }),
    ];
    for expr in [
        p.content.clone(),
        Expr::Value(Value::Sequence(values)),
        Expr::Binding(Binding {
            variable: Variable {
                id: "v".into(),
                expected: None,
            },
            value: Value::Text("value".into()),
        }),
        Expr::Reference(Reference {
            target: Value::Text("ref".into()),
            scope: Some("scope".into()),
        }),
        Expr::TypedHole(TypedHole {
            id: "hole-2".into(),
            expected: None,
            surface: None,
        }),
    ] {
        p.content = expr;
        for resolution in [
            Resolution::Resolved,
            Resolution::Pending,
            Resolution::Unknown,
            Resolution::Failed,
        ] {
            p.resolution = resolution;
            let encoded = Codec::encode_packet(&p).expect("encode every typed field");
            let restored = Codec::decode_packet(&encoded).expect("decode every typed field");
            assert_eq!(restored, p);
            assert_eq!(Codec::encode_packet(&restored).unwrap(), encoded);
        }
    }
}

#[test]
fn malformed_ids_and_unknown_status_are_rejected_not_fabricated() {
    let encoded = Codec::encode_packet(&packet()).unwrap();
    for (field, value) in [
        ("id", "broken"),
        ("act", "broken"),
        ("resolution", "InventedStatus"),
    ] {
        let mut raw: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        raw[field] = serde_json::json!(value);
        assert!(
            Codec::decode_packet(&raw.to_string()).is_err(),
            "accepted invalid {field}"
        );
    }
    let mut raw: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    raw["unrecognized"] = serde_json::json!(true);
    assert!(Codec::decode_packet(&raw.to_string()).is_err());
}

#[test]
fn non_finite_numbers_are_not_silently_serialized_to_null_or_text() {
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut p = packet();
        p.content = Expr::Value(Value::Float(number));
        assert!(Codec::encode_packet(&p).is_err());
    }
    for confidence in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut p = packet();
        p.provenance.as_mut().unwrap().confidence = Some(confidence);
        assert!(Codec::encode_packet(&p).is_err());
    }
    let mut p = packet();
    p.content = Expr::Value(Value::Text("x".repeat(1_048_576)));
    assert!(Codec::encode_packet(&p).is_err());
    assert!(Codec::decode_packet(&" ".repeat(1_048_577)).is_err());
}
