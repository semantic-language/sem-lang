//! Atomic project template creation.
//!
//! A project stores selected definitions and non-secret service declarations.
//! Authentication material remains with the referenced Zixcel/Crowsi owner.

use crate::{
    ProjectManifest, ProjectSourceDocument, ProjectSourceRole, SCHEMA_VERSION,
    SourceVersioningPresentation, ToolingError,
};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_DEFINITION_BYTES: u64 = 1_048_576;
pub use crate::project_source::{
    PROJECT_SOURCE_PATH, RUNTIME_SERVICES_PATH, SERVICES_PATH, WORKSPACE_GRAPH_PATH,
};
pub const PRIVATE_STATE_PATH: &str = ".sem-lang";
pub const PRIVATE_DATA_PATH: &str = ".sem-lang/data";
pub const RUNTIME_PATH: &str = ".sem-lang/runtime";
pub const RUNTIME_RECEIPTS_PATH: &str = ".sem-lang/runtime/receipts";

/// Read the explicitly selected version-control workspace without changing
/// project state. Version control is optional and absence is a normal state.
pub fn observe_source_versioning(
    project_source: impl AsRef<Path>,
) -> Result<SourceVersioningPresentation, ToolingError> {
    let project_source = project_source.as_ref();
    let source = fs::read_to_string(project_source)?;
    let compiled = crate::project_source::compile(&source)?;
    let Some(selection) = compiled.manifest.version_control else {
        return Ok(SourceVersioningPresentation {
            state: "not-configured".into(),
            match_state: "not-applicable".into(),
            provider: None,
            root: None,
            observation: None,
            detail: None,
        });
    };
    let base = project_source
        .parent()
        .ok_or_else(|| ToolingError::Project("project.sem has no project directory".into()))?;
    let observed_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| ToolingError::Project(error.to_string()))?;
    match zixcel_version_control::GitObserver::observe(base.join(&selection.root), &observed_at) {
        Ok(Some(observation)) => {
            let match_state = match observation.working_state {
                zixcel_version_control::WorkingState::Clean
                    if observation.immutable_revision_ref.is_some() =>
                {
                    "exact"
                }
                zixcel_version_control::WorkingState::Clean => "unborn",
                _ => "working-copy",
            };
            Ok(SourceVersioningPresentation {
                state: "observed".into(),
                match_state: match_state.into(),
                provider: Some(selection.provider),
                root: Some(selection.root),
                observation: Some(observation),
                detail: None,
            })
        }
        Ok(None) => Ok(SourceVersioningPresentation {
            state: "unavailable".into(),
            match_state: "unavailable".into(),
            provider: Some(selection.provider),
            root: Some(selection.root),
            observation: None,
            detail: Some("the selected directory is not a Git repository".into()),
        }),
        Err(error) => Ok(SourceVersioningPresentation {
            state: "unavailable".into(),
            match_state: "unavailable".into(),
            provider: Some(selection.provider),
            root: Some(selection.root),
            observation: None,
            detail: Some(error.to_string()),
        }),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitializedProject {
    pub root: PathBuf,
    pub project: PathBuf,
    pub workspace_graph: PathBuf,
    pub services_root: PathBuf,
    pub runtime_services_root: PathBuf,
    pub runtime_root: PathBuf,
}

/// Initialize only the durable storage selected by an existing `project.sem`.
/// This is an explicit migration operation for authored projects; normal load
/// and compile paths never create missing storage as a fallback.
pub fn initialize_storage(
    project_source: impl AsRef<Path>,
) -> Result<crate::ProjectCompilationReceipt, ToolingError> {
    let project_source = project_source.as_ref();
    let metadata = fs::symlink_metadata(project_source)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_DEFINITION_BYTES
    {
        return Err(ToolingError::Project(
            "project.sem must be a bounded regular file".into(),
        ));
    }
    let source = fs::read_to_string(project_source)?;
    let compiled = crate::project_source::compile(&source)?;
    let graph_ref = compiled
        .manifest
        .workspace_graph
        .as_deref()
        .ok_or_else(|| {
            ToolingError::Project(
                "compiled project manifest is missing its derived workspace graph".into(),
            )
        })?;
    let base = project_source
        .parent()
        .ok_or_else(|| ToolingError::Project("project.sem has no project directory".into()))?;
    crate::workspace_graph::create(base, graph_ref)?;
    match crate::compile_project(project_source) {
        Ok((_, receipt)) => Ok(receipt),
        Err(error) => {
            let _ = fs::remove_file(base.join(graph_ref));
            Err(error)
        }
    }
}

/// Explicitly remove only the derived compilation generation selected by an
/// existing project. This is used when the source contract itself changes;
/// accepted workspace records remain intact and the next compile republishes
/// their new deterministic compiler view.
pub fn retire_compilation(
    project_source: impl AsRef<Path>,
) -> Result<crate::ProjectCompilationRetirementReceipt, ToolingError> {
    let project_source = project_source.as_ref();
    let source = fs::read_to_string(project_source)?;
    let compiled = crate::project_source::compile(&source)?;
    let graph_ref = compiled
        .manifest
        .workspace_graph
        .as_deref()
        .ok_or_else(|| {
            ToolingError::Project(
                "compiled project manifest is missing its derived workspace graph".into(),
            )
        })?;
    let base = project_source
        .parent()
        .ok_or_else(|| ToolingError::Project("project.sem has no project directory".into()))?;
    crate::workspace_graph::retire_project_compilation(
        base,
        graph_ref,
        &compiled.manifest.project_id,
    )
}

/// Create a complete project in a new directory. Existing destinations are
/// rejected; initialization never merges with or overwrites user data.
pub fn initialize(
    destination: impl AsRef<Path>,
    project_id: &str,
    locale: &str,
) -> Result<InitializedProject, ToolingError> {
    let destination = destination.as_ref();
    if destination.exists() {
        return Err(ToolingError::Capability(
            "project destination already exists".into(),
        ));
    }
    if !zixcel_interaction::valid_reference(project_id)
        || locale.is_empty()
        || locale.len() > 64
        || locale.chars().any(char::is_control)
    {
        return Err(ToolingError::Capability(
            "project identity or locale is invalid".into(),
        ));
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .map_or_else(std::env::current_dir, Ok)?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ToolingError::Capability("project directory name is invalid".into()))?;
    let staging = (0_u8..16)
        .map(|attempt| {
            parent.join(format!(
                ".{name}.sem-lang-init-{}-{attempt}",
                std::process::id()
            ))
        })
        .find(|path| !path.exists())
        .ok_or_else(|| ToolingError::Capability("project staging directory is busy".into()))?;

    let result = create_staged_project(&staging, project_id, locale).and_then(|_| {
        fs::rename(&staging, destination)?;
        Ok(layout(destination))
    });
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn create_staged_project(root: &Path, project_id: &str, locale: &str) -> Result<(), ToolingError> {
    for relative in [
        PRIVATE_DATA_PATH,
        SERVICES_PATH,
        RUNTIME_SERVICES_PATH,
        RUNTIME_RECEIPTS_PATH,
    ] {
        fs::create_dir_all(root.join(relative))?;
    }
    for relative in [
        PRIVATE_STATE_PATH,
        PRIVATE_DATA_PATH,
        SERVICES_PATH,
        RUNTIME_PATH,
        RUNTIME_SERVICES_PATH,
        RUNTIME_RECEIPTS_PATH,
    ] {
        private_runtime_permissions(&root.join(relative))?;
    }

    let manifest = ProjectManifest {
        schema_version: SCHEMA_VERSION.into(),
        project_id: project_id.into(),
        locale: locale.into(),
        time_zone: None,
        semantic_packages: Vec::new(),
        source_documents: vec![ProjectSourceDocument {
            path: PROJECT_SOURCE_PATH.into(),
            role: ProjectSourceRole::ProjectLanguage,
        }],
        service_roots: vec![SERVICES_PATH.into()],
        service_state_roots: vec![RUNTIME_SERVICES_PATH.into()],
        behavior_packages: Vec::new(),
        procedure_packages: Vec::new(),
        workspace_graph: Some(WORKSPACE_GRAPH_PATH.into()),
        version_control: None,
    };

    let project_source = crate::project_source::render(&manifest, "")?;
    write_new(&root.join(PROJECT_SOURCE_PATH), project_source.as_bytes())?;
    crate::workspace_graph::create(root, WORKSPACE_GRAPH_PATH)?;
    write_new(&root.join(".gitignore"), b".sem-lang/\n")?;
    write_new(&root.join("README.md"), PROJECT_README.as_bytes())?;
    crate::compile_project(root.join(PROJECT_SOURCE_PATH))?;
    Ok(())
}

fn layout(root: &Path) -> InitializedProject {
    InitializedProject {
        root: root.into(),
        project: root.join(PROJECT_SOURCE_PATH),
        workspace_graph: root.join(WORKSPACE_GRAPH_PATH),
        services_root: root.join(SERVICES_PATH),
        runtime_services_root: root.join(RUNTIME_SERVICES_PATH),
        runtime_root: root.join(RUNTIME_PATH),
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ToolingError> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn private_runtime_permissions(path: &Path) -> Result<(), ToolingError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn private_runtime_permissions(_path: &Path) -> Result<(), ToolingError> {
    Ok(())
}

const PROJECT_README: &str = r#"# sem-lang project

This project contains selected semantic definitions and non-secret declarations.

```text
project.sem                                  project selection and structural language
work/<wbs-path>.sem                          one authored WBS work specification
records/<definition>.sem                     record retention and selection
contributions/<digest>.sem                   authored semantic contributions
packages/<owner>/<name>/package.sem          selected semantic package authority
packages/<owner>/<name>/language/*.sem       package-owned language sources
packages/<owner>/<name>/modules/**/*.sem     package-owned semantic modules
.sem-lang/data/workspace.redb                canonical typed project graph
.sem-lang/services/<meaning>/capability.json
                                             internal operation contract
.sem-lang/packages/<owner>/<name>/behavior-package.json
                                             selected behavior declaration
.sem-lang/packages/<owner>/<name>/procedure-package.json
                                             selected inert procedure data
.sem-lang/services/authentication/<provider>/<account>/authentication.json
                                             internal non-secret reference
.sem-lang/runtime/services/authentication/<provider>/<account>/state.json
                                     one non-secret service state projection
.sem-lang/runtime/receipts/          generated operation receipts
```

Every selected `packages/<owner>/<name>/package.sem` is a reviewable,
version-controlled semantic package selected by its exact digest. It binds its
language sources and semantic modules by relative path and digest. Declared
package dependencies must also be selected; declaration order is not semantic.
The package identity comes from `<owner>/<name>`, and a contained source
identity comes from its package-relative path. Package-owned `.sem` sources do
not repeat those identities or the package version. Physical traversal uses
the host path API; `/` is only the portable logical/wire representation after
path components have been validated.
`.sem-lang/` is the private machine-owned
project area and is excluded from version control. It contains generated state,
inference intermediates, operation contracts, and receipts. It
must not contain access tokens, passwords, private keys, authorization codes,
or browser sessions. Each fixed filename is a document kind and its parent path
is the semantic identity. For example, `.sem-lang/services/calendar/personal/capability.json`
defines `calendar/personal`; an ID is never duplicated inside that document.
Authentication definitions contain only an opaque `configuration_ref`. Current
readiness is a separate, non-secret mirrored projection under `.sem-lang/`.
Actual credential material is stored and operated on by the referenced
Zixcel/Crowsi custody owner, outside this project.

Multiple accounts for one provider are separate authentication paths and
configuration references. A capability binds to exactly one authentication
path; changing an active account is an explicit definition change, never an
implicit environment lookup.

`振る舞いパッケージ` (`Behavior package`) sections in `project.sem` select every external behavior
package by a project-relative manifest and exact SHA-256. A package may bind
fixed behavior interfaces to owner operations and depend on other interfaces;
it cannot add grammar, semantic definitions, credentials, paths, loaders or
transport endpoints. The editor reports unresolved or ambiguous injection
instead of guessing an implementation.

`手順パッケージ` (`Procedure package`) sections select immutable reusable ways of working. A procedure
defines ordered steps, input/output roles, completion conditions, actor roles,
and operation requirements. It is inert data: selection cannot start a run,
change a WBS, authenticate, or execute an operation. Executable behavior remains
an independent Zixcel capability and behavior-DI concern.

`意味パッケージ` (`Semantic package`) sections select exact `package.sem`
manifests. Modules are selected only through those manifests, never directly
from `project.sem` or by scanning a directory.
They compile subjects, organizations, public resources, relations and
procedure-role bindings to stable TermIds. Identity stays with its exact
external authority and customer payload stays with its owner.

`project.sem` is the only visible project selection and the highest project
language layer. Its preamble explicitly lists every selected `.sem` source
document. The first ordinary semantic section after its typed project sections
begins the project program. Each selected `work`
source specifies one concrete WBS responsibility and may bind to at most one
activity identity. The graph joins that source to its explicit actor, WBS
position, schedule, procedure run, inputs, outputs, and completion state.
Reusable review or manufacturing methods belong to procedure packages.
Project startup analyzes that exact list. Durable graph records may reference a
source digest and bounded line range; the graph never copies source text. A
missing semantic definition remains feedback until an owner explicitly adopts
an exact candidate.
"#;
