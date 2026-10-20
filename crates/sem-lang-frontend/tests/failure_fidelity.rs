use sem_lang_context::{
    CommitConflict, CommitFailure, CommitRejection, CommitRevision, ContextError,
};
use sem_lang_frontend::current::ConfirmationError;

#[test]
fn foundation_failures_are_lossless_and_distinct_from_semantic_uncertainty() {
    let values = [
        ContextError::Commit(CommitFailure::Storage),
        ContextError::Commit(CommitFailure::Corrupt),
        ContextError::Commit(CommitFailure::Capacity),
        ContextError::Commit(CommitFailure::DeliveryUnknown),
        ContextError::Commit(CommitFailure::Rejected(CommitRejection::InvalidIntent)),
        ContextError::CommitRejected(CommitRejection::InvalidIntent),
        ContextError::CommitRejected(CommitRejection::InvalidParent),
        ContextError::CommitRejected(CommitRejection::Capacity),
        ContextError::CommitRejected(CommitRejection::Abandoned),
        ContextError::CommitRejected(CommitRejection::NotPrepared),
        ContextError::CommitConflict(CommitConflict::Stale {
            expected: CommitRevision::default(),
            actual: CommitRevision {
                sequence: 1,
                commit: Some("f".repeat(64)),
            },
        }),
        ContextError::CommitConflict(CommitConflict::OperationReuse {
            original_prepared_ref: "e".repeat(64),
        }),
        ContextError::Busy { wait_millis: 25 },
        ContextError::Persistence("fixture encoding failure".into()),
        ContextError::RepositoryUnavailable {
            kind: "PermissionDenied".into(),
            detail: "fixture I/O failure".into(),
        },
        ContextError::Repository(sem_lang_context::ContextRepositoryError::MissingObject(
            "missing".into(),
        )),
    ];
    for error in values {
        let owner = serde_json::to_value(&error).unwrap();
        let forwarded = ConfirmationError::from(error.clone());
        assert_eq!(serde_json::to_value(&forwarded).unwrap(), owner);
        assert_eq!(
            serde_json::from_value::<ContextError>(owner.clone()).unwrap(),
            error
        );
        assert_eq!(
            serde_json::from_value::<ConfirmationError>(owner).unwrap(),
            forwarded
        );
    }
    assert_eq!(
        ConfirmationError::from(ContextError::RevisionConflict),
        ConfirmationError::StaleRevision
    );
}
