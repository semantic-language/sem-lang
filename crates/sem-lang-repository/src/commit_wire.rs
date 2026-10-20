//! Lossless wire encoding for foreign Foundation types. No decision policy.
//! Serde remote derives match every variant; Foundation additions fail compilation.
//! C4 2026: preserve noncanonical/reclamation refusal without adding semantic policy.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(remote = "crate::Conflict")]
pub enum Conflict {
    Stale {
        expected: crate::RevisionRef,
        actual: crate::RevisionRef,
    },
    OperationReuse {
        original_prepared_ref: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "crate::Rejection")]
pub enum Rejection {
    PreparedNotCanonical,
    Reclaiming,
    InvalidIntent,
    Capacity,
    InvalidParent,
    Abandoned,
    NotPrepared,
}
#[derive(Serialize, Deserialize)]
#[serde(remote = "crate::Failure")]
pub enum Failure {
    Storage,
    Corrupt,
    RecoveryRequired,
    Capacity,
    Rejected(#[serde(with = "Rejection")] crate::Rejection),
    DeliveryUnknown,
}

/// Representation/codec diagnostics are text; Foundation failures never use this.
pub fn display_error<S: serde::Serializer, E: std::fmt::Display>(
    error: &E,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&error.to_string())
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    #[derive(Serialize, Deserialize)]
    struct Envelope(#[serde(with = "super::Failure")] crate::Failure);
    #[test]
    fn foundation_refusals_remain_lossless_typed_failures_on_the_wire() {
        for expected in [crate::Failure::Corrupt, crate::Failure::RecoveryRequired] {
            let wire = serde_json::to_value(Envelope(expected.clone())).unwrap();
            assert_eq!(wire, serde_json::Value::String(format!("{expected:?}")));
            let decoded: Envelope = serde_json::from_value(wire).unwrap();
            assert_eq!(decoded.0, expected);
        }
        for refusal in [
            crate::Rejection::PreparedNotCanonical,
            crate::Rejection::Reclaiming,
        ] {
            let expected = crate::Failure::Rejected(refusal);
            let wire = serde_json::to_value(Envelope(expected.clone())).unwrap();
            assert!(wire.get("Rejected").is_some());
            let decoded: Envelope = serde_json::from_value(wire).unwrap();
            assert_eq!(decoded.0, expected);
        }
    }
}
