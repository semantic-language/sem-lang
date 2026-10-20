//! Exact semantic source ownership. Leases are temporary physical protection,
//! not memory duration, canonical publication, projection repair or authority.
use crate::restoration::{self, RestorationError, RestorationInput};
use sem_lang_core::Digest;
use sem_lang_repository::{
    Backend, CommitReceipt, CommitStore, ExternalRegistration, ExternalRegistrationStatus,
    ExternalRetention, ExternalRoot, ExternalRootKind, Failure, HeadPublication, Repository,
    RepositoryError, RepositoryRoot, RetentionStore, RevisionRef, commit_image::BorrowedBackend,
    head_intent, retention::external_root,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
mod publication;
pub use publication::{RepositoryPublication, SourcePublication, SourceRequirements};

/// Owner limit, deliberately independent of the projection runtime repair limit.
pub const MAX_LEASE_MS: u64 = 60_000;
const MAX_GROUPS: usize = 128;
const MAX_LEASES: usize = 32;
const MAX_DEPENDENCIES: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum RetentionError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Restoration(#[from] RestorationError),
    #[error("source lease or exact publication does not match")]
    Mismatch,
    #[error("exact source is not retained")]
    Unavailable,
    #[error("source is required by a current head or active owner lease")]
    Required,
    #[error("source retention capacity exceeded")]
    Capacity,
}
impl From<Failure> for RetentionError {
    fn from(error: Failure) -> Self {
        Self::Repository(error.into())
    }
}
type Result<T> = std::result::Result<T, RetentionError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLease {
    pub generation: u64,
    pub holder: String,
    pub receipt: CommitReceipt,
    pub input: RestorationInput,
    pub expires_at_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    receipt: CommitReceipt,
    input: RestorationInput,
    registrations: Vec<ExternalRegistration>,
    retired: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseRecord {
    lease: SourceLease,
    acquired: bool,
    releasing: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    publication_owner: Option<String>,
    sequence: u64,
    groups: BTreeMap<String, Group>,
    leases: BTreeMap<u64, LeaseRecord>,
}
fn load<R: RetentionStore>(repo: &R) -> Result<Metadata> {
    let Some(bytes) = repo.retention_metadata()? else {
        return Ok(Metadata::default());
    };
    let value: Metadata = serde_json::from_slice(&bytes).map_err(|_| Failure::Corrupt)?;
    if value.groups.len() > MAX_GROUPS
        || value.leases.len() > MAX_LEASES
        || value
            .groups
            .values()
            .any(|g| g.registrations.len() > MAX_DEPENDENCIES)
        || value.leases.iter().any(|(id, r)| {
            *id == 0
                || *id > value.sequence
                || *id != r.lease.generation
                || !value.groups.contains_key(&r.lease.receipt.commit_ref)
        })
    {
        return Err(Failure::Corrupt.into());
    }
    Ok(value)
}
fn save<R: RetentionStore>(repo: &mut R, metadata: &Metadata) -> Result<()> {
    let bytes = serde_json::to_vec(metadata).map_err(|_| Failure::Corrupt)?;
    if bytes.len() > sem_lang_repository::retention::MAX_RETENTION_BYTES {
        return Err(RetentionError::Capacity);
    }
    repo.store_retention_metadata(&bytes)?;
    Ok(())
}
fn bind(metadata: &mut Metadata, identity: &str) -> Result<()> {
    if identity.is_empty() || identity.len() > 128 || identity.chars().any(char::is_control) {
        return Err(RetentionError::Mismatch);
    }
    if metadata.publication_owner.is_none() {
        metadata.publication_owner = Some(identity.into());
    }
    bound(metadata, identity)
}
fn bound(metadata: &Metadata, identity: &str) -> Result<()> {
    if metadata.publication_owner.as_deref() != Some(identity) {
        return Err(RetentionError::Unavailable);
    }
    Ok(())
}
fn commits<R: Backend>(repo: &R) -> CommitStore<BorrowedBackend<'_, R>> {
    CommitStore::new(BorrowedBackend(repo))
}

/// Receipt replay is metadata-only, even after its physical dependencies expire.
pub fn replay<R: Backend>(repo: &R, receipt: &CommitReceipt) -> Result<CommitReceipt> {
    let store = commits(repo);
    let prepared = store
        .committed(&receipt.commit_ref)?
        .ok_or(RetentionError::Mismatch)?;
    let actual = store
        .receipt(prepared.domain(), prepared.operation_id())?
        .ok_or(RetentionError::Mismatch)?;
    if actual != *receipt {
        return Err(RetentionError::Mismatch);
    }
    Ok(actual)
}
fn exact<R: Repository + Backend, P: SourcePublication<R>>(
    repo: &R,
    publication: &P,
    receipt: &CommitReceipt,
    input: &RestorationInput,
) -> Result<()> {
    publication.verify_receipt(repo, receipt)?;
    if publication.restoration(repo, receipt)? != *input {
        return Err(RetentionError::Mismatch);
    }
    restoration::read(repo, input)?;
    Ok(())
}
fn closure<R: Repository>(repo: &R, input: &RestorationInput) -> Result<Vec<RepositoryRoot>> {
    restoration::read(repo, input)?;
    let mut roots = BTreeMap::new();
    let mut insert = |root: RepositoryRoot| -> Result<()> {
        roots.insert(external_root(&root)?.object, root);
        if roots.len() > MAX_DEPENDENCIES {
            return Err(RetentionError::Capacity);
        }
        Ok(())
    };
    insert(RepositoryRoot::Object(input.memory.clone()))?;
    insert(RepositoryRoot::Snapshot(input.runtime_snapshot.clone()))?;
    let mut seen = BTreeSet::new();
    for reference in input.definitions.iter().chain(&input.bindings) {
        let mut next = Some(reference.revision.clone());
        while let Some(id) = next {
            if !seen.insert(id.clone()) {
                break;
            }
            let revision = repo.load_revision(&id)?;
            insert(RepositoryRoot::Revision(id))?;
            for object in revision.objects {
                let bytes = repo
                    .get_object(&object)?
                    .ok_or(RetentionError::Unavailable)?;
                if Digest::object_digest(&bytes).to_hex() != object {
                    return Err(Failure::Corrupt.into());
                }
                insert(RepositoryRoot::Object(object))?;
            }
            next = revision.parent;
        }
    }
    Ok(roots.into_values().collect())
}

/// Explicit canonical checkpoint publication; no source body is copied into Graph.
/// The caller already owns the repository writer fence, including filesystem locks.
pub fn publish<R: Repository + Backend + RetentionStore>(
    repo: &mut R,
    head: &str,
    operation: &str,
    expected: &RevisionRef,
    input: &RestorationInput,
) -> Result<CommitReceipt> {
    // Reject an authority substitution before staging objects or publishing a
    // canonical head. Binding is persisted only by explicit registration.
    let mut metadata = load(repo)?;
    bind(&mut metadata, "sem-lang/repository")?;
    let roots = closure(repo, input)?;
    let prepared = head_intent(
        head,
        operation,
        expected,
        &HeadPublication {
            root: RepositoryRoot::Snapshot(input.runtime_snapshot.clone()),
            request_digest: Digest::object_digest(
                &serde_json::to_vec(input).map_err(|_| Failure::Corrupt)?,
            ),
        },
    )?;
    if let Some(receipt) = commits(repo).receipt(head, operation)? {
        if receipt.prepared_ref != prepared.reference() {
            return Err(RetentionError::Mismatch);
        }
        // A lost response may have interrupted registration after the canonical
        // commit. A publication retry must finish that explicit operation.
        register(repo, &RepositoryPublication, &receipt, input)?;
        return Ok(receipt);
    }
    if commits(repo).check_base(head, expected)?.is_some() {
        return Err(RetentionError::Mismatch);
    }
    if metadata.groups.len() >= MAX_GROUPS {
        return Err(RetentionError::Capacity);
    }
    commits(repo).reserve_external(
        &prepared,
        &roots
            .iter()
            .map(external_root)
            .collect::<std::result::Result<Vec<_>, _>>()?,
    )?;
    let receipt = repo.publish_head(&prepared)?;
    register(repo, &RepositoryPublication, &receipt, input)?;
    Ok(receipt)
}

/// Explicit historical repair. Pure reads never create a registration.
pub fn register<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &mut R,
    publication: &P,
    receipt: &CommitReceipt,
    input: &RestorationInput,
) -> Result<Vec<ExternalRegistration>> {
    exact(repo, publication, receipt, input)?;
    let roots = closure(repo, input)?;
    let mut metadata = load(repo)?;
    bind(&mut metadata, publication.identity())?;
    if let Some(group) = metadata.groups.get(&receipt.commit_ref) {
        if group.retired || group.input != *input {
            return Err(RetentionError::Unavailable);
        }
        if !group.registrations.is_empty() {
            return Ok(group.registrations.clone());
        }
    }
    if !metadata.groups.contains_key(&receipt.commit_ref) && metadata.groups.len() >= MAX_GROUPS {
        return Err(RetentionError::Capacity);
    }
    metadata.groups.insert(
        receipt.commit_ref.clone(),
        Group {
            receipt: receipt.clone(),
            input: input.clone(),
            registrations: vec![],
            retired: false,
        },
    );
    save(repo, &metadata)?;
    let registrations = publication.retention(repo).register_committed_external(
        receipt,
        &roots
            .iter()
            .map(external_root)
            .collect::<std::result::Result<Vec<_>, _>>()?,
    )?;
    metadata.groups.insert(
        receipt.commit_ref.clone(),
        Group {
            receipt: receipt.clone(),
            input: input.clone(),
            registrations: registrations.clone(),
            retired: false,
        },
    );
    save(repo, &metadata)?;
    Ok(registrations)
}

fn hold(lease: &SourceLease) -> ExternalRoot {
    ExternalRoot {
        kind: ExternalRootKind::InFlight,
        holder: format!("sem-lang/source/{}", lease.generation),
    }
}
pub fn acquire<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &mut R,
    publication: &P,
    receipt: &CommitReceipt,
    input: &RestorationInput,
    holder: &str,
    now: u64,
    expires: u64,
) -> Result<SourceLease> {
    if holder.is_empty()
        || holder.len() > 128
        || holder.chars().any(char::is_control)
        || expires <= now
        || expires - now > MAX_LEASE_MS
    {
        return Err(RetentionError::Mismatch);
    }
    exact(repo, publication, receipt, input)?;
    // Owner expiry is independent of the projection process. Validation occurs
    // before maintenance; a foreign or malformed request cannot mutate leases.
    reconcile(repo, publication, now)?;
    let mut metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    let group = metadata
        .groups
        .get(&receipt.commit_ref)
        .ok_or(RetentionError::Unavailable)?
        .clone();
    if group.retired || group.registrations.is_empty() {
        return Err(RetentionError::Unavailable);
    }
    let lease = if let Some(record) = metadata.leases.values().find(|r| {
        r.lease.holder == holder && r.lease.receipt == *receipt && r.lease.input == *input
    }) {
        let lease = &record.lease;
        if record.releasing || lease.receipt != *receipt || lease.input != *input {
            return Err(RetentionError::Mismatch);
        }
        lease.clone()
    } else {
        if metadata.leases.len() >= MAX_LEASES {
            return Err(RetentionError::Capacity);
        }
        metadata.sequence = metadata
            .sequence
            .checked_add(1)
            .ok_or(RetentionError::Capacity)?;
        let lease = SourceLease {
            generation: metadata.sequence,
            holder: holder.into(),
            receipt: receipt.clone(),
            input: input.clone(),
            expires_at_ms: expires,
        };
        metadata.leases.insert(
            lease.generation,
            LeaseRecord {
                lease: lease.clone(),
                acquired: false,
                releasing: false,
            },
        );
        // Intent before the first hold: a crash never leaves an unaccounted root.
        save(repo, &metadata)?;
        lease
    };
    for registration in &group.registrations {
        let retained = publication
            .retention(repo)
            .retain_registration(registration, &hold(&lease));
        if let Err(error) = retained {
            release(repo, publication, &lease)?;
            return Err(error.into());
        }
    }
    metadata
        .leases
        .get_mut(&lease.generation)
        .ok_or(Failure::Corrupt)?
        .acquired = true;
    save(repo, &metadata)?;
    Ok(lease)
}
pub fn read<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &R,
    publication: &P,
    lease: &SourceLease,
    now: u64,
) -> Result<()> {
    let metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    let record = metadata
        .leases
        .get(&lease.generation)
        .ok_or(RetentionError::Unavailable)?;
    if record.lease != *lease || !record.acquired || record.releasing || now >= lease.expires_at_ms
    {
        return Err(RetentionError::Unavailable);
    }
    exact(repo, publication, &lease.receipt, &lease.input)
}

/// Resolve a durable opaque lease generation inside its owner. Dependency
/// membership stays private; callers may use this input only after visibility
/// validation at their own boundary. This operation never creates or renews.
pub fn leased_input<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &R,
    publication: &P,
    holder: &str,
    generation: u64,
    now: u64,
) -> Result<(CommitReceipt, RestorationInput)> {
    let metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    let record = metadata
        .leases
        .get(&generation)
        .ok_or(RetentionError::Unavailable)?;
    if record.lease.holder != holder {
        return Err(RetentionError::Mismatch);
    }
    read(repo, publication, &record.lease, now)?;
    Ok((record.lease.receipt.clone(), record.lease.input.clone()))
}
pub fn release<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &mut R,
    publication: &P,
    lease: &SourceLease,
) -> Result<()> {
    let mut metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    let Some(record) = metadata.leases.get_mut(&lease.generation) else {
        return Ok(());
    };
    if record.lease != *lease {
        return Err(RetentionError::Mismatch);
    }
    record.releasing = true;
    save(repo, &metadata)?;
    let group = metadata
        .groups
        .get(&lease.receipt.commit_ref)
        .ok_or(Failure::Corrupt)?;
    for registration in &group.registrations {
        publication
            .retention(repo)
            .release_registration(registration, &hold(lease))?;
    }
    metadata.leases.remove(&lease.generation);
    save(repo, &metadata)
}
pub fn reconcile<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &mut R,
    publication: &P,
    now: u64,
) -> Result<usize> {
    let expired: Vec<_> = load(repo)?
        .leases
        .into_values()
        .filter(|r| r.releasing || r.lease.expires_at_ms <= now)
        .map(|r| r.lease)
        .collect();
    for lease in &expired {
        release(repo, publication, lease)?;
    }
    Ok(expired.len())
}

/// Release every exact source held by a durable job identity, including an
/// acquisition whose response was lost. Does not require the caller to enumerate
/// owner dependency objects or copy lease bodies into another owner.
pub fn release_holder<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &mut R,
    publication: &P,
    holder: &str,
) -> Result<usize> {
    if holder.is_empty() || holder.len() > 128 || holder.chars().any(char::is_control) {
        return Err(RetentionError::Mismatch);
    }
    let metadata = load(repo)?;
    // An unprovisioned owner has no holds to release. Never initialize on cleanup.
    if metadata.publication_owner.is_none() {
        return Ok(0);
    }
    bound(&metadata, publication.identity())?;
    let leases: Vec<_> = metadata
        .leases
        .into_values()
        .filter(|r| r.lease.holder == holder)
        .map(|r| r.lease)
        .collect();
    for lease in &leases {
        release(repo, publication, lease)?;
    }
    Ok(leases.len())
}

/// Exact restart-safe release using only owner generation and durable holder.
/// A stale generation never selects another lease belonging to the same job.
pub fn release_generation<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &mut R,
    publication: &P,
    holder: &str,
    generation: u64,
) -> Result<()> {
    if holder.is_empty()
        || holder.len() > 128
        || holder.chars().any(char::is_control)
        || generation == 0
    {
        return Err(RetentionError::Mismatch);
    }
    let metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    if let Some(record) = metadata.leases.get(&generation) {
        if record.lease.holder != holder {
            return Err(RetentionError::Mismatch);
        }
        release(repo, publication, &record.lease)?;
    }
    Ok(())
}

fn is_current<R: Repository + Backend, P: SourceRequirements<R>>(
    repo: &R,
    publication: &P,
    input: &RestorationInput,
) -> Result<bool> {
    Ok(required_sources(repo, publication)?.contains(input))
}

fn required_sources<R: Repository + Backend, P: SourceRequirements<R>>(
    repo: &R,
    publication: &P,
) -> Result<Vec<RestorationInput>> {
    let inputs = publication.required_sources(repo)?;
    if inputs.len() > MAX_GROUPS {
        return Err(RetentionError::Capacity);
    }
    Ok(inputs)
}
pub fn retire<
    R: Repository + Backend + RetentionStore,
    P: SourcePublication<R> + SourceRequirements<R>,
>(
    repo: &mut R,
    publication: &P,
    receipt: &CommitReceipt,
    input: &RestorationInput,
) -> Result<()> {
    publication.verify_receipt(repo, receipt)?;
    let mut metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    let group = metadata
        .groups
        .get(&receipt.commit_ref)
        .ok_or(RetentionError::Unavailable)?;
    if group.input != *input || group.registrations.is_empty() {
        return Err(RetentionError::Mismatch);
    }
    // Replay an already persisted retirement intent, not a fresh policy decision.
    // A later publication may independently require the same immutable bytes.
    if !group.retired
        && (is_current(repo, publication, input)?
            || metadata
                .leases
                .values()
                .any(|r| r.lease.receipt == *receipt))
    {
        return Err(RetentionError::Required);
    }
    let group = metadata
        .groups
        .get_mut(&receipt.commit_ref)
        .ok_or(RetentionError::Unavailable)?;
    if group.input != *input || group.registrations.is_empty() {
        return Err(RetentionError::Mismatch);
    }
    // Write intent first; concurrent/restarted admission sees the retirement fence.
    group.retired = true;
    let registrations = group.registrations.clone();
    save(repo, &metadata)?;
    for registration in registrations {
        publication
            .retention(repo)
            .retire_external_registration(&registration)?;
    }
    Ok(())
}

/// Physical maintenance is owner-only. PP has no retirement/reclamation port.
pub fn reclaim<
    R: Repository + Backend + RetentionStore,
    P: SourcePublication<R> + SourceRequirements<R>,
>(
    repo: &mut R,
    publication: &P,
) -> Result<usize> {
    let metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    let mut required = current_dependencies(repo)?;
    for input in required_sources(repo, publication)?.into_iter().chain(
        metadata
            .groups
            .values()
            .filter(|g| !g.retired)
            .map(|g| g.input.clone()),
    ) {
        for root in closure(repo, &input)? {
            required.insert(external_root(&root)?.object);
        }
        if required.len() > 4096 {
            return Err(RetentionError::Capacity);
        }
    }
    let mut removed = BTreeSet::new();
    for group in metadata.groups.values().filter(|g| g.retired) {
        for registration in &group.registrations {
            // Reconcile retirement interrupted before its Graph fence was durable.
            publication
                .retention(repo)
                .retire_external_registration(registration)?;
            if publication
                .retention(repo)
                .registration_status(registration)?
                != ExternalRegistrationStatus::Retired
            {
                continue;
            }
            let root: RepositoryRoot =
                serde_json::from_str(&registration.object.object).map_err(|_| Failure::Corrupt)?;
            if required.contains(&registration.object.object) {
                continue;
            }
            let claimed = publication
                .retention(repo)
                .claim_external_reclamation(&registration.object, 0)?;
            if let Some(permit) = claimed {
                let permit = sem_lang_repository::retention::validate_permit(
                    &publication.retention(repo),
                    &root,
                    &permit,
                )?;
                repo.remove_retained_root(&root, &permit)?;
                publication
                    .retention(repo)
                    .complete_external_reclamation(permit.permit())?;
                removed.insert(registration.object.object.clone());
            }
        }
    }
    Ok(removed.len())
}
fn current_dependencies<R: Repository>(repo: &R) -> Result<BTreeSet<String>> {
    let mut required = BTreeSet::new();
    for current_root in repo.root_references()? {
        required.insert(external_root(&current_root)?.object);
        if let RepositoryRoot::Snapshot(id) = &current_root
            && id.starts_with("runtime-")
        {
            for root in closure(repo, &restoration::checkpoint_input(repo, id)?)? {
                required.insert(external_root(&root)?.object);
            }
        }
        // Revision ancestry is independently retained by adoption and must not
        // lose an object merely because the last checkpoint reader has ended.
        if let RepositoryRoot::Revision(id) = current_root {
            let mut next = Some(id);
            let mut seen = BTreeSet::new();
            while let Some(id) = next {
                if !seen.insert(id.clone()) || seen.len() > 4096 {
                    return Err(Failure::Corrupt.into());
                }
                let revision = repo.load_revision(&id)?;
                required.insert(external_root(&RepositoryRoot::Revision(id))?.object);
                for object in revision.objects {
                    required.insert(external_root(&RepositoryRoot::Object(object))?.object);
                }
                next = revision.parent;
            }
        }
        if required.len() > 4096 {
            return Err(RetentionError::Capacity);
        }
    }
    Ok(required)
}

/// Pure semantic-owner integrity check includes retained historical groups.
pub fn verify<R: Repository + Backend + RetentionStore, P: SourcePublication<R>>(
    repo: &R,
    publication: &P,
) -> Result<()> {
    let metadata = load(repo)?;
    bound(&metadata, publication.identity())?;
    for group in metadata.groups.values() {
        if group.retired {
            continue;
        }
        exact(repo, publication, &group.receipt, &group.input)?;
        let actual = closure(repo, &group.input)?
            .iter()
            .map(external_root)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if actual
            != group
                .registrations
                .iter()
                .map(|r| r.object.clone())
                .collect::<Vec<_>>()
        {
            return Err(Failure::Corrupt.into());
        }
        for registration in &group.registrations {
            if publication
                .retention(repo)
                .registration_status(registration)?
                != ExternalRegistrationStatus::Active
            {
                return Err(Failure::Corrupt.into());
            }
        }
    }
    Ok(())
}
