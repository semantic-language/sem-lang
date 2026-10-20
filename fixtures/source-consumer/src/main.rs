//! Black-box archive wire check. Actual restored compiler acceptance lives in
//! the owner current_observation and consumer CLI/RPC compound scenarios.
use sem_lang_codec::{Codec, LanguageSnapshotDto};
use sem_lang_frontend::current::{EvaluationBound, ProcessingError};

fn main() {
    let language = LanguageSnapshotDto::standard();
    assert_eq!(language.version, "0.10.0");
    assert!(!language.terms.is_empty());
    let encoded = Codec::canonical_json(&language).unwrap();
    let decoded: LanguageSnapshotDto = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, language);
    assert_eq!(Codec::canonical_json(&decoded).unwrap(), encoded);
    for error in [
        ProcessingError::StaleRevision,
        ProcessingError::InvalidReference,
        ProcessingError::UnsupportedRepresentation,
        ProcessingError::EvaluationLimitExceeded {
            bound: EvaluationBound::EvidenceReferences,
            limit: 64,
            observed: 65,
        },
    ] {
        let encoded = Codec::canonical_json(&error).unwrap();
        let decoded: ProcessingError = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, error);
        assert_eq!(Codec::canonical_json(&decoded).unwrap(), encoded);
    }
    println!("Immutable registry archive: exact language snapshot and typed current failures verified; no execution or promotion implied.");
}
