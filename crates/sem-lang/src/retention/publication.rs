//! Canonical anchor validation is supplied by the publisher of the source ref.
//! This port cannot decide meaning, expand a dependency closure or grant execution.
use super::*;

/// A semantic source may be referenced by another owner's canonical publication.
/// Its original receipt stays in that owner's Foundation; it is never copied into
/// this repository as a second canonical commit. A repository binds one retention
/// authority identity explicitly and rejects substitution (no backend fallback).
pub trait SourcePublication<R: Repository + Backend> {
    type Retention<'a>: ExternalRetention
    where
        Self: 'a,
        R: 'a;
    fn identity(&self) -> &str;
    fn retention<'a>(&'a self, repository: &'a R) -> Self::Retention<'a>;
    fn verify_receipt(&self, repository: &R, receipt: &CommitReceipt) -> Result<()>;
    fn restoration(&self, repository: &R, receipt: &CommitReceipt) -> Result<RestorationInput>;
}

/// Retirement needs an explicit domain requirement authority in addition to
/// receipt validation. A read-only anchor adapter does not gain this capability.
pub trait SourceRequirements<R: Repository + Backend> {
    /// Exact current/in-flight owner requirements; the semantic owner alone
    /// resolves the internal objects behind these references.
    fn required_sources(&self, repository: &R) -> Result<Vec<RestorationInput>>;
}

/// Standalone sem-lang's existing repository-head publication authority.
pub struct RepositoryPublication;
impl<R: Repository + Backend> SourcePublication<R> for RepositoryPublication {
    type Retention<'a>
        = CommitStore<BorrowedBackend<'a, R>>
    where
        R: 'a;
    fn identity(&self) -> &str {
        "sem-lang/repository"
    }
    fn retention<'a>(&'a self, repo: &'a R) -> Self::Retention<'a> {
        commits(repo)
    }
    fn verify_receipt(&self, repo: &R, receipt: &CommitReceipt) -> Result<()> {
        replay(repo, receipt).map(|_| ())
    }
    fn restoration(&self, repo: &R, receipt: &CommitReceipt) -> Result<RestorationInput> {
        self.verify_receipt(repo, receipt)?;
        let Some(RepositoryRoot::Snapshot(id)) = repo.head_at(&receipt.committed_revision)? else {
            return Err(RetentionError::Mismatch);
        };
        Ok(restoration::checkpoint_input(repo, &id)?)
    }
}
impl<R: Repository + Backend> SourceRequirements<R> for RepositoryPublication {
    fn required_sources(&self, repo: &R) -> Result<Vec<RestorationInput>> {
        let mut inputs = Vec::new();
        for root in repo.root_references()? {
            if let RepositoryRoot::Snapshot(id) = root
                && id.starts_with("runtime-")
            {
                inputs.push(restoration::checkpoint_input(repo, &id)?);
            }
        }
        Ok(inputs)
    }
}
