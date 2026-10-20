//! Serialization and deserialization for sem-lang DTOs.

mod language_snapshot;
pub use language_snapshot::{LanguageSnapshotDto, LanguageTermDto};

use sem_lang_core::{Digest, SemanticPacket};
use sem_lang_repository::{RepositoryRevision, RepositorySnapshot, VerificationReport};
use serde::Serialize;
use std::fmt::{self, Display, Formatter};

pub type Result<T> = std::result::Result<T, CodecError>;

#[derive(Debug, Clone)]
pub enum CodecError {
    Serialize(String),
    Deserialize(String),
}

impl Display for CodecError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(msg) => write!(f, "serialize error: {msg}"),
            Self::Deserialize(msg) => write!(f, "deserialize error: {msg}"),
        }
    }
}

impl std::error::Error for CodecError {}

pub struct Codec;

impl Codec {
    pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
        let value =
            serde_json::to_value(value).map_err(|err| CodecError::Serialize(err.to_string()))?;
        let sorted = canonicalize_json(value);
        serde_json::to_string(&sorted).map_err(|err| CodecError::Serialize(err.to_string()))
    }

    pub fn encode_packet(packet: &SemanticPacket) -> Result<String> {
        let encoded = Self::canonical_json(packet)?;
        if encoded.len() > 1_048_576 {
            return Err(CodecError::Serialize("packet exceeds 1 MiB".into()));
        }
        Ok(encoded)
    }

    pub fn decode_packet(raw: &str) -> Result<SemanticPacket> {
        if raw.len() > 1_048_576 {
            return Err(CodecError::Deserialize("packet exceeds 1 MiB".into()));
        }
        serde_json::from_str(raw).map_err(|err| CodecError::Deserialize(err.to_string()))
    }

    pub fn encode_repository_revision(rev: &RepositoryRevision) -> Result<String> {
        Self::canonical_json(rev)
    }

    pub fn decode_repository_revision(raw: &str) -> Result<RepositoryRevision> {
        serde_json::from_str(raw).map_err(|err| CodecError::Deserialize(err.to_string()))
    }

    pub fn encode_repository_snapshot(snapshot: &RepositorySnapshot) -> Result<String> {
        Self::canonical_json(snapshot)
    }

    pub fn decode_repository_snapshot(raw: &str) -> Result<RepositorySnapshot> {
        serde_json::from_str(raw).map_err(|err| CodecError::Deserialize(err.to_string()))
    }

    pub fn encode_verification_report(report: &VerificationReport) -> Result<String> {
        Self::canonical_json(report)
    }
}

pub fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(mut map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut ordered = serde_json::Map::new();
            for key in keys {
                if let Some(value) = map.remove(&key) {
                    ordered.insert(key, canonicalize_json(value));
                }
            }
            serde_json::Value::Object(ordered)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonicalize_json).collect())
        }
        value => value,
    }
}

pub struct PacketDigest;

impl PacketDigest {
    pub fn proposition(packet: &SemanticPacket) -> Result<Digest> {
        Ok(Digest::proposition_digest(
            Codec::canonical_json(&packet.content)?.as_bytes(),
        ))
    }

    pub fn interaction(packet: &SemanticPacket) -> Result<Digest> {
        Ok(Digest::interaction_digest(
            Codec::encode_packet(packet)?.as_bytes(),
        ))
    }

    pub fn semantic_state(packet: &SemanticPacket) -> Result<Digest> {
        Ok(Digest::semantic_state_digest(
            Codec::canonical_json(&(&packet.content, &packet.context_delta))?.as_bytes(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_core::{PacketId, Resolution, TermId, Value};
    use sem_lang_linguistics::{OpaqueReason, ResidualReason, SemanticResidual};

    #[test]
    fn deterministic_json_order() {
        let left = serde_json::json!({"b":1,"a":2});
        let right = serde_json::json!({"a":2,"b":1});
        assert_eq!(
            Codec::canonical_json(&left).unwrap(),
            Codec::canonical_json(&right).unwrap(),
        );
    }

    #[test]
    fn packet_roundtrip() {
        let packet = sem_lang_core::SemanticPacket {
            id: PacketId::new_random(1, 1),
            conversation_id: Some("main".to_string()),
            sender: None,
            recipient: None,
            cause: None,
            correlation: None,
            act: None,
            content: sem_lang_core::Expr::Value(Value::Text("x".to_string())),
            context_delta: None,
            vocabulary_manifest: None,
            resolution: Resolution::Resolved,
            pragmatics: Vec::new(),
            provenance: None,
        };
        let encoded = Codec::encode_packet(&packet).unwrap();
        let decoded = Codec::decode_packet(&encoded).unwrap();
        assert_eq!(decoded.id.to_hex(), packet.id.to_hex());
    }

    #[test]
    fn typed_residuals_preserve_structure_without_runtime_fields() {
        for reason in [
            OpaqueReason::DomainSpecific("specialized".into()),
            OpaqueReason::InsufficientEvidence("missing source".into()),
            OpaqueReason::ExplicitNoExpand,
        ] {
            let residual = SemanticResidual {
                source_term: TermId::new_random(8, 1),
                reason: ResidualReason::Opaque(reason),
            };
            let encoded = Codec::canonical_json(&residual).unwrap();
            assert_eq!(
                serde_json::from_str::<SemanticResidual>(&encoded).unwrap(),
                residual
            );
        }
        assert!(
            serde_json::from_value::<SemanticResidual>(serde_json::json!({
                "source_term": TermId::new_random(8, 1),
                "reason_code": "domain-specific", "detail": "guessed"
            }))
            .is_err()
        );
    }
}
