//! ZG1: real owner storage, exact historical reads and PP-absent expiry.
#[cfg(feature = "fs-store")]
use sem_lang::FsRepository;
use sem_lang::{InMemoryRepository, RuntimeState, restoration, retention};
use sem_lang_context::ContextStore;
use sem_lang_memory::definitions;
use sem_lang_repository::{Backend, HeadStore, Repository, RetentionStore, RevisionRef};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn source<R: Repository + Backend>(
    repo: &mut R,
    namespace: &str,
) -> Result<restoration::RestorationInput> {
    let revision = definitions::adopt(
        repo,
        namespace,
        None,
        &[],
        namespace,
        &RevisionRef::default(),
    )?;
    Ok(restoration::checkpoint(
        repo,
        &[restoration::RevisionReference {
            namespace: namespace.into(),
            revision,
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )?)
}

fn scenario<R: Repository + Backend + RetentionStore>(repo: &mut R) -> Result {
    let definitions = definitions::adopt(
        repo,
        "retention",
        None,
        &[],
        "adopt",
        &RevisionRef::default(),
    )?;
    let input = restoration::checkpoint(
        repo,
        &[restoration::RevisionReference {
            namespace: "retention".into(),
            revision: definitions,
        }],
        &[],
        &ContextStore::default(),
        &RuntimeState::new(),
    )?;
    let receipt = retention::publish(repo, "runtime", "first", &RevisionRef::default(), &input)?;
    let registered =
        retention::register(repo, &retention::RepositoryPublication, &receipt, &input)?;
    assert_eq!(
        registered,
        retention::register(repo, &retention::RepositoryPublication, &receipt, &input)?
    );
    let lease = retention::acquire(
        repo,
        &retention::RepositoryPublication,
        &receipt,
        &input,
        "reader",
        1000,
        2000,
    )?;
    assert_eq!(
        lease,
        retention::acquire(
            repo,
            &retention::RepositoryPublication,
            &receipt,
            &input,
            "reader",
            1000,
            2000
        )?
    );
    assert!(retention::retire(repo, &retention::RepositoryPublication, &receipt, &input).is_err());
    assert!(
        retention::acquire(
            repo,
            &retention::RepositoryPublication,
            &receipt,
            &input,
            "overlong",
            1000,
            1_000_000
        )
        .is_err()
    );
    let mut other = RuntimeState::new();
    other.commit_state = Default::default();
    // Another accepted definition creates a distinct, fully valid checkpoint.
    let next_definition = definitions::adopt(
        repo,
        "other",
        None,
        &[],
        "adopt-other",
        &RevisionRef::default(),
    )?;
    let next = restoration::checkpoint(
        repo,
        &[restoration::RevisionReference {
            namespace: "other".into(),
            revision: next_definition,
        }],
        &[],
        &ContextStore::default(),
        &other,
    )?;
    let newer = retention::publish(
        repo,
        "runtime",
        "second",
        &receipt.committed_revision,
        &next,
    )?;
    assert!(
        retention::retire(repo, &retention::RepositoryPublication, &receipt, &input).is_err(),
        "active owner lease protects old source"
    );
    retention::read(repo, &retention::RepositoryPublication, &lease, 1500)?;
    assert_eq!(
        retention::reconcile(repo, &retention::RepositoryPublication, 2000)?,
        1
    );
    assert!(retention::read(repo, &retention::RepositoryPublication, &lease, 2000).is_err());
    retention::release(repo, &retention::RepositoryPublication, &lease)?;
    retention::release(repo, &retention::RepositoryPublication, &lease)?;
    retention::retire(repo, &retention::RepositoryPublication, &receipt, &input)?;
    assert!(
        retention::acquire(
            repo,
            &retention::RepositoryPublication,
            &receipt,
            &input,
            "late",
            2000,
            3000
        )
        .is_err()
    );
    assert!(retention::reclaim(repo, &retention::RepositoryPublication)? > 0);
    assert!(
        retention::register(repo, &retention::RepositoryPublication, &receipt, &input).is_err()
    );
    assert_eq!(retention::replay(repo, &receipt)?, receipt);
    assert_eq!(repo.head_revision("runtime")?, newer.committed_revision);
    assert_eq!(retention::replay(repo, &newer)?, newer);
    let current = retention::acquire(
        repo,
        &retention::RepositoryPublication,
        &newer,
        &next,
        "current",
        3000,
        4000,
    )?;
    retention::read(repo, &retention::RepositoryPublication, &current, 3001)?;
    retention::release(repo, &retention::RepositoryPublication, &current)?;
    retention::verify(repo, &retention::RepositoryPublication)?;
    assert!(repo.verify()?.errors.is_empty());
    Ok(())
}

#[test]
fn exact_source_lifecycle_is_available_in_the_default_memory_store() -> Result {
    scenario(&mut InMemoryRepository::new())?;
    Ok(())
}

#[cfg(feature = "fs-store")]
#[test]
fn exact_source_lifecycle_has_filesystem_parity() -> Result {
    let path = std::env::temp_dir().join(format!("sem-zg1-{}", std::process::id()));
    std::fs::create_dir(&path)?;
    let result = scenario(&mut FsRepository::create(&path)?);
    if result.is_ok() {
        std::fs::remove_dir_all(path)?;
    }
    result
}

#[test]
fn one_holder_can_read_multiple_exact_sources_and_cleanup_lost_responses() -> Result {
    let mut repo = InMemoryRepository::new();
    let owner = retention::RepositoryPublication;
    assert_eq!(retention::release_holder(&mut repo, &owner, "job")?, 0);
    assert!(repo.retention_metadata()?.is_none());
    let first = source(&mut repo, "first")?;
    let r1 = retention::publish(&mut repo, "runtime", "one", &RevisionRef::default(), &first)?;
    let second = source(&mut repo, "second")?;
    let r2 = retention::publish(&mut repo, "runtime", "two", &r1.committed_revision, &second)?;
    let l1 = retention::acquire(&mut repo, &owner, &r1, &first, "job", 100, 200)?;
    let l2 = retention::acquire(&mut repo, &owner, &r2, &second, "job", 100, 200)?;
    assert_ne!(l1.generation, l2.generation);
    // Repeated acquire does not silently renew the owner lease.
    assert_eq!(
        retention::acquire(&mut repo, &owner, &r1, &first, "job", 110, 210)?,
        l1
    );
    retention::read(&repo, &owner, &l1, 150)?;
    retention::read(&repo, &owner, &l2, 150)?;
    let (receipt, input) = retention::leased_input(&repo, &owner, "job", l1.generation, 150)?;
    assert_eq!(receipt, l1.receipt);
    assert_eq!(input, l1.input);
    let view = sem_lang::Engine::restore_view(&repo, input.clone())?;
    assert_eq!(view.restoration_input(), Some(&input));
    assert!(view.repository().is_none());
    assert!(retention::leased_input(&repo, &owner, "other", l1.generation, 150).is_err());
    assert!(
        retention::leased_input(&repo, &owner, "job", l1.generation, l1.expires_at_ms).is_err()
    );
    assert_eq!(retention::release_holder(&mut repo, &owner, "job")?, 2);
    assert_eq!(retention::release_holder(&mut repo, &owner, "job")?, 0);
    let l3 = retention::acquire(&mut repo, &owner, &r1, &first, "job", 200, 300)?;
    retention::release(&mut repo, &owner, &l1)?;
    retention::release_generation(&mut repo, &owner, "job", l1.generation)?;
    assert!(retention::release_generation(&mut repo, &owner, "other", l3.generation).is_err());
    retention::read(&repo, &owner, &l3, 250)?;
    // Acquisition itself reconciles dead-owner leases before admitting new work.
    let l4 = retention::acquire(&mut repo, &owner, &r2, &second, "job", 300, 400)?;
    assert!(retention::read(&repo, &owner, &l3, 300).is_err());
    assert_eq!(retention::reconcile(&mut repo, &owner, 300)?, 0);
    assert_eq!(retention::release_holder(&mut repo, &owner, "job")?, 1);
    retention::release_generation(&mut repo, &owner, "job", l4.generation)?;
    assert_eq!(retention::replay(&repo, &r1)?, r1);
    assert_eq!(repo.head_revision("runtime")?, r2.committed_revision);
    Ok(())
}

#[test]
#[cfg(feature = "fs-store")]
fn abrupt_reader_exit_is_reconciled_by_owner_without_projection_runtime() -> Result {
    let path = std::env::temp_dir().join(format!("sem-zg1-crash-{}", std::process::id()));
    std::fs::create_dir(&path)?;
    let mut repo = FsRepository::create(&path)?;
    let input = source(&mut repo, "crash")?;
    let receipt = retention::publish(&mut repo, "runtime", "old", &RevisionRef::default(), &input)?;
    let newer = source(&mut repo, "newer")?;
    retention::publish(
        &mut repo,
        "runtime",
        "new",
        &receipt.committed_revision,
        &newer,
    )?;
    std::fs::write(
        path.join("request.json"),
        serde_json::to_vec(&(receipt.clone(), input.clone()))?,
    )?;
    drop(repo);
    let output = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "crash_reader", "--nocapture"])
        .env("SEM_ZG1_CRASH_FIXTURE", &path)
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(77),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut repo = FsRepository::open_existing(&path)?;
    assert_eq!(
        retention::reconcile(&mut repo, &retention::RepositoryPublication, 1999)?,
        0
    );
    assert!(
        retention::retire(
            &mut repo,
            &retention::RepositoryPublication,
            &receipt,
            &input
        )
        .is_err()
    );
    assert_eq!(
        retention::reconcile(&mut repo, &retention::RepositoryPublication, 2000)?,
        1
    );
    assert_eq!(
        retention::reconcile(&mut repo, &retention::RepositoryPublication, 2000)?,
        0
    );
    retention::retire(
        &mut repo,
        &retention::RepositoryPublication,
        &receipt,
        &input,
    )?;
    assert!(retention::reclaim(&mut repo, &retention::RepositoryPublication)? > 0);
    assert_eq!(retention::replay(&repo, &receipt)?, receipt);
    retention::verify(&repo, &retention::RepositoryPublication)?;
    drop(repo);
    let mut repo = FsRepository::open_existing(&path)?;
    assert!(
        retention::register(
            &mut repo,
            &retention::RepositoryPublication,
            &receipt,
            &input
        )
        .is_err()
    );
    assert!(sem_lang::Engine::restore(repo, input).is_err());
    std::fs::remove_dir_all(path)?;
    Ok(())
}

#[test]
#[cfg(feature = "fs-store")]
fn crash_reader() -> Result {
    let Some(path) = std::env::var_os("SEM_ZG1_CRASH_FIXTURE") else {
        return Ok(());
    };
    let path = std::path::PathBuf::from(path);
    let (receipt, input) = serde_json::from_slice(&std::fs::read(path.join("request.json"))?)?;
    let mut repo = FsRepository::open_existing(&path)?;
    retention::acquire(
        &mut repo,
        &retention::RepositoryPublication,
        &receipt,
        &input,
        "child-reader",
        1000,
        2000,
    )?;
    // Intentionally bypass Drop: only durable owner metadata can release this hold.
    std::process::exit(77)
}

#[test]
fn historical_repair_is_explicit_exact_and_does_not_republish() -> Result {
    use sem_lang_repository::{HeadPublication, HeadStore, RepositoryRoot, head_intent};
    let mut repo = InMemoryRepository::new();
    let input = source(&mut repo, "historical")?;
    let prepared = head_intent(
        "historical",
        "original",
        &RevisionRef::default(),
        &HeadPublication {
            root: RepositoryRoot::Snapshot(input.runtime_snapshot.clone()),
            request_digest: RuntimeState::new().semantic_state_digest()?,
        },
    )?;
    let original = repo.publish_head(&prepared)?;
    assert!(repo.retention_metadata()?.is_none());
    assert!(
        retention::acquire(
            &mut repo,
            &retention::RepositoryPublication,
            &original,
            &input,
            "reader",
            1000,
            2000
        )
        .is_err()
    );
    assert!(repo.retention_metadata()?.is_none(), "lookup is not repair");
    let mut substituted = original.clone();
    substituted.committed_revision.sequence += 1;
    assert!(
        retention::register(
            &mut repo,
            &retention::RepositoryPublication,
            &substituted,
            &input
        )
        .is_err()
    );
    assert!(repo.retention_metadata()?.is_none());
    let registrations = retention::register(
        &mut repo,
        &retention::RepositoryPublication,
        &original,
        &input,
    )?;
    assert!(!registrations.is_empty());
    assert_eq!(
        retention::register(
            &mut repo,
            &retention::RepositoryPublication,
            &original,
            &input
        )?,
        registrations
    );
    assert_eq!(repo.publish_head(&prepared)?, original);
    assert_eq!(
        repo.head_revision("historical")?,
        original.committed_revision
    );
    let first = retention::acquire(
        &mut repo,
        &retention::RepositoryPublication,
        &original,
        &input,
        "reader",
        1000,
        2000,
    )?;
    retention::release(&mut repo, &retention::RepositoryPublication, &first)?;
    let second = retention::acquire(
        &mut repo,
        &retention::RepositoryPublication,
        &original,
        &input,
        "reader",
        1000,
        2000,
    )?;
    assert_ne!(first.generation, second.generation);
    retention::release(&mut repo, &retention::RepositoryPublication, &first)?;
    retention::read(&repo, &retention::RepositoryPublication, &second, 1001)?;
    let mut wrong = second.clone();
    wrong.input.memory = "substituted".into();
    assert!(retention::read(&repo, &retention::RepositoryPublication, &wrong, 1001).is_err());
    assert!(retention::release(&mut repo, &retention::RepositoryPublication, &wrong).is_err());
    retention::read(&repo, &retention::RepositoryPublication, &second, 1001)?;
    retention::release(&mut repo, &retention::RepositoryPublication, &second)?;
    assert!(
        retention::reclaim(&mut repo, &retention::RepositoryPublication)? == 0,
        "release never retires registrations"
    );
    retention::verify(&repo, &retention::RepositoryPublication)?;
    Ok(())
}

#[test]
fn interrupted_registration_retries_without_a_second_commit_and_rejects_authority_switch() -> Result
{
    use sem_lang_repository::HeadStore;
    let mut repo = InMemoryRepository::new();
    let input = source(&mut repo, "registration-retry")?;
    let receipt = retention::publish(
        &mut repo,
        "runtime",
        "original",
        &RevisionRef::default(),
        &input,
    )?;
    let original_metadata = repo.retention_metadata()?.unwrap();
    let mut metadata: serde_json::Value = serde_json::from_slice(&original_metadata)?;
    // Exact durable registration-intent image at the post-Graph/pre-response
    // boundary. No source or canonical receipt is modified by this failpoint.
    metadata["groups"][&receipt.commit_ref]["registrations"] = serde_json::json!([]);
    repo.store_retention_metadata(&serde_json::to_vec(&metadata)?)?;
    assert!(retention::verify(&repo, &retention::RepositoryPublication).is_err());
    assert_eq!(
        retention::publish(
            &mut repo,
            "runtime",
            "original",
            &RevisionRef::default(),
            &input,
        )?,
        receipt
    );
    assert_eq!(repo.retention_metadata()?.unwrap(), original_metadata);
    assert_eq!(repo.head_revision("runtime")?, receipt.committed_revision);
    retention::verify(&repo, &retention::RepositoryPublication)?;

    metadata = serde_json::from_slice(&original_metadata)?;
    metadata["publication_owner"] = serde_json::json!("different/canonical/owner");
    let foreign = serde_json::to_vec(&metadata)?;
    repo.store_retention_metadata(&foreign)?;
    assert!(
        retention::publish(
            &mut repo,
            "runtime",
            "must-not-commit",
            &receipt.committed_revision,
            &input,
        )
        .is_err()
    );
    assert_eq!(repo.retention_metadata()?.unwrap(), foreign);
    assert_eq!(repo.head_revision("runtime")?, receipt.committed_revision);
    assert_eq!(retention::replay(&repo, &receipt)?, receipt);
    Ok(())
}

#[test]
fn retirement_replay_does_not_retire_a_new_publication_of_shared_source() -> Result {
    let mut repo = InMemoryRepository::new();
    let first = source(&mut repo, "shared-first")?;
    let second = source(&mut repo, "shared-second")?;
    let a = retention::publish(&mut repo, "runtime", "a", &RevisionRef::default(), &first)?;
    let b = retention::publish(&mut repo, "runtime", "b", &a.committed_revision, &second)?;
    retention::retire(&mut repo, &retention::RepositoryPublication, &a, &first)?;
    let c = retention::publish(&mut repo, "runtime", "c", &b.committed_revision, &first)?;
    retention::retire(&mut repo, &retention::RepositoryPublication, &a, &first)?;
    assert_eq!(
        retention::reclaim(&mut repo, &retention::RepositoryPublication)?,
        0
    );
    let lease = retention::acquire(
        &mut repo,
        &retention::RepositoryPublication,
        &c,
        &first,
        "current",
        1,
        2,
    )?;
    retention::read(&repo, &retention::RepositoryPublication, &lease, 1)?;
    assert_eq!(retention::replay(&repo, &a)?, a);
    assert_eq!(retention::replay(&repo, &c)?, c);
    retention::release(&mut repo, &retention::RepositoryPublication, &lease)?;
    Ok(())
}
