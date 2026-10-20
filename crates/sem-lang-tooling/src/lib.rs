//! Editor-facing semantic analysis.
//!
//! This crate deliberately does not execute capabilities. Its analysis path is
//! read-only: it compiles text against an explicitly selected semantic
//! definition package, then checks references against exact path-addressed
//! service definitions supplied by the workspace owner. Explicit persistence
//! is isolated in `workspace_change`, where immutable plans are validated and
//! applied with compare-and-set receipts.

mod document_analysis;
mod issue;
mod model;
mod package_loading;
mod path_identity;
mod physical_input;
pub mod procedure;
pub mod project;
mod project_compilation;
pub mod project_source;
pub mod semantic_module;
pub mod semantic_package;
pub mod workspace;
pub mod workspace_change;
pub mod workspace_graph;

pub use issue::{ToolingError, ToolingIssue};
pub use model::*;
pub use project_compilation::compile_project;

use document_analysis::{line_range, uncovered_fragments};
use package_loading::*;
use physical_input::*;
use project_compilation::*;
use sem_lang_core::Digest;
use sem_lang_language::{CompiledLanguage, PipelineOperator, validate_pipeline_shape};
use sem_lang_linguistics::LinguisticKnowledge;
use sem_lang_vocab::{ContributionKind, VocabularyModule, VocabularyRegistry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use zixcel_contracts::{BehaviorPackageV1, parse_behavior_package_json};
use zixcel_interaction::{Availability, ValueSchema, valid_reference};

pub const SCHEMA_VERSION: &str = "0.10.0";
const MAX_SERVICE_DOCUMENT_BYTES: u64 = 1_048_576;
const MAX_PROJECT_DOCUMENT_BYTES: u64 = 1_048_576;
const MAX_SERVICE_DOCUMENTS: usize = 1_024;
const MAX_SERVICE_DEPTH: usize = 16;

pub struct WorkspaceAnalyzer {
    manifest: ProjectManifest,
    compilation: ProjectCompilation,
    vocabularies: Vec<VocabularyModule>,
    language: CompiledLanguage,
    operations: BTreeMap<String, Vec<(String, String, zixcel_interaction::ActionContract)>>,
    bindings: BTreeMap<String, Vec<CapabilityBinding>>,
    input_bindings: BTreeMap<String, Vec<CapabilityInputBinding>>,
    authentication: BTreeMap<String, ResolvedAuthenticationService>,
    target_authentication: BTreeMap<String, String>,
    behavior_packages: Vec<BehaviorPackagePresentation>,
    procedure_packages: Vec<procedure::ProcedurePackage>,
    semantic_packages: semantic_package::LoadedPackages,
    semantic_modules: Vec<semantic_module::CompiledSemanticModule>,
    workspace: Option<workspace::WorkspaceProjection>,
    project_sources: Vec<ProjectSourceInput>,
    standard_language_set: sem_lang_standard::StandardLanguageSetDescriptor,
}

struct ProjectSourceInput {
    document: ProjectSourceDocument,
    authored: String,
    analysis: String,
}

struct ResolvedAuthenticationService {
    declaration_owner: String,
    declaration_revision: String,
    service_id: String,
    service: AuthenticationDefinition,
    state_owner: String,
    state_revision: String,
    state: AuthenticationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceDocumentKind {
    Capability,
    Authentication,
    State,
}

struct ServiceDocument {
    id: String,
    kind: ServiceDocumentKind,
    path: PathBuf,
}

impl WorkspaceAnalyzer {
    pub fn load(project_file: impl AsRef<Path>) -> Result<Self, ToolingError> {
        let project_file = project_file.as_ref();
        if project_file.file_name().and_then(|value| value.to_str())
            != Some(project_source::PROJECT_SOURCE_PATH)
        {
            return Err(ToolingError::Project(
                "the canonical project definition must be named project.sem".into(),
            ));
        }
        let base = project_directory(project_file)?;
        validate_physical_file(project_file, "project source", MAX_PROJECT_DOCUMENT_BYTES)?;
        let project_text = fs::read_to_string(project_file)?;
        let project_source_digest = Digest::document_digest(project_text.as_bytes()).to_hex();
        let compiled_project = project_source::compile(&project_text)?;
        let manifest = compiled_project.manifest;
        check_version(&manifest.schema_version, "project source")?;
        validate_project_layout(&manifest)?;
        if !valid_reference(&manifest.project_id)
            || (!manifest.locale.is_empty() && !valid_locale(&manifest.locale))
            || manifest
                .time_zone
                .as_deref()
                .is_some_and(|value| !valid_time_zone(value))
        {
            return Err(ToolingError::Capability(
                "project identity, locale, or timezone is invalid".into(),
            ));
        }
        let semantic_packages = semantic_package::load(&base, &manifest.semantic_packages)?;
        let standard_set = sem_lang_standard::language_set()
            .map_err(|error| ToolingError::Vocabulary(error.to_string()))?;
        let mut language_paths = BTreeSet::new();
        let mut pending_vocabularies = standard_set
            .modules
            .iter()
            .cloned()
            .map(|module| (module.vocabulary_id.clone(), module))
            .collect::<BTreeMap<_, _>>();
        for selection in &semantic_packages.language_sources {
            if !language_paths.insert(selection.source.clone()) {
                return Err(ToolingError::Vocabulary(
                    "semantic package language source paths must be unique".into(),
                ));
            }
            let vocabulary_path = resolve_project_file(
                &base,
                &selection.source,
                "semantic package language source",
                MAX_PROJECT_DOCUMENT_BYTES,
            )?;
            let vocabulary_bytes = fs::read(&vocabulary_path)?;
            if Digest::document_digest(&vocabulary_bytes).to_hex() != selection.digest_sha256 {
                return Err(ToolingError::Vocabulary(format!(
                    "semantic package language source digest does not match package.sem: {}",
                    selection.source
                )));
            }
            let vocabulary_text = std::str::from_utf8(&vocabulary_bytes)
                .map_err(|_| ToolingError::Vocabulary("language source must be UTF-8".into()))?;
            let vocabulary = VocabularyModule::from_package_sem(
                &selection.source_identity,
                &selection.package_version,
                vocabulary_text,
            )
            .map_err(|error| ToolingError::Vocabulary(error.to_string()))?;
            check_version(&vocabulary.schema_version, "semantic definition package")?;
            check_version(&vocabulary.version, "semantic definition package")?;
            if pending_vocabularies
                .insert(vocabulary.vocabulary_id.clone(), vocabulary)
                .is_some()
            {
                return Err(ToolingError::Vocabulary(
                    "semantic package language source identities must be unique".into(),
                ));
            }
        }
        if manifest.source_documents.is_empty() || manifest.source_documents.len() > 256 {
            return Err(ToolingError::Capability(
                "project must declare between 1 and 256 source documents".into(),
            ));
        }
        let mut source_refs = BTreeSet::new();
        let mut project_sources = Vec::with_capacity(manifest.source_documents.len());
        for source in &manifest.source_documents {
            if !source.path.ends_with(".sem") || !source_refs.insert(source.path.clone()) {
                return Err(ToolingError::Capability(
                    "project source documents must be unique .sem files".into(),
                ));
            }
            if source.path == project_source::PROJECT_SOURCE_PATH {
                project_sources.push(ProjectSourceInput {
                    document: source.clone(),
                    authored: project_text.clone(),
                    analysis: compiled_project.analysis_source.clone(),
                });
            } else {
                let source_path = resolve_project_file(
                    &base,
                    &source.path,
                    "sem-lang source document",
                    MAX_PROJECT_DOCUMENT_BYTES,
                )?;
                let authored = fs::read_to_string(source_path)?;
                project_sources.push(ProjectSourceInput {
                    document: source.clone(),
                    analysis: authored.clone(),
                    authored,
                });
            }
        }
        let selected_vocabulary_ids = pending_vocabularies
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        for vocabulary in pending_vocabularies.values() {
            if let Some(dependency) = vocabulary
                .dependencies
                .iter()
                .find(|dependency| !selected_vocabulary_ids.contains(*dependency))
            {
                return Err(ToolingError::Vocabulary(format!(
                    "language source {} requires an unavailable language dependency {dependency}",
                    vocabulary.vocabulary_id
                )));
            }
        }
        let mut registry = VocabularyRegistry::new();
        while !pending_vocabularies.is_empty() {
            let ready = pending_vocabularies
                .iter()
                .find(|(_, vocabulary)| {
                    vocabulary
                        .dependencies
                        .iter()
                        .all(|dependency| registry.get(dependency).is_some())
                })
                .map(|(id, _)| id.clone())
                .ok_or_else(|| {
                    ToolingError::Vocabulary(
                        "semantic package language dependencies contain a cycle".into(),
                    )
                })?;
            let vocabulary = pending_vocabularies
                .remove(&ready)
                .expect("ready vocabulary remains selected");
            registry
                .register(vocabulary)
                .map_err(|error| ToolingError::Vocabulary(error.to_string()))?;
        }
        let vocabularies = registry.modules().cloned().collect::<Vec<_>>();
        let language_revision =
            Digest::from_serializable(1, &(&standard_set.descriptor, &semantic_packages.packages))
                .map_err(|error| ToolingError::Vocabulary(error.to_string()))?
                .to_hex();
        let language =
            CompiledLanguage::build(&LinguisticKnowledge::new(), &registry, language_revision)
                .map_err(|error| ToolingError::Vocabulary(error.to_string()))?;
        let semantic_terms = vocabularies
            .iter()
            .flat_map(|vocabulary| &vocabulary.terms)
            .map(|term| term.id.to_hex())
            .collect::<BTreeSet<_>>();
        let mut operations: BTreeMap<
            String,
            Vec<(String, String, zixcel_interaction::ActionContract)>,
        > = BTreeMap::new();
        let mut resource_ids = BTreeSet::new();
        let mut bindings: BTreeMap<String, Vec<CapabilityBinding>> = BTreeMap::new();
        let mut input_bindings: BTreeMap<String, Vec<CapabilityInputBinding>> = BTreeMap::new();
        let mut declared_authentication = BTreeMap::new();
        let mut target_authentication = BTreeMap::new();
        let service_documents = discover_service_documents(&base, &manifest.service_roots, false)?;
        for document in &service_documents {
            match document.kind {
                ServiceDocumentKind::Capability => {
                    let definition: CapabilityDefinition = read_json_bounded(&document.path)?;
                    check_version(&definition.schema_version, "capability definition")?;
                    let resource = definition.resource;
                    if document.id != resource.resource_id
                        || !valid_reference(&definition.owner)
                        || !valid_reference(&definition.revision)
                        || !valid_reference(&resource.contract_revision)
                        || !resource.value_schema.is_valid()
                        || resource.operations.len() > 128
                        || !resource_ids.insert(resource.resource_id.clone())
                        || resource.operations.iter().any(|operation| {
                            operation.target != resource.resource_id
                                || !valid_reference(&operation.operation_id)
                                || !valid_reference(&operation.contract_revision)
                                || !operation.input_schema.is_valid()
                        })
                    {
                        return Err(ToolingError::Capability(format!(
                            "capability path {} does not match a valid resource",
                            document.id
                        )));
                    }
                    for binding in &definition.semantic_bindings {
                        let operation_exists = resource
                            .operations
                            .iter()
                            .any(|operation| operation.operation_id == binding.operation_id);
                        if !semantic_terms.contains(&binding.term_id) || !operation_exists {
                            return Err(ToolingError::Capability(format!(
                                "{} refers to an unknown term or operation {}",
                                binding.term_id, binding.operation_id
                            )));
                        }
                        bindings
                            .entry(binding.term_id.clone())
                            .or_default()
                            .push(binding.clone());
                    }
                    for binding in &definition.input_bindings {
                        let matching_operations = resource
                            .operations
                            .iter()
                            .filter(|operation| operation.operation_id == binding.operation_id)
                            .collect::<Vec<_>>();
                        if !semantic_terms.contains(&binding.term_id)
                            || !valid_pointer(&binding.input_path)
                            || matching_operations.is_empty()
                            || matching_operations.iter().any(|operation| {
                                schema_at_pointer(&operation.input_schema, &binding.input_path)
                                    .is_none()
                            })
                        {
                            return Err(ToolingError::Capability(
                                "input binding does not resolve to a declared operation field"
                                    .into(),
                            ));
                        }
                        input_bindings
                            .entry(binding.operation_id.clone())
                            .or_default()
                            .push(binding.clone());
                    }
                    if let Some(authentication) = definition.authentication
                        && (!valid_reference(&authentication)
                            || target_authentication
                                .insert(resource.resource_id.clone(), authentication)
                                .is_some())
                    {
                        return Err(ToolingError::Capability(format!(
                            "resource {} has an invalid authentication reference",
                            resource.resource_id
                        )));
                    }
                    for operation in resource.operations {
                        operations
                            .entry(operation.operation_id.clone())
                            .or_default()
                            .push((
                                definition.owner.clone(),
                                definition.revision.clone(),
                                operation,
                            ));
                    }
                }
                ServiceDocumentKind::Authentication => {
                    let definition: AuthenticationDefinition = read_json_bounded(&document.path)?;
                    check_version(&definition.schema_version, "authentication definition")?;
                    if !document.id.starts_with("authentication/")
                        || !valid_reference(&definition.owner)
                        || !valid_reference(&definition.revision)
                        || !valid_configuration_ref(&definition.configuration_ref)
                        || definition.label.trim().is_empty()
                        || declared_authentication
                            .insert(
                                document.id.clone(),
                                (
                                    definition.owner.clone(),
                                    definition.revision.clone(),
                                    definition,
                                ),
                            )
                            .is_some()
                    {
                        return Err(ToolingError::Capability(format!(
                            "authentication path {} is invalid or duplicated",
                            document.id
                        )));
                    }
                }
                ServiceDocumentKind::State => unreachable!("state files are not stable services"),
            }
        }
        if target_authentication.iter().any(|(target, service)| {
            !resource_ids.contains(target) || !declared_authentication.contains_key(service)
        }) {
            return Err(ToolingError::Capability(
                "capability refers to an unknown authentication service".into(),
            ));
        }
        let mut authentication_states = BTreeMap::new();
        for document in discover_service_documents(&base, &manifest.service_state_roots, true)? {
            let state: AuthenticationStateProjection = read_json_bounded(&document.path)?;
            check_version(&state.schema_version, "authentication state projection")?;
            if document.kind != ServiceDocumentKind::State
                || !document.id.starts_with("authentication/")
                || !valid_reference(&state.owner)
                || !valid_reference(&state.revision)
                || !declared_authentication.contains_key(&document.id)
                || authentication_states
                    .insert(
                        document.id.clone(),
                        (state.owner, state.revision, state.state),
                    )
                    .is_some()
            {
                return Err(ToolingError::Capability(
                    "authentication state path is invalid, unknown, or duplicated".into(),
                ));
            }
        }
        if declared_authentication.len() != authentication_states.len() {
            return Err(ToolingError::Capability(
                "every declared authentication service requires one runtime state projection"
                    .into(),
            ));
        }
        let authentication = declared_authentication
            .into_iter()
            .map(|(id, (owner, revision, service))| {
                let (state_owner, state_revision, state) = authentication_states[&id].clone();
                (
                    id.clone(),
                    ResolvedAuthenticationService {
                        declaration_owner: owner,
                        declaration_revision: revision,
                        service_id: id,
                        service,
                        state_owner,
                        state_revision,
                        state,
                    },
                )
            })
            .collect();
        let behavior_packages = load_behavior_packages(&base, &manifest, &operations)?;
        let procedure_packages = load_procedure_packages(&base, &manifest)?;
        let semantic_modules = load_semantic_modules(
            &base,
            &semantic_packages.semantic_modules,
            &procedure_packages,
        )?;
        validate_value_binding_terms(&semantic_modules, &vocabularies)?;
        let semantic_revision = language.digest.to_hex();
        let compilation = build_project_compilation(ProjectCompilationContext {
            base: &base,
            manifest: &manifest,
            semantic_packages: &semantic_packages,
            standard_set: &standard_set.descriptor,
            source_digest: &project_source_digest,
            semantic_revision: &semantic_revision,
            sources: &project_sources,
            service_documents: &service_documents,
        })?;
        let workspace = manifest
            .workspace_graph
            .as_deref()
            .map(|graph| workspace_graph::load(&base, graph, &manifest.project_id))
            .transpose()?;
        Ok(Self {
            manifest,
            compilation,
            vocabularies,
            language,
            operations,
            bindings,
            input_bindings,
            authentication,
            target_authentication,
            behavior_packages,
            procedure_packages,
            semantic_packages,
            semantic_modules,
            workspace,
            project_sources,
            standard_language_set: standard_set.descriptor.clone(),
        })
    }

    /// Return the validated project identity used by this analyzer.
    ///
    /// Editor clients may display this read-only contract, but they do not
    /// receive a mutation handle or any credential-bearing service state.
    pub fn project_manifest(&self) -> &ProjectManifest {
        &self.manifest
    }

    /// Return the complete validated, read-only project context shared by
    /// editor clients and coding-agent adapters. The context contains no
    /// credential bodies, executable handles, mutation authority, or source
    /// payloads. Callers must use the separate plan/apply boundary for writes.
    pub fn project_context(&self, source_versioning: SourceVersioningPresentation) -> Value {
        let analysis = self.analyze("");
        let mut diagnostics = self.project_diagnostics();
        diagnostics.extend(analysis.diagnostics.iter().cloned());
        json!({
            "schema_version": SCHEMA_VERSION,
            "project_compilation": self.project_compilation(),
            "source_versioning": source_versioning,
            "semantic_revision": self.semantic_revision(),
            "standard_language_set": self.standard_language_set_presentation(),
            "project": self.project_manifest(),
            "storage_topology": self.storage_topology(),
            "workspace_input_roots": self.workspace_input_roots(),
            "source_analyses": self.project_source_analyses(),
            "presentation": self.workspace_presentation(),
            "capabilities": self.capability_presentation(),
            "behavior_packages": self.behavior_package_presentation(),
            "procedure_packages": self.procedure_package_presentation(),
            "semantic_packages": self.semantic_package_presentation(),
            "semantic_modules": self.semantic_module_presentation(),
            "projection": analysis.projection,
            "diagnostics": diagnostics
        })
    }

    /// Return the deterministic artifact produced from the exact source and
    /// selected dependency bytes loaded by this analyzer.
    pub fn project_compilation(&self) -> &ProjectCompilation {
        &self.compilation
    }

    pub fn storage_topology(&self) -> ProjectStorageTopology {
        ProjectStorageTopology {
            project_ref: self.manifest.project_id.clone(),
            authored_sources: self
                .manifest
                .source_documents
                .iter()
                .map(|source| source.path.clone())
                .collect(),
            selected_semantic_packages: self
                .manifest
                .semantic_packages
                .iter()
                .map(|package| package.manifest.clone())
                .collect(),
            workspace_graph: self.manifest.workspace_graph.clone(),
            service_declarations: self.manifest.service_roots.clone(),
            service_state: self.manifest.service_state_roots.clone(),
            operation_receipts: project::RUNTIME_RECEIPTS_PATH.into(),
        }
    }

    pub fn workspace_input_roots(&self) -> &[String] {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.input_roots.as_slice())
            .unwrap_or_default()
    }

    pub fn workspace_presentation(&self) -> Option<&workspace::WorkspacePresentation> {
        self.workspace
            .as_ref()
            .map(|workspace| &workspace.presentation)
    }

    /// Project-context feedback is independent from authored `.sem` text. It
    /// remains available while setup is incomplete so clients can offer an
    /// explicit owner action instead of failing analyzer startup.
    pub fn project_diagnostics(&self) -> Vec<Diagnostic> {
        let range = TextRange {
            start_line: 0,
            start_character: 0,
            end_line: 0,
            end_character: 0,
        };
        let mut diagnostics = Vec::new();
        if self.manifest.locale.is_empty() {
            diagnostics.push(Diagnostic {
                code: "SEM0001".into(),
                severity: Severity::Warning,
                message: "Project language is not selected. Select a language before semantic surface resolution.".into(),
                range: range.clone(),
                owner: "sem-lang/project-context".into(),
                recovery: "configure-project-locale".into(),
            });
        }
        if self.manifest.time_zone.is_none() {
            diagnostics.push(Diagnostic {
                code: "SEM0002".into(),
                severity: Severity::Warning,
                message: "Project timezone is not selected. Select an IANA timezone before resolving relative or local civil time.".into(),
                range: range.clone(),
                owner: "sem-lang/project-context".into(),
                recovery: "configure-project-timezone".into(),
            });
        }
        for package in self
            .behavior_packages
            .iter()
            .filter(|package| package.state != BehaviorInjectionState::Ready)
        {
            diagnostics.push(Diagnostic {
                code: "SEM0003".into(),
                severity: if package.state == BehaviorInjectionState::Unavailable {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                message: format!(
                    "Behavior package {} is {:?}; review its exact operation routes and required behavior providers.",
                    package.package_ref, package.state
                ),
                range: range.clone(),
                owner: "sem-lang/behavior-injection".into(),
                recovery: "review-behavior-package".into(),
            });
        }
        let installed_operations = self
            .operations
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for requirement in self
            .procedure_packages
            .iter()
            .flat_map(|package| &package.procedures)
            .flat_map(|procedure| &procedure.steps)
            .flat_map(|step| &step.operation_requirements)
            .filter(|requirement| {
                !requirement.optional
                    && !installed_operations.contains(requirement.operation_id.as_str())
            })
        {
            diagnostics.push(Diagnostic {
                code: "SEM0004".into(),
                severity: Severity::Warning,
                message: format!(
                    "Procedure requirement {} has no selected operation provider for {}.",
                    requirement.requirement_ref, requirement.operation_id
                ),
                range: range.clone(),
                owner: "sem-lang/procedure-package".into(),
                recovery: "select-operation-provider".into(),
            });
        }
        diagnostics
    }

    /// Analyze every exact source declared by the project. This is used for
    /// background feedback and never adopts, persists, or executes a proposal.
    pub fn project_source_analyses(&self) -> Vec<ProjectSourceAnalysis> {
        self.project_sources
            .iter()
            .map(|source| {
                let physical_digest = Digest::document_digest(source.authored.as_bytes()).to_hex();
                let mut analysis = self.analyze_without_workspace_with_digest(
                    &source.analysis,
                    &physical_digest,
                );
                analysis.diagnostics.extend(source_role_diagnostics(
                    source.document.role,
                    &source.analysis,
                    &analysis.pipelines,
                    &self.vocabularies,
                ));
                let lines = source.authored.lines().collect::<Vec<_>>();
                let source_bindings = self
                    .workspace
                    .as_ref()
                    .into_iter()
                    .flat_map(|workspace| &workspace.presentation.records)
                    .filter_map(|record| record.source.as_ref())
                    .filter(|locator| locator.source_ref == source.document.path.as_str())
                    .map(|locator| {
                        let current_unit_digest = lines
                            .get(locator.start_line as usize..=locator.end_line as usize)
                            .map(|unit| Digest::document_digest(unit.join("\n").as_bytes()).to_hex());
                        ProjectSourceBinding {
                            semantic_unit_ref: locator.semantic_unit_ref.clone(),
                            start_line: locator.start_line,
                            end_line: locator.end_line,
                            bound_unit_digest: locator.unit_digest.clone(),
                            current: current_unit_digest.as_deref()
                                == Some(locator.unit_digest.as_str()),
                            current_unit_digest,
                        }
                    })
                    .collect::<Vec<_>>();
                let stale_ranges = source_bindings
                    .iter()
                    .filter(|binding| !binding.current)
                    .map(|binding| (binding.start_line, binding.end_line))
                    .collect::<BTreeSet<_>>();
                if let Some((start_line, end_line)) = stale_ranges.first() {
                    analysis.diagnostics.push(Diagnostic {
                        code: "SEM4001".into(),
                        severity: Severity::Error,
                        message: format!(
                            "{} durable semantic unit binding(s) refer to changed statement bytes. Review and explicitly rebind the affected records; unrelated edits elsewhere in the document do not stale this unit.",
                            stale_ranges.len()
                        ),
                        range: TextRange {
                            start_line: *start_line,
                            start_character: 0,
                            end_line: *end_line,
                            end_character: 0,
                        },
                        owner: "sem-lang/workspace".into(),
                        recovery: "review-and-rebind-source".into(),
                    });
                }
                ProjectSourceAnalysis {
                    source_ref: source.document.path.clone(),
                    role: source.document.role,
                    source_bindings,
                    analysis,
                }
            })
            .collect()
    }

    /// Return registered operation routes as immutable presentation data.
    /// This cannot execute an operation and never includes authentication
    /// secrets or owner configuration bodies.
    pub fn capability_presentation(&self) -> Vec<CapabilityOperationPresentation> {
        let mut values = self
            .operations
            .iter()
            .flat_map(|(operation_id, routes)| {
                routes.iter().map(move |(owner, revision, action)| {
                    let authentication = self
                        .target_authentication
                        .get(&action.target)
                        .and_then(|service_id| self.authentication.get(service_id))
                        .map(|authentication| AuthenticationProjection {
                            service_id: authentication.service_id.clone(),
                            label: authentication.service.label.clone(),
                            owner: authentication.declaration_owner.clone(),
                            revision: authentication.declaration_revision.clone(),
                            state_owner: authentication.state_owner.clone(),
                            state_revision: authentication.state_revision.clone(),
                            state: authentication.state,
                            configuration_ref: authentication.service.configuration_ref.clone(),
                        });
                    CapabilityOperationPresentation {
                        operation_id: operation_id.clone(),
                        target: action.target.clone(),
                        owner: owner.clone(),
                        revision: revision.clone(),
                        availability: target_availability(&action.availability),
                        availability_reason: availability_reason(&action.availability),
                        authentication,
                    }
                })
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| {
            (
                &left.operation_id,
                &left.target,
                &left.owner,
                &left.revision,
            )
                .cmp(&(
                    &right.operation_id,
                    &right.target,
                    &right.owner,
                    &right.revision,
                ))
        });
        values
    }

    /// Return deterministic behavior-DI resolution without exposing loaders,
    /// credentials, transports or executable handles.
    pub fn behavior_package_presentation(&self) -> &[BehaviorPackagePresentation] {
        &self.behavior_packages
    }

    /// Return exact selected procedure data.  Definitions are inert: callers
    /// cannot use this presentation to execute a capability or mutate a run.
    pub fn procedure_package_presentation(&self) -> &[procedure::ProcedurePackage] {
        &self.procedure_packages
    }

    pub fn semantic_package_presentation(&self) -> &[semantic_package::SemanticPackage] {
        &self.semantic_packages.packages
    }

    /// Return the exact bundled language set admitted into this compilation.
    /// The descriptor is presentation-only and does not expose mutable source.
    pub fn standard_language_set_presentation(
        &self,
    ) -> &sem_lang_standard::StandardLanguageSetDescriptor {
        &self.standard_language_set
    }

    pub fn language_source_selections(&self) -> &[semantic_package::SemanticPackageSource] {
        &self.semantic_packages.language_sources
    }

    pub fn semantic_module_source_selections(&self) -> &[semantic_package::SemanticPackageSource] {
        &self.semantic_packages.semantic_modules
    }

    /// Return compiled reusable semantic modules without resolving external
    /// identity or exposing credentials and payload bodies.
    pub fn semantic_module_presentation(&self) -> &[semantic_module::CompiledSemanticModule] {
        &self.semantic_modules
    }

    pub fn locale(&self) -> &str {
        &self.manifest.locale
    }

    /// Exact identity of the compiled project language. This includes every
    /// typed definition and surface binding and cannot be supplied by config.
    pub fn semantic_revision(&self) -> String {
        self.language.digest.to_hex()
    }

    pub fn completions(&self) -> Vec<Completion> {
        let mut values = self
            .vocabularies
            .iter()
            .flat_map(|vocabulary| {
                vocabulary
                    .lexical_bindings
                    .iter()
                    .map(move |item| (vocabulary, item))
            })
            .filter(|(_, item)| {
                item.language_tag == "*"
                    || item
                        .language_tag
                        .split(',')
                        .any(|tag| tag == self.manifest.locale)
            })
            .map(|(vocabulary, item)| {
                let term = vocabulary
                    .terms
                    .iter()
                    .find(|term| term.id == item.target_term);
                let label = term
                    .map(|term| term.label.as_str())
                    .unwrap_or("semantic term");
                let description = term
                    .map(|term| term.description.as_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or(label);
                let term_id = item.target_term.to_hex();
                let operation_ids = self
                    .bindings
                    .get(&term_id)
                    .into_iter()
                    .flatten()
                    .map(|binding| binding.operation_id.as_str())
                    .collect::<BTreeSet<_>>();
                let mut behavior_bindings = self
                    .behavior_packages
                    .iter()
                    .flat_map(|package| {
                        package
                            .implementations
                            .iter()
                            .filter(|implementation| {
                                operation_ids.contains(implementation.operation_id.as_str())
                            })
                            .map(|implementation| CompletionBehaviorBinding {
                                package_ref: package.package_ref.clone(),
                                package_revision: package.package_revision.clone(),
                                behavior_ref: implementation.behavior_ref.clone(),
                                operation_id: implementation.operation_id.clone(),
                                effect: implementation.effect,
                                state: implementation.state,
                            })
                    })
                    .collect::<Vec<_>>();
                behavior_bindings.sort_by(|left, right| {
                    (&left.package_ref, &left.behavior_ref, &left.operation_id).cmp(&(
                        &right.package_ref,
                        &right.behavior_ref,
                        &right.operation_id,
                    ))
                });
                behavior_bindings.dedup_by(|left, right| {
                    left.package_ref == right.package_ref
                        && left.behavior_ref == right.behavior_ref
                        && left.operation_id == right.operation_id
                });
                let behavior_detail = behavior_bindings
                    .first()
                    .map(|binding| {
                        format!(
                            " · {} → {} · {:?}",
                            binding.package_ref, binding.behavior_ref, binding.state
                        )
                    })
                    .unwrap_or_default();
                Completion {
                    label: item.surface_form.clone(),
                    detail: format!(
                        "{label} · {description}{behavior_detail} · {:?} · {}",
                        item.contribution_kind, item.provenance,
                    ),
                    insert_text: item.surface_form.clone(),
                    behavior_bindings,
                }
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| {
            (&left.label, left.behavior_bindings.is_empty(), &left.detail).cmp(&(
                &right.label,
                right.behavior_bindings.is_empty(),
                &right.detail,
            ))
        });
        values.dedup_by(|left, right| left.label == right.label);
        values
    }

    fn partial_preview(
        &self,
        operation_id: &str,
        observed_terms: &BTreeSet<String>,
    ) -> PartialExecutionPreview {
        let bindings = self
            .input_bindings
            .get(operation_id)
            .cloned()
            .unwrap_or_default();
        let mut targets = self
            .operations
            .get(operation_id)
            .into_iter()
            .flatten()
            .map(|(_owner, _catalog_revision, action)| {
                let mut required = Vec::new();
                required_input_paths(&action.input_schema, "", &mut required);
                required.sort();
                required.dedup();
                let fields = required
                    .into_iter()
                    .map(|path| {
                        let semantic_term = bindings
                            .iter()
                            .find(|binding| {
                                binding.input_path == path
                                    && observed_terms.contains(&binding.term_id)
                            })
                            .map(|binding| binding.term_id.clone());
                        PreviewField {
                            path,
                            state: if semantic_term.is_some() {
                                PreviewFieldState::ObservedNeedsNormalization
                            } else {
                                PreviewFieldState::Missing
                            },
                            semantic_term,
                        }
                    })
                    .collect();
                let authentication =
                    self.target_authentication
                        .get(&action.target)
                        .and_then(|service_id| {
                            self.authentication.get(service_id).map(|authentication| {
                                AuthenticationProjection {
                                    service_id: authentication.service_id.clone(),
                                    label: authentication.service.label.clone(),
                                    owner: authentication.declaration_owner.clone(),
                                    revision: authentication.declaration_revision.clone(),
                                    state_owner: authentication.state_owner.clone(),
                                    state_revision: authentication.state_revision.clone(),
                                    state: authentication.state,
                                    configuration_ref: authentication
                                        .service
                                        .configuration_ref
                                        .clone(),
                                }
                            })
                        });
                TargetCandidate {
                    target: action.target.clone(),
                    contract_revision: action.contract_revision.clone(),
                    availability: target_availability(&action.availability),
                    availability_reason: availability_reason(&action.availability),
                    authentication,
                    fields,
                }
            })
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.target.cmp(&right.target));

        let available = targets
            .iter()
            .filter(|target| target.availability == TargetAvailability::Available)
            .collect::<Vec<_>>();
        let state = if available.is_empty() {
            PreviewState::Unavailable
        } else if available.iter().any(|target| {
            target
                .fields
                .iter()
                .any(|field| field.state == PreviewFieldState::Missing)
        }) {
            PreviewState::NeedsInput
        } else if available.len() > 1 {
            PreviewState::NeedsTarget
        } else if available[0]
            .authentication
            .as_ref()
            .is_some_and(|auth| auth.state != AuthenticationState::Ready)
        {
            PreviewState::NeedsAuthentication
        } else {
            PreviewState::Ready
        };
        PartialExecutionPreview { state, targets }
    }

    pub fn semantic_fix_request(
        &self,
        source: &str,
        source_line: u32,
        diagnostic_code: &str,
    ) -> Option<SemanticFixRequest> {
        let analysis = self.analyze(source);
        self.semantic_fix_request_from_analysis(source, source_line, diagnostic_code, analysis)
    }

    /// Build a semantic repair request for the authored program in
    /// `project.sem`. The project preamble is compiled as configuration and is
    /// never exposed to the repair provider as an instruction.
    pub fn semantic_fix_request_project_source(
        &self,
        source: &str,
        source_line: u32,
        diagnostic_code: &str,
    ) -> Option<SemanticFixRequest> {
        let analysis = self.analyze_project_source(source).ok()?;
        let compiled = project_source::compile(source).ok()?;
        self.semantic_fix_request_from_analysis(
            &compiled.analysis_source,
            source_line,
            diagnostic_code,
            analysis,
        )
    }

    fn semantic_fix_request_from_analysis(
        &self,
        source: &str,
        source_line: u32,
        diagnostic_code: &str,
        analysis: Analysis,
    ) -> Option<SemanticFixRequest> {
        const MAX_ALLOWED_TERMS: usize = 512;

        let diagnostic = analysis
            .diagnostics
            .iter()
            .find(|item| item.code == diagnostic_code && item.range.start_line == source_line)?;
        let instruction = source.lines().nth(source_line as usize)?.trim().to_owned();
        let source_id = format!("source:{}", source_line + 1);
        let nodes = analysis
            .projection
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        let mut unresolved = analysis
            .projection
            .edges
            .iter()
            .filter(|edge| edge.source == source_id && edge.relation == "unresolved")
            .filter_map(|edge| nodes.get(edge.target.as_str()))
            .map(|node| SemanticFixEvidence {
                id: node.id.clone(),
                surface: node.label.clone(),
            })
            .collect::<Vec<_>>();
        unresolved.sort_by(|left, right| left.id.cmp(&right.id));
        unresolved.dedup();

        let surfaces = analysis
            .projection
            .edges
            .iter()
            .filter(|edge| edge.source == source_id && edge.relation == "contains")
            .map(|edge| edge.target.as_str())
            .collect::<BTreeSet<_>>();
        let semantic_ids = analysis
            .projection
            .edges
            .iter()
            .filter(|edge| surfaces.contains(edge.source.as_str()) && edge.relation == "denotes")
            .map(|edge| edge.target.as_str())
            .collect::<BTreeSet<_>>();
        let mut resolved_terms = semantic_ids
            .into_iter()
            .filter_map(|id| nodes.get(id))
            .map(|node| SemanticFixTerm {
                id: node.id.trim_start_matches("semantic:").to_owned(),
                label: node.label.clone(),
            })
            .collect::<Vec<_>>();
        resolved_terms.sort();
        resolved_terms.dedup();

        let mut allowed_terms = self
            .vocabularies
            .iter()
            .flat_map(|vocabulary| &vocabulary.terms)
            .map(|term| SemanticFixTerm {
                id: term.id.to_hex(),
                label: term.label.clone(),
            })
            .collect::<Vec<_>>();
        allowed_terms.sort();
        allowed_terms.dedup();
        let allowed_terms_complete = allowed_terms.len() <= MAX_ALLOWED_TERMS;
        allowed_terms.truncate(MAX_ALLOWED_TERMS);

        Some(SemanticFixRequest {
            schema_version: SCHEMA_VERSION.into(),
            diagnostic_code: diagnostic.code.clone(),
            owner: diagnostic.owner.clone(),
            recovery: diagnostic.recovery.clone(),
            locale: self.manifest.locale.clone(),
            semantic_revision: analysis.semantic_revision,
            source_digest: analysis.source_digest,
            source_line,
            instruction,
            unresolved,
            resolved_terms,
            allowed_terms,
            allowed_terms_complete,
        })
    }

    pub fn analyze(&self, source: &str) -> Analysis {
        self.analyze_inner(source, true, None)
    }

    fn value_bindings_for_terms(
        &self,
        term_ids: &BTreeSet<String>,
    ) -> Vec<PipelineValueBindingProjection> {
        let mut values = self
            .semantic_modules
            .iter()
            .flat_map(|module| {
                module
                    .value_bindings
                    .iter()
                    .map(move |binding| (module, binding))
            })
            .filter(|(_, binding)| term_ids.contains(&binding.term_id))
            .map(|(module, binding)| {
                let resolved_value_refs =
                    self.resolve_value_producer(&binding.producer_ref, binding.selector.as_deref());
                let cardinality_valid = match binding.cardinality.as_str() {
                    "one" => resolved_value_refs.len() == 1,
                    "optional" => resolved_value_refs.len() <= 1,
                    "many" => true,
                    _ => false,
                };
                let state = if !cardinality_valid {
                    "cardinality-mismatch"
                } else if resolved_value_refs.is_empty() {
                    "empty"
                } else {
                    "resolved"
                };
                PipelineValueBindingProjection {
                    value_ref: binding.value_ref.clone(),
                    term_id: binding.term_id.clone(),
                    value_type_ref: binding.value_type_ref.clone(),
                    cardinality: binding.cardinality.clone(),
                    authority_ref: binding.authority_ref.clone(),
                    producer_ref: binding.producer_ref.clone(),
                    materialization: binding.materialization.clone(),
                    resolved_value_refs,
                    state: state.into(),
                    definition_source_ref: module.source_ref.clone(),
                }
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.value_ref.cmp(&right.value_ref));
        values
    }

    fn resolve_value_producer(&self, producer_ref: &str, selector: Option<&str>) -> Vec<String> {
        let mut values = match producer_ref {
            "workspace/activity/current" => self
                .workspace
                .as_ref()
                .into_iter()
                .flat_map(|workspace| &workspace.presentation.activities)
                .map(|value| value.activity_ref.clone())
                .collect::<Vec<_>>(),
            "workspace/plan/all" => self
                .workspace
                .as_ref()
                .into_iter()
                .flat_map(|workspace| &workspace.presentation.plans)
                .map(|value| value.plan_ref.clone())
                .collect::<Vec<_>>(),
            "workspace/history/all" => self
                .workspace
                .as_ref()
                .into_iter()
                .flat_map(|workspace| &workspace.presentation.history)
                .map(|value| value.history_ref.clone())
                .collect::<Vec<_>>(),
            "workspace/record-set/all" => self
                .workspace
                .as_ref()
                .into_iter()
                .flat_map(|workspace| &workspace.presentation.record_sets)
                .map(|value| value.record_set_ref.clone())
                .collect::<Vec<_>>(),
            "workspace/output/all" => self
                .workspace
                .as_ref()
                .into_iter()
                .flat_map(|workspace| &workspace.presentation.outputs)
                .map(|value| value.output_ref.clone())
                .collect::<Vec<_>>(),
            "project/source" => selector
                .into_iter()
                .flat_map(|selector| {
                    self.project_sources
                        .iter()
                        .filter(move |source| source.document.path == selector)
                })
                .flat_map(|source| {
                    self.workspace
                        .as_ref()
                        .into_iter()
                        .flat_map(|workspace| &workspace.presentation.records)
                        .filter_map(|record| record.source.as_ref())
                        .filter(move |locator| locator.source_ref == source.document.path)
                        .map(|locator| locator.semantic_unit_ref.clone())
                })
                .collect::<Vec<_>>(),
            "project/resource" => selector
                .into_iter()
                .flat_map(|selector| {
                    self.semantic_modules
                        .iter()
                        .flat_map(|module| &module.resources)
                        .filter(move |resource| resource.resource_ref == selector)
                        .map(|resource| resource.resource_ref.clone())
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        values.sort();
        values.dedup();
        values
    }

    /// Analyze a live `project.sem`; configuration is not instruction text.
    pub fn analyze_project_source(&self, source: &str) -> Result<Analysis, ToolingError> {
        let compiled = project_source::compile(source)?;
        let digest = Digest::document_digest(source.as_bytes()).to_hex();
        Ok(self.analyze_inner(&compiled.analysis_source, true, Some(&digest)))
    }

    /// Analyze one not-yet-adopted contribution without mixing project graph
    /// records into its candidate projection.
    pub fn analyze_contribution(&self, source: &str) -> Analysis {
        self.analyze_without_workspace(source)
    }

    /// Return package-declared structural contribution kinds.
    pub fn contribution_structure_kinds(&self) -> Vec<ContributionStructureKind> {
        ["Request", "Question", "Assertion"]
            .into_iter()
            .filter_map(|name| {
                let candidates = self
                    .vocabularies
                    .iter()
                    .flat_map(|vocabulary| &vocabulary.terms)
                    .filter(|term| term.label == name)
                    .filter_map(|term| {
                        let surface = self
                            .vocabularies
                            .iter()
                            .flat_map(|vocabulary| &vocabulary.lexical_bindings)
                            .filter(|binding| {
                                binding.target_term == term.id
                                    && binding.contribution_kind == ContributionKind::Structural
                                    && (binding.language_tag == self.manifest.locale
                                        || binding.language_tag == "*")
                            })
                            .max_by_key(|binding| binding.priority)?;
                        Some((term, surface))
                    })
                    .collect::<Vec<_>>();
                let [(term, surface)] = candidates.as_slice() else {
                    return None;
                };
                Some(ContributionStructureKind {
                    term_id: term.id.to_hex(),
                    label: term.label.clone(),
                    surface: surface.surface_form.clone(),
                })
            })
            .collect()
    }

    /// Build a localized Markdown-compatible draft without guessing a result.
    pub fn structure_contribution(&self, term_id: &str, surface: &str) -> Option<String> {
        let kind = self
            .contribution_structure_kinds()
            .into_iter()
            .find(|kind| kind.term_id == term_id)?;
        let body = surface
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| format!("  - {line}"))
            .collect::<Vec<_>>();
        if body.is_empty() {
            return None;
        }
        let source_surface = self
            .vocabularies
            .iter()
            .flat_map(|vocabulary| &vocabulary.lexical_bindings)
            .filter(|binding| {
                (binding.language_tag == "*"
                    || binding
                        .language_tag
                        .split(',')
                        .any(|tag| tag == self.manifest.locale))
                    && self
                        .vocabularies
                        .iter()
                        .flat_map(|vocabulary| &vocabulary.terms)
                        .find(|term| term.id == binding.target_term)
                        .and_then(|term| PipelineOperator::from_canonical_name(&term.label))
                        == Some(PipelineOperator::Source)
            })
            .max_by_key(|binding| binding.priority)?
            .surface_form
            .clone();
        let values = body.join("\n");
        Some(format!(
            "# {}\n\n{}:\n{}\n",
            kind.surface, source_surface, values
        ))
    }

    fn analyze_without_workspace(&self, source: &str) -> Analysis {
        self.analyze_inner(source, false, None)
    }

    fn analyze_without_workspace_with_digest(&self, source: &str, source_digest: &str) -> Analysis {
        self.analyze_inner(source, false, Some(source_digest))
    }

    fn analyze_inner(
        &self,
        source: &str,
        include_workspace: bool,
        source_digest: Option<&str>,
    ) -> Analysis {
        let source_digest = source_digest
            .map(str::to_owned)
            .unwrap_or_else(|| Digest::document_digest(source.as_bytes()).to_hex());
        let mut diagnostics = Vec::new();
        let mut execution_choices = Vec::new();
        let mut pipelines = Vec::<PipelineProjection>::new();
        let mut pipeline_terms = Vec::<BTreeSet<String>>::new();
        let mut nodes = BTreeMap::<String, ProjectionNode>::new();
        let mut edges = Vec::<ProjectionEdge>::new();
        if include_workspace && let Some(workspace) = &self.workspace {
            diagnostics.extend(workspace.diagnostics.iter().cloned());
            for node in &workspace.nodes {
                nodes.insert(node.id.clone(), node.clone());
            }
            edges.extend(workspace.edges.iter().cloned());
        }
        let terms = self
            .vocabularies
            .iter()
            .flat_map(|vocabulary| {
                vocabulary
                    .terms
                    .iter()
                    .map(move |term| (term, vocabulary.vocabulary_id.as_str()))
            })
            .map(|(term, owner)| (term.id.to_hex(), (term, owner)))
            .collect::<BTreeMap<_, _>>();
        let bindings = self
            .vocabularies
            .iter()
            .flat_map(|vocabulary| {
                vocabulary
                    .lexical_bindings
                    .iter()
                    .map(move |binding| (binding, vocabulary.vocabulary_id.as_str()))
            })
            .filter(|(binding, _)| {
                binding.language_tag == "*"
                    || binding
                        .language_tag
                        .split(',')
                        .any(|tag| tag == self.manifest.locale)
            })
            .collect::<Vec<_>>();

        let structural_bindings = bindings
            .iter()
            .copied()
            .filter(|(binding, _)| binding.contribution_kind == ContributionKind::Structural)
            .collect::<Vec<_>>();
        let content_bindings = bindings
            .iter()
            .copied()
            .filter(|(binding, _)| binding.contribution_kind != ContributionKind::Structural)
            .collect::<Vec<_>>();
        let mut current_section = None::<String>;
        let mut current_section_line = None::<u32>;
        let mut current_section_term_id = None::<String>;
        let mut current_pipeline = None::<usize>;
        let mut current_pipeline_step = None::<usize>;
        let mut current_property = None::<String>;
        let mut current_property_term_id = None::<String>;

        let semantic_lines = match document_analysis::semantic_lines(source) {
            Ok(lines) => lines,
            Err(error) => {
                diagnostics.push(Diagnostic {
                    code: "SEM0100".into(),
                    severity: Severity::Error,
                    message: error.to_string(),
                    range: line_range(0, source.lines().next().unwrap_or_default()),
                    owner: "sem-lang/document-syntax".into(),
                    recovery: "repair-document-structure".into(),
                });
                Vec::new()
            }
        };
        for semantic_line in &semantic_lines {
            let index = semantic_line.physical_line;
            let raw = semantic_line.raw.as_str();
            let line = semantic_line.semantic.as_str();
            let line_number = index as u32;
            if let Some(heading) = line
                .strip_prefix('#')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let section_id = format!("section:{}", index + 1);
                nodes.insert(
                    section_id.clone(),
                    ProjectionNode {
                        id: section_id.clone(),
                        kind: "section".into(),
                        label: heading.into(),
                        state: "structured".into(),
                        owner: "workspace".into(),
                        revision: source_digest.clone(),
                    },
                );
                current_section_term_id = None;
                if let Some((binding, binding_owner)) = structural_bindings
                    .iter()
                    .find(|(binding, _)| binding.surface_form == heading)
                {
                    let term_id = binding.target_term.to_hex();
                    current_section_term_id = Some(term_id.clone());
                    let semantic_id = format!("semantic:{term_id}");
                    let term = terms.get(&term_id);
                    nodes
                        .entry(semantic_id.clone())
                        .or_insert_with(|| ProjectionNode {
                            id: semantic_id.clone(),
                            kind: "semantic".into(),
                            label: term.map(|(item, _)| item.label.clone()).unwrap_or(term_id),
                            state: "resolved".into(),
                            owner: (*binding_owner).into(),
                            revision: self.semantic_revision(),
                        });
                    edges.push(ProjectionEdge {
                        source: section_id.clone(),
                        relation: "denotes".into(),
                        target: semantic_id,
                    });
                } else {
                    if !self.manifest.locale.is_empty() {
                        let unresolved_id = format!("unresolved-heading:{}", index + 1);
                        nodes.insert(
                            unresolved_id.clone(),
                            ProjectionNode {
                                id: unresolved_id.clone(),
                                kind: "unresolved".into(),
                                label: heading.into(),
                                state: "requires-definition".into(),
                                owner: "sem-lang/language".into(),
                                revision: source_digest.clone(),
                            },
                        );
                        edges.push(ProjectionEdge {
                            source: section_id.clone(),
                            relation: "unresolved".into(),
                            target: unresolved_id,
                        });
                        diagnostics.push(Diagnostic {
                            code: "SEM1001".into(),
                            severity: Severity::Warning,
                            message: "No selected semantic definition resolves this heading. Select or extend a definition package; local inference may propose a typed candidate but cannot activate it.".into(),
                            range: line_range(line_number, raw),
                            owner: "sem-lang/language".into(),
                            recovery: "select-or-extend-definition".into(),
                        });
                    }
                }
                current_section = Some(section_id);
                current_section_line = Some(line_number);
                current_pipeline = None;
                current_pipeline_step = None;
                current_property = None;
                current_property_term_id = None;
                continue;
            }

            if let Some(operator_surface) = line.strip_suffix(':').map(str::trim)
                && let Some((binding, binding_owner)) = bindings.iter().find(|(binding, _)| {
                    binding.surface_form == operator_surface
                        && terms
                            .get(&binding.target_term.to_hex())
                            .and_then(|(term, _)| {
                                PipelineOperator::from_canonical_name(&term.label)
                            })
                            .is_some()
                })
            {
                let Some(section_id) = current_section.as_ref() else {
                    diagnostics.push(Diagnostic {
                        code: "SEM0101".into(),
                        severity: Severity::Error,
                        message: "A pipeline operator must be inside a structured .sem section."
                            .into(),
                        range: line_range(line_number, raw),
                        owner: "sem-lang/source-structure".into(),
                        recovery: "select-contribution-structure".into(),
                    });
                    continue;
                };
                let operator = terms
                    .get(&binding.target_term.to_hex())
                    .and_then(|(term, _)| PipelineOperator::from_canonical_name(&term.label))
                    .expect("operator binding was checked above");
                let pipeline_index = *current_pipeline.get_or_insert_with(|| {
                    pipelines.push(PipelineProjection {
                        section_line: current_section_line.unwrap_or(line_number),
                        section_term_id: current_section_term_id.clone(),
                        state: "pending-validation".into(),
                        steps: Vec::new(),
                    });
                    pipeline_terms.push(BTreeSet::new());
                    pipelines.len() - 1
                });
                let step_index = pipelines[pipeline_index].steps.len();
                pipelines[pipeline_index]
                    .steps
                    .push(PipelineStepProjection {
                        operator: operator.name().into(),
                        operator_term_id: binding.target_term.to_hex(),
                        source_line: line_number,
                        values: Vec::new(),
                    });
                current_pipeline_step = Some(step_index);
                let step_id = format!("pipeline:{}:{}", pipeline_index + 1, step_index + 1);
                nodes.insert(
                    step_id.clone(),
                    ProjectionNode {
                        id: step_id.clone(),
                        kind: "pipeline-step".into(),
                        label: operator_surface.into(),
                        state: "structured".into(),
                        owner: (*binding_owner).into(),
                        revision: self.semantic_revision(),
                    },
                );
                edges.push(ProjectionEdge {
                    source: section_id.clone(),
                    relation: "contains-step".into(),
                    target: step_id,
                });
                current_property = None;
                current_property_term_id = None;
                continue;
            }
            if let Some(property_surface) = line.strip_suffix(':').map(str::trim)
                && let Some((binding, binding_owner)) = structural_bindings
                    .iter()
                    .find(|(binding, _)| binding.surface_form == property_surface)
            {
                let Some(section_id) = current_section.as_ref() else {
                    diagnostics.push(Diagnostic {
                        code: "SEM0101".into(),
                        severity: Severity::Error,
                        message: "A semantic property must be inside a structured .sem section."
                            .into(),
                        range: line_range(line_number, raw),
                        owner: "sem-lang/source-structure".into(),
                        recovery: "select-contribution-structure".into(),
                    });
                    continue;
                };
                let term_id = binding.target_term.to_hex();
                let property_id = format!("property:{}:{term_id}", index + 1);
                nodes.insert(
                    property_id.clone(),
                    ProjectionNode {
                        id: property_id.clone(),
                        kind: "property".into(),
                        label: property_surface.into(),
                        state: "resolved".into(),
                        owner: (*binding_owner).into(),
                        revision: self.semantic_revision(),
                    },
                );
                edges.push(ProjectionEdge {
                    source: section_id.clone(),
                    relation: "contains-property".into(),
                    target: property_id.clone(),
                });
                let semantic_id = format!("semantic:{term_id}");
                let term = terms.get(&term_id);
                nodes
                    .entry(semantic_id.clone())
                    .or_insert_with(|| ProjectionNode {
                        id: semantic_id.clone(),
                        kind: "semantic".into(),
                        label: term
                            .map(|(item, _)| item.label.clone())
                            .unwrap_or_else(|| term_id.clone()),
                        state: "resolved".into(),
                        owner: (*binding_owner).into(),
                        revision: self.semantic_revision(),
                    });
                edges.push(ProjectionEdge {
                    source: property_id.clone(),
                    relation: "denotes".into(),
                    target: semantic_id,
                });
                current_property = Some(property_id);
                current_property_term_id = Some(term_id);
                current_pipeline_step = None;
                continue;
            }
            let source_id = format!("source:{}", index + 1);
            nodes.insert(
                source_id.clone(),
                ProjectionNode {
                    id: source_id.clone(),
                    kind: "instruction".into(),
                    label: line.into(),
                    state: "observed".into(),
                    owner: "workspace".into(),
                    revision: source_digest.clone(),
                },
            );
            if let Some(property_id) = &current_property {
                edges.push(ProjectionEdge {
                    source: property_id.clone(),
                    relation: "has-value".into(),
                    target: source_id.clone(),
                });
            } else if let Some(section_id) = &current_section {
                edges.push(ProjectionEdge {
                    source: section_id.clone(),
                    relation: "contains-statement".into(),
                    target: source_id.clone(),
                });
            } else {
                diagnostics.push(Diagnostic {
                    code: "SEM0101".into(),
                    severity: Severity::Error,
                    message: "This statement is outside a structured .sem section. Add a selected communicative-act heading before it.".into(),
                    range: line_range(line_number, raw),
                    owner: "sem-lang/source-structure".into(),
                    recovery: "select-contribution-structure".into(),
                });
            }
            if let (Some(pipeline_index), Some(step_index)) =
                (current_pipeline, current_pipeline_step)
            {
                edges.push(ProjectionEdge {
                    source: format!("pipeline:{}:{}", pipeline_index + 1, step_index + 1),
                    relation: "accepts-value".into(),
                    target: source_id.clone(),
                });
            }
            // Missing project language is one setup issue, not one issue per line.
            if self.manifest.locale.is_empty() {
                continue;
            }
            // Longer exact lexical forms win overlapping ranges.
            let mut candidates = content_bindings
                .iter()
                .enumerate()
                .flat_map(|(binding_index, (binding, _))| {
                    line.match_indices(&binding.surface_form)
                        .map(move |(start, surface)| (start, start + surface.len(), binding_index))
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                (left.0, std::cmp::Reverse(left.1 - left.0), left.2).cmp(&(
                    right.0,
                    std::cmp::Reverse(right.1 - right.0),
                    right.2,
                ))
            });
            let mut covered = vec![false; line.len()];
            let mut matched = Vec::new();
            for (start, end, binding_index) in candidates {
                if covered[start..end].iter().any(|value| *value) {
                    continue;
                }
                covered[start..end].fill(true);
                matched.push((start, end, binding_index));
            }
            matched.sort_by_key(|(start, end, binding_index)| (*start, *end, *binding_index));

            let unresolved = uncovered_fragments(line, &covered);
            for (fragment_index, (start, end, label)) in unresolved.iter().enumerate() {
                let unresolved_id = format!(
                    "unresolved:{}:{}:{}:{}",
                    index + 1,
                    start,
                    end,
                    fragment_index
                );
                nodes.insert(
                    unresolved_id.clone(),
                    ProjectionNode {
                        id: unresolved_id.clone(),
                        kind: "unresolved".into(),
                        label: label.clone(),
                        state: "requires-definition".into(),
                        owner: "sem-lang/language".into(),
                        revision: source_digest.clone(),
                    },
                );
                edges.push(ProjectionEdge {
                    source: source_id.clone(),
                    relation: "unresolved".into(),
                    target: unresolved_id,
                });
            }

            // Purpose and completion criteria are human-authored evidence.
            // Preserve every unresolved fragment in the projection, but do
            // not present ordinary prose as a compiler warning. Properties
            // that identify an actual target remain strict and therefore
            // continue to require selected semantic definitions.
            let narrative_property = current_property_term_id
                .as_deref()
                .and_then(|term_id| terms.get(term_id))
                .is_some_and(|(term, _)| document_analysis::is_narrative_property(&term.label));
            if matched.is_empty() && !narrative_property {
                diagnostics.push(Diagnostic {
                    code: "SEM1001".into(),
                    severity: Severity::Warning,
                    message: "No selected semantic definition resolves this instruction. Select or extend a definition package; the editor will not guess.".into(),
                    range: line_range(line_number, raw),
                    owner: "sem-lang/language".into(),
                    recovery: "select-or-extend-definition".into(),
                });
                continue;
            }
            if !unresolved.is_empty() && !narrative_property {
                diagnostics.push(Diagnostic {
                    code: "SEM1003".into(),
                    severity: Severity::Warning,
                    message: "Only part of this instruction is covered by the selected semantic definition package. Unresolved surface evidence is preserved without guessing.".into(),
                    range: line_range(line_number, raw),
                    owner: "sem-lang/language".into(),
                    recovery: "select-or-extend-definition".into(),
                });
            }
            let mut seen_terms = BTreeSet::new();
            for (start, end, binding_index) in matched {
                let (binding, binding_owner) = content_bindings[binding_index];
                let term_id = binding.target_term.to_hex();
                let surface_id = format!("surface:{}:{start}:{end}:{term_id}", index + 1);
                nodes.insert(
                    surface_id.clone(),
                    ProjectionNode {
                        id: surface_id.clone(),
                        kind: "surface".into(),
                        label: line[start..end].into(),
                        state: "matched".into(),
                        owner: binding_owner.into(),
                        revision: self.semantic_revision(),
                    },
                );
                edges.push(ProjectionEdge {
                    source: source_id.clone(),
                    relation: "contains".into(),
                    target: surface_id.clone(),
                });
                let term = terms.get(&term_id);
                let semantic_id = format!("semantic:{term_id}");
                nodes
                    .entry(semantic_id.clone())
                    .or_insert_with(|| ProjectionNode {
                        id: semantic_id.clone(),
                        kind: "semantic".into(),
                        label: term
                            .map(|(item, _)| item.label.clone())
                            .unwrap_or(term_id.clone()),
                        state: "resolved".into(),
                        owner: term
                            .map(|(_, owner)| (*owner).to_owned())
                            .unwrap_or_else(|| binding_owner.to_owned()),
                        revision: self.semantic_revision(),
                    });
                edges.push(ProjectionEdge {
                    source: surface_id,
                    relation: "denotes".into(),
                    target: semantic_id.clone(),
                });
                seen_terms.insert(term_id);
            }
            if let (Some(pipeline_index), Some(step_index)) =
                (current_pipeline, current_pipeline_step)
            {
                let authored_value = line.strip_prefix("- ").unwrap_or(line).trim();
                let exact_term_ids = content_bindings
                    .iter()
                    .filter(|(binding, _)| binding.surface_form == authored_value)
                    .map(|(binding, _)| binding.target_term.to_hex())
                    .collect::<BTreeSet<_>>();
                let binding_required = exact_term_ids.iter().any(|term_id| {
                    terms.get(term_id).is_some_and(|(term, _)| {
                        matches!(
                            term.semantic_definition,
                            Some(sem_lang_linguistics::SemanticDefinition::Primitive(
                                sem_lang_linguistics::PrimitiveSemantic::Value
                                    | sem_lang_linguistics::PrimitiveSemantic::Reference
                            ))
                        )
                    })
                });
                let value_bindings = self.value_bindings_for_terms(&exact_term_ids);
                for binding in &value_bindings {
                    let binding_node = format!("value-binding:{}", binding.value_ref);
                    nodes
                        .entry(binding_node.clone())
                        .or_insert_with(|| ProjectionNode {
                            id: binding_node.clone(),
                            kind: "value-binding".into(),
                            label: binding.value_ref.clone(),
                            state: binding.state.clone(),
                            owner: binding.authority_ref.clone(),
                            revision: self.semantic_revision(),
                        });
                    edges.push(ProjectionEdge {
                        source: source_id.clone(),
                        relation: "binds-value".into(),
                        target: binding_node.clone(),
                    });
                    for value_ref in &binding.resolved_value_refs {
                        nodes
                            .entry(value_ref.clone())
                            .or_insert_with(|| ProjectionNode {
                                id: value_ref.clone(),
                                kind: "value-reference".into(),
                                label: value_ref.clone(),
                                state: "resolved".into(),
                                owner: binding.authority_ref.clone(),
                                revision: self.semantic_revision(),
                            });
                        edges.push(ProjectionEdge {
                            source: binding_node.clone(),
                            relation: "resolves-to".into(),
                            target: value_ref.clone(),
                        });
                    }
                }
                pipelines[pipeline_index].steps[step_index]
                    .values
                    .push(PipelineValueProjection {
                        source_line: line_number,
                        label: line.into(),
                        term_ids: seen_terms.iter().cloned().collect(),
                        binding_required,
                        bindings: value_bindings,
                    });
                pipeline_terms[pipeline_index].extend(seen_terms.iter().cloned());
            }

            // Source and Result describe data boundaries; Select only narrows
            // data. None of them invokes injected behavior. Transform,
            // Expand, Aggregate, and Effect are the only canonical pipeline
            // positions that may project an owner-declared operation. A
            // source without a pipeline can still expose a non-executable
            // authoring preview while it is being structured.
            let operation_position = current_pipeline.zip(current_pipeline_step).and_then(
                |(pipeline_index, step_index)| {
                    pipelines
                        .get(pipeline_index)
                        .and_then(|pipeline| pipeline.steps.get(step_index))
                        .map(|step| step.operator.as_str())
                },
            );
            let may_project_operation = current_property.is_none()
                && (operation_position.is_none()
                    || matches!(
                        operation_position,
                        Some("transform" | "expand" | "aggregate" | "effect")
                    ));
            let preview_terms = current_pipeline
                .and_then(|pipeline_index| pipeline_terms.get(pipeline_index))
                .cloned()
                .unwrap_or_else(|| seen_terms.clone());
            let mut triggered_operations = BTreeMap::<String, String>::new();
            if may_project_operation {
                for term_id in &seen_terms {
                    for binding in self.bindings.get(term_id).into_iter().flatten() {
                        triggered_operations
                            .entry(binding.operation_id.clone())
                            .or_insert_with(|| term_id.clone());
                    }
                }
            }
            let mut routes = Vec::new();
            for (operation_id, trigger_term) in triggered_operations {
                let preview = self.partial_preview(&operation_id, &preview_terms);
                let operation_node = format!("operation:{operation_id}");
                nodes.insert(
                    operation_node.clone(),
                    ProjectionNode {
                        id: operation_node.clone(),
                        kind: "operation".into(),
                        label: operation_id.clone(),
                        state: preview_state_name(preview.state).into(),
                        owner: "zixcel/interaction".into(),
                        revision: SCHEMA_VERSION.into(),
                    },
                );
                edges.push(ProjectionEdge {
                    source: format!("semantic:{trigger_term}"),
                    relation: "enables".into(),
                    target: operation_node.clone(),
                });

                for target in &preview.targets {
                    let target_node = format!("target:{}:{}", operation_id, target.target);
                    nodes.insert(
                        target_node.clone(),
                        ProjectionNode {
                            id: target_node.clone(),
                            kind: "target".into(),
                            label: target.target.clone(),
                            state: target_availability_name(target.availability).into(),
                            owner: "zixcel/interaction".into(),
                            revision: target.contract_revision.clone(),
                        },
                    );
                    edges.push(ProjectionEdge {
                        source: operation_node.clone(),
                        relation: "can-target".into(),
                        target: target_node.clone(),
                    });
                    for field in &target.fields {
                        let field_node = format!(
                            "input:{}:{}:{}",
                            operation_id,
                            target.target,
                            field.path.replace('/', ":")
                        );
                        nodes.insert(
                            field_node.clone(),
                            ProjectionNode {
                                id: field_node.clone(),
                                kind: "input".into(),
                                label: field.path.clone(),
                                state: preview_field_state_name(field.state).into(),
                                owner: "zixcel/interaction".into(),
                                revision: target.contract_revision.clone(),
                            },
                        );
                        edges.push(ProjectionEdge {
                            source: target_node.clone(),
                            relation: "requires-input".into(),
                            target: field_node,
                        });
                    }
                    if let Some(authentication) = &target.authentication {
                        let auth_node = format!("authentication:{}", authentication.service_id);
                        nodes.insert(
                            auth_node.clone(),
                            ProjectionNode {
                                id: auth_node.clone(),
                                kind: "authentication".into(),
                                label: authentication.label.clone(),
                                state: authentication_state_name(authentication.state).into(),
                                owner: authentication.state_owner.clone(),
                                revision: authentication.state_revision.clone(),
                            },
                        );
                        edges.push(ProjectionEdge {
                            source: target_node,
                            relation: "authenticates-with".into(),
                            target: auth_node,
                        });
                    }
                }

                match preview.state {
                    PreviewState::NeedsInput => {
                        let fields = preview.targets.iter().flat_map(|target| {
                            target.fields.iter().map(move |field| format!(
                                "{}:{} ({})",
                                target.target,
                                field.path,
                                preview_field_state_name(field.state)
                            ))
                        }).collect::<Vec<_>>().join(", ");
                        diagnostics.push(Diagnostic {
                            code: "SEM3001".into(),
                            // Analysis describes an operation but does not
                            // execute it. Missing invocation inputs are an
                            // authoring prompt until the user starts that
                            // operation, not a malformed source document.
                            severity: Severity::Information,
                            message: format!(
                                "The partial preview for {operation_id} requires these exact inputs: {fields}."
                            ),
                            range: line_range(line_number, raw),
                            owner: "zixcel/interaction".into(),
                            recovery: "complete-required-input".into(),
                        });
                    }
                    PreviewState::NeedsTarget => diagnostics.push(Diagnostic {
                        code: "SEM3002".into(),
                        severity: Severity::Information,
                        message: format!(
                            "Select one exact target for {operation_id}; the preview lists the available destinations."
                        ),
                        range: line_range(line_number, raw),
                        owner: "workspace/target-selection".into(),
                        recovery: "select-operation-target".into(),
                    }),
                    PreviewState::NeedsAuthentication => diagnostics.push(Diagnostic {
                        code: "SEM3003".into(),
                        severity: Severity::Information,
                        message: format!(
                            "The selected target for {operation_id} requires its declared authentication service."
                        ),
                        range: line_range(line_number, raw),
                        owner: "workspace/authentication-selection".into(),
                        recovery: "configure-authentication-service".into(),
                    }),
                    // Availability is evaluated across all routes for the
                    // instruction below. One unavailable route must not turn
                    // a viable alternative into a compilation failure.
                    PreviewState::Unavailable => {}
                    PreviewState::Ready => {}
                }
                routes.push(ExecutionRoute {
                    operation_id,
                    preview,
                });
            }
            if !routes.is_empty() {
                routes.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
                let viable = routes
                    .iter()
                    .filter(|route| route.preview.state != PreviewState::Unavailable)
                    .collect::<Vec<_>>();
                let state = if viable.is_empty() {
                    diagnostics.push(Diagnostic {
                        code: "SEM2001".into(),
                        severity: Severity::Error,
                        message: "No available execution route is declared for this intent in the selected environment.".into(),
                        range: line_range(line_number, raw),
                        owner: "zixcel/interaction".into(),
                        recovery: "configure-or-select-capability".into(),
                    });
                    ExecutionChoiceState::Unavailable
                } else if viable.len() > 1 {
                    for unavailable in routes
                        .iter()
                        .filter(|route| route.preview.state == PreviewState::Unavailable)
                    {
                        diagnostics.push(Diagnostic {
                            code: "SEM2002".into(),
                            severity: Severity::Information,
                            message: format!(
                                "The route {} is unavailable, but another route can fulfill this intent.",
                                unavailable.operation_id
                            ),
                            range: line_range(line_number, raw),
                            owner: "zixcel/interaction".into(),
                            recovery: "review-execution-route".into(),
                        });
                    }
                    diagnostics.push(Diagnostic {
                        code: "SEM3000".into(),
                        severity: Severity::Information,
                        message: "Select how this intent should be fulfilled; the preview preserves every owner-declared route and its requirements.".into(),
                        range: line_range(line_number, raw),
                        owner: "workspace/route-selection".into(),
                        recovery: "select-execution-route".into(),
                    });
                    ExecutionChoiceState::NeedsRoute
                } else {
                    execution_choice_state(viable[0].preview.state)
                };
                let decision_ref = Digest::from_serializable(
                    1,
                    &(
                        &source_digest,
                        line_number,
                        routes
                            .iter()
                            .map(|route| route.operation_id.as_str())
                            .collect::<Vec<_>>(),
                    ),
                )
                .expect("execution choice identity is serializable")
                .to_hex();
                execution_choices.push(ExecutionChoice {
                    decision_ref,
                    source_line: line_number,
                    state,
                    routes,
                });
            }
            // Compilation remains useful evidence, but failure never causes a
            // guessed replacement. Composition gaps are explicit diagnostics.
            if !narrative_property
                && let Err(error) = self.language.compile_semantics(line, &self.manifest.locale)
            {
                diagnostics.push(Diagnostic {
                    code: "SEM1002".into(),
                    severity: Severity::Warning,
                    message: format!("The instruction is only partially structured: {error}"),
                    range: line_range(line_number, raw),
                    owner: "sem-lang/compiler".into(),
                    recovery: "review-structured-meaning".into(),
                });
            }
        }
        for pipeline in &mut pipelines {
            let missing_value = pipeline.steps.iter().find(|step| step.values.is_empty());
            let missing_binding = pipeline
                .steps
                .iter()
                .filter(|step| step.operator == "source")
                .flat_map(|step| &step.values)
                .find(|value| value.binding_required && value.bindings.is_empty())
                .cloned();
            let invalid_binding = pipeline
                .steps
                .iter()
                .filter(|step| step.operator == "source")
                .flat_map(|step| &step.values)
                .flat_map(|value| &value.bindings)
                .find(|binding| binding.state == "cardinality-mismatch")
                .cloned();
            let operators = pipeline
                .steps
                .iter()
                .filter_map(|step| PipelineOperator::from_name(&step.operator));
            let shape = validate_pipeline_shape(operators);
            pipeline.state = if missing_value.is_none()
                && missing_binding.is_none()
                && invalid_binding.is_none()
                && shape.is_ok()
            {
                "structured".into()
            } else {
                "invalid".into()
            };
            if let Some(step) = missing_value {
                diagnostics.push(Diagnostic {
                    code: "SEM0110".into(),
                    severity: Severity::Error,
                    message: format!(
                        "The {} pipeline step requires at least one semantic value.",
                        step.operator
                    ),
                    range: TextRange {
                        start_line: step.source_line,
                        start_character: 0,
                        end_line: step.source_line,
                        end_character: 0,
                    },
                    owner: "sem-lang/pipeline".into(),
                    recovery: "complete-pipeline-step".into(),
                });
            }
            if let Some(value) = missing_binding {
                diagnostics.push(Diagnostic {
                    code: "SEM0120".into(),
                    severity: Severity::Error,
                    message: format!(
                        "The input `{}` resolves only to a semantic term. Select a typed value binding that names its authority and producer.",
                        value.label.trim_start_matches("- ").trim()
                    ),
                    range: TextRange {
                        start_line: value.source_line,
                        start_character: 0,
                        end_line: value.source_line,
                        end_character: 0,
                    },
                    owner: "sem-lang/value-binding".into(),
                    recovery: "select-value-binding".into(),
                });
            }
            if let Some(binding) = invalid_binding {
                diagnostics.push(Diagnostic {
                    code: "SEM0121".into(),
                    severity: Severity::Error,
                    message: format!(
                        "The value producer {} does not satisfy the declared {} cardinality for {}.",
                        binding.producer_ref, binding.cardinality, binding.value_ref
                    ),
                    range: TextRange {
                        start_line: pipeline.section_line,
                        start_character: 0,
                        end_line: pipeline.section_line,
                        end_character: 0,
                    },
                    owner: binding.authority_ref.clone(),
                    recovery: "repair-value-producer".into(),
                });
            }
            if let Err(error) = shape {
                diagnostics.push(Diagnostic {
                    code: "SEM0111".into(),
                    severity: Severity::Error,
                    message: format!("The declarative pipeline shape is invalid: {error}."),
                    range: TextRange {
                        start_line: pipeline.section_line,
                        start_character: 0,
                        end_line: pipeline.section_line,
                        end_character: 0,
                    },
                    owner: "sem-lang/pipeline".into(),
                    recovery: "repair-pipeline-shape".into(),
                });
            }
        }
        execution_choices.sort_by(|left, right| {
            (left.source_line, &left.decision_ref).cmp(&(right.source_line, &right.decision_ref))
        });
        let nodes = nodes.into_values().collect::<Vec<_>>();
        let projection_digest = Digest::from_serializable(
            1,
            &(&self.semantic_revision(), &source_digest, &nodes, &edges),
        )
        .expect("tooling projection contains serializable values")
        .to_hex();
        Analysis {
            schema_version: SCHEMA_VERSION.into(),
            semantic_revision: self.semantic_revision(),
            source_digest,
            diagnostics,
            execution_choices,
            pipelines,
            projection: TechnicalProjection {
                schema_version: SCHEMA_VERSION.into(),
                semantic_revision: self.semantic_revision(),
                projection_digest,
                nodes,
                edges,
            },
        }
    }
}

/// Compile the current project sources and atomically publish the exact
/// derived artifact to the configured project graph. A byte-identical retry
/// returns the existing receipt without creating another graph revision.
fn valid_pointer(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 256
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .skip(1)
            .all(|part| !part.contains('~') || valid_pointer_segment(part))
}

fn valid_pointer_segment(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'~' {
            if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                return false;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    true
}

fn valid_configuration_ref(value: &str) -> bool {
    let Some((scheme, path)) = value.split_once("://") else {
        return false;
    };
    !scheme.is_empty()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !path.is_empty()
        && value.len() <= 256
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && !path
            .chars()
            .any(|character| matches!(character, '?' | '#' | '@'))
}

fn decode_pointer_segment(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match characters.next()? {
            '0' => decoded.push('~'),
            '1' => decoded.push('/'),
            _ => return None,
        }
    }
    Some(decoded)
}

fn schema_at_pointer<'a>(schema: &'a ValueSchema, pointer: &str) -> Option<&'a ValueSchema> {
    let mut current = schema;
    for part in pointer.split('/').skip(1) {
        let key = decode_pointer_segment(part)?;
        let ValueSchema::Object { fields, .. } = current else {
            return None;
        };
        current = fields.get(&key)?;
    }
    Some(current)
}

fn pointer(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn required_input_paths(schema: &ValueSchema, parent: &str, output: &mut Vec<String>) {
    let ValueSchema::Object { fields, required } = schema else {
        if !parent.is_empty() {
            output.push(parent.into());
        }
        return;
    };
    for key in required {
        let path = pointer(parent, key);
        match fields.get(key) {
            Some(ValueSchema::Object { required, .. }) if !required.is_empty() => {
                required_input_paths(&fields[key], &path, output);
            }
            Some(_) | None => output.push(path),
        }
    }
}

fn validate_value_binding_terms(
    modules: &[semantic_module::CompiledSemanticModule],
    vocabularies: &[VocabularyModule],
) -> Result<(), ToolingError> {
    let terms = vocabularies
        .iter()
        .flat_map(|vocabulary| {
            vocabulary
                .terms
                .iter()
                .map(move |term| (term.id.to_hex(), (vocabulary.vocabulary_id.as_str(), term)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut bound_terms = BTreeSet::new();
    for binding in modules.iter().flat_map(|module| &module.value_bindings) {
        let Some((source_ref, term)) = terms.get(&binding.term_id) else {
            return Err(ToolingError::Vocabulary(format!(
                "semantic value binding {} references an unknown term",
                binding.value_ref
            )));
        };
        if *source_ref != binding.term_source_ref || term.label != binding.term_name {
            return Err(ToolingError::Vocabulary(format!(
                "semantic value binding {} does not match its exact term source",
                binding.value_ref
            )));
        }
        if !bound_terms.insert(binding.term_id.as_str()) {
            return Err(ToolingError::Vocabulary(format!(
                "semantic term {} has more than one selected value binding",
                binding.term_name
            )));
        }
        if binding.producer_ref == "project/source"
            && binding
                .selector
                .as_deref()
                .is_none_or(|value| !value.ends_with(".sem"))
        {
            return Err(ToolingError::Vocabulary(format!(
                "project source value binding {} requires an exact .sem selector",
                binding.value_ref
            )));
        }
        if binding.producer_ref == "project/resource"
            && binding.selector.as_deref().is_none_or(|selector| {
                !modules
                    .iter()
                    .flat_map(|module| &module.resources)
                    .any(|resource| resource.resource_ref == selector)
            })
        {
            return Err(ToolingError::Vocabulary(format!(
                "project resource value binding {} requires an exact selected resource",
                binding.value_ref
            )));
        }
        if !matches!(
            binding.producer_ref.as_str(),
            "project/source" | "project/resource"
        ) && binding.selector.is_some()
        {
            return Err(ToolingError::Vocabulary(format!(
                "value binding {} supplies a selector to a producer that does not accept one",
                binding.value_ref
            )));
        }
        if !matches!(
            binding.producer_ref.as_str(),
            "workspace/activity/current"
                | "workspace/plan/all"
                | "workspace/history/all"
                | "workspace/record-set/all"
                | "workspace/output/all"
                | "project/source"
                | "project/resource"
        ) {
            return Err(ToolingError::Vocabulary(format!(
                "semantic value binding {} uses an unknown value producer {}",
                binding.value_ref, binding.producer_ref
            )));
        }
        let produced_type = match binding.producer_ref.as_str() {
            "workspace/activity/current" => "kind/work/activity",
            "workspace/plan/all" => "kind/work/plan-entry",
            "workspace/history/all" => "kind/work/history",
            "workspace/record-set/all" => "kind/work/record-set",
            "workspace/output/all" => "kind/work/output",
            "project/source" => "kind/project/semantic-unit",
            "project/resource" => modules
                .iter()
                .flat_map(|module| &module.resources)
                .find(|resource| {
                    Some(resource.resource_ref.as_str()) == binding.selector.as_deref()
                })
                .map(|resource| resource.resource_kind.as_str())
                .expect("selected project resource was checked above"),
            _ => unreachable!("producer was checked above"),
        };
        if binding.value_type_ref != produced_type {
            return Err(ToolingError::Vocabulary(format!(
                "semantic value binding {} declares type {} but producer {} yields {}",
                binding.value_ref, binding.value_type_ref, binding.producer_ref, produced_type
            )));
        }
    }
    Ok(())
}

fn target_availability(value: &Availability) -> TargetAvailability {
    match value {
        Availability::Available => TargetAvailability::Available,
        Availability::Unavailable { .. } => TargetAvailability::Unavailable,
        Availability::Forbidden { .. } => TargetAvailability::Forbidden,
        Availability::PreconditionFailed { .. } => TargetAvailability::PreconditionFailed,
    }
}

fn availability_reason(value: &Availability) -> Option<String> {
    match value {
        Availability::Available => None,
        Availability::Unavailable { reason }
        | Availability::Forbidden { reason }
        | Availability::PreconditionFailed { reason } => Some(reason.clone()),
    }
}

const fn preview_state_name(value: PreviewState) -> &'static str {
    match value {
        PreviewState::NeedsInput => "needs-input",
        PreviewState::NeedsTarget => "needs-target",
        PreviewState::NeedsAuthentication => "needs-authentication",
        PreviewState::Ready => "ready",
        PreviewState::Unavailable => "unavailable",
    }
}

const fn execution_choice_state(value: PreviewState) -> ExecutionChoiceState {
    match value {
        PreviewState::NeedsInput => ExecutionChoiceState::NeedsInput,
        PreviewState::NeedsTarget => ExecutionChoiceState::NeedsTarget,
        PreviewState::NeedsAuthentication => ExecutionChoiceState::NeedsAuthentication,
        PreviewState::Ready => ExecutionChoiceState::Ready,
        PreviewState::Unavailable => ExecutionChoiceState::Unavailable,
    }
}

const fn preview_field_state_name(value: PreviewFieldState) -> &'static str {
    match value {
        PreviewFieldState::ObservedNeedsNormalization => "observed-needs-normalization",
        PreviewFieldState::Missing => "missing",
    }
}

const fn target_availability_name(value: TargetAvailability) -> &'static str {
    match value {
        TargetAvailability::Available => "available",
        TargetAvailability::Unavailable => "unavailable",
        TargetAvailability::Forbidden => "forbidden",
        TargetAvailability::PreconditionFailed => "precondition-failed",
    }
}

const fn authentication_state_name(value: AuthenticationState) -> &'static str {
    match value {
        AuthenticationState::Ready => "ready",
        AuthenticationState::AuthorizationRequired => "authorization-required",
        AuthenticationState::Unavailable => "unavailable",
    }
}

fn check_version(value: &str, owner: &str) -> Result<(), ToolingError> {
    if value == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(ToolingError::Version(owner.into()))
    }
}

pub(crate) fn validate_project_layout(manifest: &ProjectManifest) -> Result<(), ToolingError> {
    fn below(value: &str, root: &str) -> bool {
        value == root
            || value
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    let source_is_visible = |value: &str| {
        value
            .split('/')
            .next()
            .is_some_and(|segment| !segment.starts_with('.'))
    };
    if manifest.semantic_packages.len() > 64
        || manifest.semantic_packages.iter().any(|package| {
            !below(&package.manifest, "packages") || !package.manifest.ends_with("/package.sem")
        })
        || manifest
            .workspace_graph
            .as_deref()
            .is_some_and(|value| !below(value, ".sem-lang/data"))
        || manifest
            .service_roots
            .iter()
            .any(|value| !below(value, ".sem-lang/services"))
        || manifest
            .service_state_roots
            .iter()
            .any(|value| !below(value, ".sem-lang/runtime/services"))
        || manifest
            .behavior_packages
            .iter()
            .any(|value| !below(&value.manifest, ".sem-lang/packages"))
        || manifest
            .procedure_packages
            .iter()
            .any(|value| !below(&value.manifest, ".sem-lang/packages"))
        || manifest
            .source_documents
            .iter()
            .any(|value| !source_is_visible(&value.path))
    {
        return Err(ToolingError::Capability(
            "semantic packages and authored sources must be visible .sem files; machine-owned project inputs must remain under .sem-lang"
                .into(),
        ));
    }
    if manifest
        .source_documents
        .iter()
        .filter(|source| source.role == ProjectSourceRole::ProjectLanguage)
        .count()
        != 1
    {
        return Err(ToolingError::Capability(
            "project must declare exactly one project_language source".into(),
        ));
    }
    Ok(())
}

fn valid_locale(value: &str) -> bool {
    value.len() <= 64
        && value == value.trim()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part.len() <= 16
                && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn valid_time_zone(value: &str) -> bool {
    value == "UTC"
        || (value.len() <= 128
            && value == value.trim()
            && value.contains('/')
            && !value.contains("//")
            && value.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+')
                    })
            }))
}

fn source_role_diagnostics(
    role: ProjectSourceRole,
    source: &str,
    pipelines: &[PipelineProjection],
    vocabularies: &[VocabularyModule],
) -> Vec<Diagnostic> {
    let accepted_labels: &[&str] = match role {
        ProjectSourceRole::ProjectLanguage => return Vec::new(),
        ProjectSourceRole::Work => &["Request", "Proposal"],
        ProjectSourceRole::Records => &["Assertion", "Report"],
    };
    let accepted = vocabularies
        .iter()
        .flat_map(|vocabulary| &vocabulary.terms)
        .filter(|term| accepted_labels.contains(&term.label.as_str()))
        .map(|term| term.id.to_hex())
        .collect::<BTreeSet<_>>();
    let lines = source.lines().collect::<Vec<_>>();
    let mut diagnostics = Vec::new();
    for pipeline in pipelines {
        let section = pipeline.section_term_id.as_deref();
        let invalid_section = !section.is_some_and(|value| accepted.contains(value));
        let records_with_effect = role == ProjectSourceRole::Records
            && pipeline.steps.iter().any(|step| step.operator == "effect");
        if invalid_section || records_with_effect {
            let raw = lines
                .get(pipeline.section_line as usize)
                .copied()
                .unwrap_or_default();
            diagnostics.push(Diagnostic {
                code: "SEM0102".into(),
                severity: Severity::Error,
                message: match role {
                    ProjectSourceRole::Work => {
                        "A work source may contain only request or proposal pipelines.".into()
                    }
                    ProjectSourceRole::Records => {
                        "A records source may contain only assertion or report pipelines and cannot declare effects.".into()
                    }
                    ProjectSourceRole::ProjectLanguage => unreachable!(),
                },
                range: line_range(pipeline.section_line, raw),
                owner: "sem-lang/source-role".into(),
                recovery: "move-statement-to-matching-source-role".into(),
            });
        }
    }
    diagnostics
}

#[cfg(test)]
mod source_role_tests {
    use super::*;
    use sem_lang_core::TermId;

    fn pipeline(section: &str, operator: &str) -> PipelineProjection {
        PipelineProjection {
            section_line: 0,
            section_term_id: Some(section.into()),
            state: "structured".into(),
            steps: vec![PipelineStepProjection {
                operator: operator.into(),
                operator_term_id: "operator/test".into(),
                source_line: 1,
                values: Vec::new(),
            }],
        }
    }

    #[test]
    fn source_roles_reject_cross_role_and_effectful_records() {
        let vocabulary = VocabularyModule::from_sem(
            "# 言語\n  識別子: fixture/core\n  版: 0.10.0\n  依存: なし\n\n# 用語\n  名称: Request\n  説明: Request\n  意味: なし\n\n# 用語\n  名称: Proposal\n  説明: Proposal\n  意味: なし\n\n# 用語\n  名称: Assertion\n  説明: Assertion\n  意味: なし\n\n# 用語\n  名称: Report\n  説明: Report\n  意味: なし\n",
        )
        .unwrap();
        let request = TermId::from_declaration("fixture/core", "Request").to_hex();
        let assertion = TermId::from_declaration("fixture/core", "Assertion").to_hex();
        assert!(
            source_role_diagnostics(
                ProjectSourceRole::Work,
                "# Request\n  source",
                &[pipeline(&request, "source")],
                std::slice::from_ref(&vocabulary),
            )
            .is_empty()
        );
        assert_eq!(
            source_role_diagnostics(
                ProjectSourceRole::Work,
                "# Assertion\n  source",
                &[pipeline(&assertion, "source")],
                std::slice::from_ref(&vocabulary),
            )[0]
            .code,
            "SEM0102"
        );
        assert_eq!(
            source_role_diagnostics(
                ProjectSourceRole::Records,
                "# Assertion\n  effect",
                &[pipeline(&assertion, "effect")],
                std::slice::from_ref(&vocabulary),
            )[0]
            .recovery,
            "move-statement-to-matching-source-role"
        );
    }

    #[test]
    fn value_binding_rejects_a_producer_type_mismatch() {
        let vocabulary = VocabularyModule::from_package_sem(
            "fixture/work/language/work",
            "0.10.0",
            "# 言語\n  依存: sem-lang/standard/semantic\n\n# 用語\n  名称: CurrentWork\n  説明: Current work\n  意味: Primitive/Value\n",
        )
        .unwrap();
        let source = "# 意味モジュール\n  所有者: subject/fixture/project\n\n# 主体\n  識別子: subject/fixture/project\n  種別: kind/project\n  表示名: Fixture\n  言語: en-US\n  公開範囲: internal\n  正本: authority/fixture\n  正本の版: revision/fixture/one\n  根拠: reference/fixture\n\n# 値束縛\n  識別子: value/fixture/current-work\n  用語ソース: fixture/work/language/work\n  用語: CurrentWork\n  値の型: kind/work/history\n  件数: many\n  正本: workspace/fixture\n  生成元: workspace/activity/current\n  実体化: reference_only\n  根拠: source/work/current\n";
        let module = semantic_module::compile(
            "fixture/work/modules/current-work",
            "packages/fixture/work/modules/current-work.sem",
            source,
            SCHEMA_VERSION,
        )
        .unwrap();
        let error = validate_value_binding_terms(&[module], &[vocabulary]).unwrap_err();
        assert!(error.to_string().contains("yields kind/work/activity"));
    }

    #[test]
    fn project_resource_binding_uses_the_selected_resources_declared_kind() {
        let vocabulary = VocabularyModule::from_package_sem(
            "fixture/source/language/observation",
            "0.10.0",
            "# 言語\n  依存: sem-lang/standard/semantic\n\n# 用語\n  名称: SourceReference\n  説明: Exact source reference\n  意味: Primitive/Reference\n",
        )
        .unwrap();
        let source = "# 意味モジュール\n  所有者: subject/fixture/project\n\n# 主体\n  識別子: subject/fixture/project\n  種別: kind/project\n  表示名: Fixture\n  言語: en-US\n  公開範囲: internal\n  正本: authority/fixture\n  正本の版: revision/fixture/one\n  根拠: reference/fixture\n\n# 資源\n  識別子: resource/fixture/workbook\n  種別: kind/source/workbook\n  所有者: subject/fixture/project\n  表示名: Workbook\n  言語: en-US\n  アクセス: reference_only\n  データ境界: exact_reference\n  根拠: reference/fixture/workbook\n\n# 値束縛\n  識別子: value/fixture/source-reference\n  用語ソース: fixture/source/language/observation\n  用語: SourceReference\n  値の型: kind/source/workbook\n  件数: one\n  正本: subject/fixture/project\n  生成元: project/resource\n  選択: resource/fixture/workbook\n  実体化: reference_only\n  根拠: reference/fixture/workbook\n";
        let module = semantic_module::compile(
            "fixture/source/modules/workbook",
            "packages/fixture/source/modules/workbook.sem",
            source,
            SCHEMA_VERSION,
        )
        .unwrap();
        validate_value_binding_terms(&[module], &[vocabulary]).unwrap();
    }
}
