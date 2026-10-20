//! Fixture driver selecting known canonical revisions from explicit test references.
//! Production APIs require the caller to supply RevisionRef; no such lookup is implicit there.
#![allow(dead_code)]
use sem_lang_memory::{bindings, definitions};
use sem_lang_repository::{
    CommitStore, Repository, RepositoryError, RevisionId, RevisionRef,
    commit_image::BorrowedBackend,
};
use sem_lang_vocab::VocabularyModule;

pub fn base<R: Repository + sem_lang_repository::Backend>(
    repository: &R,
    domain: &str,
    parent: Option<&RevisionId>,
    decision: &str,
) -> RevisionRef {
    let commits = CommitStore::new(BorrowedBackend(repository));
    if let Some(receipt) = commits.receipt(domain, decision).unwrap() {
        return receipt.previous_revision;
    }
    if let Some(parent) = parent {
        for root in commits.roots().unwrap() {
            let p = commits.committed(&root).unwrap().unwrap();
            if p.domain() == domain
                && sem_lang_repository::HeadPublication::decode(p.payload())
                    .unwrap()
                    .root
                    == sem_lang_repository::RepositoryRoot::Revision(parent.clone())
            {
                return commits
                    .receipt(domain, p.operation_id())
                    .unwrap()
                    .unwrap()
                    .committed_revision;
            }
        }
        // An intentionally invalid semantic parent in a rejection fixture is
        // paired with a real current concurrency revision, not a fabricated ID.
        return repository.head_revision(domain).unwrap();
    }
    RevisionRef::default()
}
pub fn adopt<R: Repository + sem_lang_repository::Backend>(
    repository: &mut R,
    namespace: &str,
    parent: Option<&RevisionId>,
    modules: &[VocabularyModule],
    decision: &str,
) -> Result<RevisionId, RepositoryError> {
    let expected = base(
        repository,
        &format!("definitions-{namespace}"),
        parent,
        decision,
    );
    definitions::adopt(repository, namespace, parent, modules, decision, &expected)
}
pub fn write<R: Repository + sem_lang_repository::Backend>(
    repository: &mut R,
    namespace: &str,
    parent: Option<&RevisionId>,
    bindings: &[bindings::SemanticBinding],
    state: bindings::AdoptionState,
    decision: &str,
) -> Result<RevisionId, RepositoryError> {
    let expected = base(
        repository,
        &format!("bindings-{namespace}"),
        parent,
        decision,
    );
    bindings::write(
        repository, namespace, parent, bindings, state, decision, &expected,
    )
}
pub fn publish_head<R: Repository + sem_lang_repository::Backend>(
    repo: &R,
    name: &str,
    operation: &str,
    target: sem_lang_repository::RepositoryRoot,
) {
    let expected = repo.head_revision(name).unwrap();
    let result = sem_lang_repository::HeadPublication {
        request_digest: sem_lang_core::Digest::object_digest(&serde_json::to_vec(&target).unwrap()),
        root: target,
    };
    let prepared = sem_lang_repository::head_intent(name, operation, &expected, &result).unwrap();
    repo.publish_head(&prepared).unwrap();
}
