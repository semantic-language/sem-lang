//! Public editor and project data contracts.
//!
//! These types describe data at the tooling boundary. Loading, validation and
//! execution routing remain owned by the modules that produce them.

use crate::{procedure, semantic_package};
use serde::{Deserialize, Serialize};
use zixcel_contracts::{BehaviorArtifactRefV1, BehaviorEffectV1};
use zixcel_interaction::ResourceContract;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContributionStructureKind {
    pub term_id: String,
    pub label: String,
    pub surface: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema_version: String,
    pub project_id: String,
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub time_zone: Option<String>,
    pub semantic_packages: Vec<semantic_package::SemanticPackageSelection>,
    pub source_documents: Vec<ProjectSourceDocument>,
    pub service_roots: Vec<String>,
    pub service_state_roots: Vec<String>,
    #[serde(default)]
    pub behavior_packages: Vec<BehaviorPackageSelection>,
    #[serde(default)]
    pub procedure_packages: Vec<procedure::ProcedurePackageSelection>,
    #[serde(default)]
    pub workspace_graph: Option<String>,
    #[serde(default)]
    pub version_control: Option<VersionControlSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VersionControlSelection {
    pub provider: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceVersioningPresentation {
    pub state: String,
    pub match_state: String,
    pub provider: Option<String>,
    pub root: Option<String>,
    pub observation: Option<zixcel_version_control::SourceRevisionObservation>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ProjectCompilationInput {
    pub kind: String,
    pub path: String,
    pub digest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectCompilation {
    pub schema_version: String,
    pub project_id: String,
    pub source_digest: String,
    pub semantic_revision: String,
    pub manifest: ProjectManifest,
    pub inputs: Vec<ProjectCompilationInput>,
    pub artifact_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectCompilationReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub project_id: String,
    pub source_digest: String,
    pub artifact_digest: String,
    pub graph_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectCompilationRetirementReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub project_id: String,
    pub removed_artifacts: usize,
    pub graph_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectSourceDocument {
    pub path: String,
    pub role: ProjectSourceRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectStorageTopology {
    pub project_ref: String,
    pub authored_sources: Vec<String>,
    pub selected_semantic_packages: Vec<String>,
    pub workspace_graph: Option<String>,
    pub service_declarations: Vec<String>,
    pub service_state: Vec<String>,
    pub operation_receipts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BehaviorPackageSelection {
    pub manifest: String,
    pub digest_sha256: String,
    pub publisher_ref: String,
    pub admission_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDefinition {
    pub schema_version: String,
    pub owner: String,
    pub revision: String,
    pub resource: ResourceContract,
    #[serde(default)]
    pub semantic_bindings: Vec<CapabilityBinding>,
    #[serde(default)]
    pub input_bindings: Vec<CapabilityInputBinding>,
    pub authentication: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationDefinition {
    pub schema_version: String,
    pub owner: String,
    pub revision: String,
    pub label: String,
    pub configuration_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationStateProjection {
    pub schema_version: String,
    pub owner: String,
    pub revision: String,
    pub state: AuthenticationState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationState {
    Ready,
    AuthorizationRequired,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityBinding {
    pub term_id: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInputBinding {
    pub term_id: String,
    pub operation_id: String,
    pub input_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Information,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub range: TextRange,
    pub owner: String,
    pub recovery: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    pub insert_text: String,
    pub behavior_bindings: Vec<CompletionBehaviorBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompletionBehaviorBinding {
    pub package_ref: String,
    pub package_revision: String,
    pub behavior_ref: String,
    pub operation_id: String,
    pub effect: BehaviorEffectV1,
    pub state: BehaviorInjectionState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub state: String,
    pub owner: String,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionEdge {
    pub source: String,
    pub relation: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TechnicalProjection {
    pub schema_version: String,
    pub semantic_revision: String,
    pub projection_digest: String,
    pub nodes: Vec<ProjectionNode>,
    pub edges: Vec<ProjectionEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionChoice {
    pub decision_ref: String,
    pub source_line: u32,
    pub state: ExecutionChoiceState,
    pub routes: Vec<ExecutionRoute>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionChoiceState {
    NeedsRoute,
    NeedsInput,
    NeedsTarget,
    NeedsAuthentication,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRoute {
    pub operation_id: String,
    pub preview: PartialExecutionPreview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialExecutionPreview {
    pub state: PreviewState,
    pub targets: Vec<TargetCandidate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewState {
    NeedsInput,
    NeedsTarget,
    NeedsAuthentication,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewField {
    pub path: String,
    pub state: PreviewFieldState,
    pub semantic_term: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewFieldState {
    ObservedNeedsNormalization,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetCandidate {
    pub target: String,
    pub contract_revision: String,
    pub availability: TargetAvailability,
    pub availability_reason: Option<String>,
    pub authentication: Option<AuthenticationProjection>,
    pub fields: Vec<PreviewField>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetAvailability {
    Available,
    Unavailable,
    Forbidden,
    PreconditionFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationProjection {
    pub service_id: String,
    pub label: String,
    pub owner: String,
    pub revision: String,
    pub state_owner: String,
    pub state_revision: String,
    pub state: AuthenticationState,
    pub configuration_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityOperationPresentation {
    pub operation_id: String,
    pub target: String,
    pub owner: String,
    pub revision: String,
    pub availability: TargetAvailability,
    pub availability_reason: Option<String>,
    pub authentication: Option<AuthenticationProjection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorInjectionState {
    Ready,
    Blocked,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BehaviorRequirementPresentation {
    pub behavior_ref: String,
    pub provider_package_ref: Option<String>,
    pub state: BehaviorInjectionState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BehaviorImplementationPresentation {
    pub behavior_ref: String,
    pub operation_id: String,
    pub target: String,
    pub contract_revision: String,
    pub input_type_ref: String,
    pub output_type_ref: String,
    pub effect: BehaviorEffectV1,
    pub state: BehaviorInjectionState,
    pub requirements: Vec<BehaviorRequirementPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BehaviorPackagePresentation {
    pub package_ref: String,
    pub package_revision: String,
    pub owner: String,
    pub artifact: BehaviorArtifactRefV1,
    pub state: BehaviorInjectionState,
    pub implementations: Vec<BehaviorImplementationPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub schema_version: String,
    pub semantic_revision: String,
    pub source_digest: String,
    pub diagnostics: Vec<Diagnostic>,
    pub execution_choices: Vec<ExecutionChoice>,
    pub pipelines: Vec<PipelineProjection>,
    pub projection: TechnicalProjection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineProjection {
    pub section_line: u32,
    pub section_term_id: Option<String>,
    pub state: String,
    pub steps: Vec<PipelineStepProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineStepProjection {
    pub operator: String,
    pub operator_term_id: String,
    pub source_line: u32,
    pub values: Vec<PipelineValueProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineValueProjection {
    pub source_line: u32,
    pub label: String,
    pub term_ids: Vec<String>,
    pub binding_required: bool,
    /// Authority-owned values selected for this authored semantic value.
    /// Empty means no value binding was selected; it never means that the
    /// lexical term itself was accepted as runtime data.
    pub bindings: Vec<PipelineValueBindingProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineValueBindingProjection {
    pub value_ref: String,
    pub term_id: String,
    pub value_type_ref: String,
    pub cardinality: String,
    pub authority_ref: String,
    pub producer_ref: String,
    pub materialization: String,
    pub resolved_value_refs: Vec<String>,
    pub state: String,
    pub definition_source_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSourceAnalysis {
    pub source_ref: String,
    pub role: ProjectSourceRole,
    pub source_bindings: Vec<ProjectSourceBinding>,
    pub analysis: Analysis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectSourceBinding {
    pub semantic_unit_ref: String,
    pub start_line: u32,
    pub end_line: u32,
    pub bound_unit_digest: String,
    pub current_unit_digest: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSourceRole {
    ProjectLanguage,
    Work,
    Records,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticFixEvidence {
    pub id: String,
    pub surface: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct SemanticFixTerm {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticFixRequest {
    pub schema_version: String,
    pub diagnostic_code: String,
    pub owner: String,
    pub recovery: String,
    pub locale: String,
    pub semantic_revision: String,
    pub source_digest: String,
    pub source_line: u32,
    pub instruction: String,
    pub unresolved: Vec<SemanticFixEvidence>,
    pub resolved_terms: Vec<SemanticFixTerm>,
    pub allowed_terms: Vec<SemanticFixTerm>,
    pub allowed_terms_complete: bool,
}
