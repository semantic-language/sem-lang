# sem-lang-context

Context bindings, context holes, short-term memory, and contextual default candidates.

Induction is read-only. Promotion is a separate explicit operation requiring a
decision reference, retained in the accepted default and its revision. Evidence
quality checks do not grant promotion authority. Applications may automate the
decision but must still invoke the same explicit operation; the language never
automatically promotes candidates.

## Scoped persistence

`ContextStore::create/load/publish/publish_promotion` owns a bounded file image and its OS lock;
`save_to_repository/load_from_repository` uses immutable repository objects.
Reads never initialize missing data. The private storage transaction publishes the proposed semantic
image and its commit state together, with file synchronization and atomic
replacement; exact replay does not rewrite the file.

Every typed ContextOperation and promotion/correction uses the registry-provided, core-only Zixcel commit
foundation. Requests include `expected_commit_revision`, a semantic input digest
(`expected_memory_revision`), and an explicit decision reference. The foundation
owns commit identity, CAS, receipt indexing and exact replay. sem-lang checks
candidate meaning, scope and correction rules before proposing any persistent
object. A replay returns its original semantic result even after a later
correction or short-term clearing. No receipt map or secondary checkpoint API
is retained. Semantic NoChange preserves the meaning digest; a distinct accepted
decision still has its own minimal foundation commit record.

ContextRequest carries the Foundation OperationId, expected_commit_revision,
expected_memory_revision, subject_ref, scope and a typed ContextOperation.
Retain wraps the existing ObservationRequest; Discard and Clear select owner refs.
Timezone observation evidence must be provided explicitly; counter-based IDs and
automatic current-head selection are not part of the API. CLI uses context apply
with the same request. Bare mutation commands and arbitrary update closures were removed.

The semantic input digest is not a second commit revision: it binds the exact
observations used by the decision. Reducer and repository-head publication are separate responsibilities; context
publication alone does not provide an application-wide transaction.

Contexts are bounded to 1 MiB during reading and writing, independently of any
retention period. The package adds no default TTL. `DiscardAfterCompile` never
retains the original surface. Clearing short-term entries replaces the current
checkpoint without preserving old short-term bodies in a revision archive.
Clear targets are explicit references, checked for subject and scope. The scoped
ClearShortTerm operation also removes candidate-producing observations and temporary
active/session timezone bindings for that exact subject and scope. Otherwise cleared knowledge could silently
reappear through induction or resolution. Accepted contextual defaults and
explicit interpretation rules remain intact. Definition and fact repositories
are separate and are not touched by context operations.

This is structured context persistence, not a general surface compiler. The
application must authorize role ownership, select a language frontend and
verify promotion decisions independently. Filesystem failures and conflicts
are errors, never an empty context or a request to infer replacement data.
