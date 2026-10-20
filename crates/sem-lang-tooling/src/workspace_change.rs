//! Explicit, idempotent mutations of the canonical project graph.
//!
//! Analysis is read-only. A caller first receives a content-addressed plan and
//! later applies that exact plan against the graph revision it observed.

use crate::workspace::{
    ActivityState, CivilDate, ExactInstant, WorkspaceDependencyEntry,
    WorkspaceMilestonePresentation, WorkspaceOutputPresentation, WorkspacePlanEntry,
    WorkspaceProcedureRunPresentation, WorkspaceRecordSetPresentation, WorkspaceSourceLocator,
    WorkspaceStepRunPresentation, WorkspaceTimelineEntry,
};
use crate::{ProjectManifest, ProjectSourceRole, SCHEMA_VERSION, ToolingError, WorkspaceAnalyzer};
use sem_lang_core::Digest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[path = "workspace_receipts.rs"]
mod workspace_receipts;
use workspace_receipts::persist_receipt;

const MAX_SOURCE_BYTES: u64 = 1_048_576;
const MAX_RECEIPT_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRegistrationRequest {
    pub schema_version: String,
    /// Lowercase hierarchy below `activity/`, without a document suffix.
    pub task_path: String,
    pub activity_state: ActivityState,
    pub owner_ref: String,
    pub method_ref: String,
    pub title: String,
    pub next_instruction: String,
    pub source_ref: String,
    /// Zero-based inclusive first line containing the authored task request.
    pub source_start_line: u32,
    /// Zero-based inclusive last line containing the authored task request.
    pub source_end_line: u32,
    #[serde(default)]
    pub authority_refs: Vec<String>,
    #[serde(default)]
    pub reference_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRegistrationPlan {
    pub schema_version: String,
    pub operation_id: String,
    pub task_ref: String,
    pub expected_graph_revision: WorkspaceGraphRevision,
    pub request: TaskRegistrationRequest,
    pub source: WorkspaceSourceLocator,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskRegistrationReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub task_ref: String,
    pub plan_digest: String,
    pub graph_revision: u64,
}

/// One immutable, source-bound outcome from work that has already happened.
/// History is appended independently from the replaceable current-work view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRegistrationRequest {
    pub schema_version: String,
    /// Lowercase hierarchy below `history/`, without a document suffix.
    pub history_path: String,
    pub title: String,
    pub former_activity_ref: String,
    pub owner_ref: String,
    pub outcome: String,
    pub occurred_at: ExactInstant,
    pub source_revision: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub source_ref: String,
    pub source_start_line: u32,
    pub source_end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRegistrationPlan {
    pub schema_version: String,
    pub operation_id: String,
    pub history_ref: String,
    pub expected_graph_revision: WorkspaceGraphRevision,
    pub request: HistoryRegistrationRequest,
    pub source: WorkspaceSourceLocator,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HistoryRegistrationReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub history_ref: String,
    pub plan_digest: String,
    pub graph_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceGraphRevision {
    pub sequence: u64,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBindingRequest {
    pub schema_version: String,
    pub record_ref: String,
    pub source_ref: String,
    /// Zero-based inclusive line.
    pub start_line: u32,
    /// Zero-based inclusive line.
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBindingPlan {
    pub schema_version: String,
    pub operation_id: String,
    pub expected_graph_revision: WorkspaceGraphRevision,
    pub locator: WorkspaceSourceLocator,
    pub request: SourceBindingRequest,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceBindingReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub record_ref: String,
    pub plan_digest: String,
    pub graph_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMigrationRequest {
    pub schema_version: String,
    pub bindings: Vec<SourceBindingRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMigrationEntry {
    pub record_ref: String,
    pub previous: WorkspaceSourceLocator,
    pub replacement: WorkspaceSourceLocator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMigrationPlan {
    pub schema_version: String,
    pub operation_id: String,
    pub expected_graph_revision: WorkspaceGraphRevision,
    pub entries: Vec<SourceMigrationEntry>,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceMigrationReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub migrated_records: usize,
    pub plan_digest: String,
    pub graph_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkTrackingRequest {
    pub schema_version: String,
    pub source_ref: String,
    pub source_start_line: u32,
    pub source_end_line: u32,
    pub plans: Vec<WorkspacePlanEntry>,
    pub timeline: Vec<WorkspaceTimelineEntry>,
    pub dependencies: Vec<WorkspaceDependencyEntry>,
    pub milestones: Vec<WorkspaceMilestonePresentation>,
    pub record_sets: Vec<WorkspaceRecordSetRequest>,
    pub outputs: Vec<WorkspaceOutputPresentation>,
    pub procedure_runs: Vec<WorkspaceProcedureRunPresentation>,
    pub step_runs: Vec<WorkspaceStepRunPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkTrackingPlan {
    pub schema_version: String,
    pub operation_id: String,
    pub expected_graph_revision: WorkspaceGraphRevision,
    pub request: WorkTrackingRequest,
    /// Immutable records derived by the graph owner for the exact observed revision.
    pub resolved_record_sets: Vec<WorkspaceRecordSetPresentation>,
    pub source: WorkspaceSourceLocator,
    pub content_digest: String,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecordSetRequest {
    pub record_set_ref: String,
    pub activity_ref: String,
    pub label: String,
    pub state: String,
    pub period_start: CivilDate,
    pub period_end: CivilDate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkTrackingReceipt {
    pub schema_version: String,
    pub operation_id: String,
    pub plan_digest: String,
    pub graph_revision: u64,
}

trait ImmutableReceipt: Serialize {
    fn plan_digest(&self) -> &str;
}

macro_rules! immutable_receipt {
    ($($receipt:ty),+ $(,)?) => {
        $(
            impl ImmutableReceipt for $receipt {
                fn plan_digest(&self) -> &str {
                    &self.plan_digest
                }
            }
        )+
    };
}

immutable_receipt!(
    TaskRegistrationReceipt,
    HistoryRegistrationReceipt,
    SourceBindingReceipt,
    SourceMigrationReceipt,
    WorkTrackingReceipt,
);

pub fn plan_task_registration(
    project_file: &Path,
    request: TaskRegistrationRequest,
) -> Result<TaskRegistrationPlan, ToolingError> {
    validate_task_registration(project_file, &request)?;
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let operation_id = format!("workspace/task/register/{}", request.task_path);
    let task_ref = format!("activity/{}/task", request.task_path);
    let source = validated_source_locator(
        &base,
        &manifest,
        &SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: task_ref.clone(),
            source_ref: request.source_ref.clone(),
            start_line: request.source_start_line,
            end_line: request.source_end_line,
        },
    )?;
    let expected_graph_revision =
        crate::workspace_graph::revision(&base, &graph_ref, &manifest.project_id)?;
    let mut plan = TaskRegistrationPlan {
        schema_version: SCHEMA_VERSION.into(),
        operation_id,
        task_ref,
        expected_graph_revision,
        request,
        source,
        plan_digest: String::new(),
    };
    plan.plan_digest = plan_digest(&plan)?;
    Ok(plan)
}

pub fn apply_task_registration(
    project_file: &Path,
    plan: &TaskRegistrationPlan,
) -> Result<TaskRegistrationReceipt, ToolingError> {
    validate_plan_shape(plan)?;
    match validate_task_registration(project_file, &plan.request) {
        Ok(()) => {}
        Err(error) => {
            let analyzer = WorkspaceAnalyzer::load(project_file)?;
            let exists = analyzer
                .analyze("")
                .projection
                .nodes
                .iter()
                .any(|node| node.id == plan.task_ref);
            if !exists {
                return Err(error);
            }
        }
    }
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let current_source = validated_source_locator(
        &base,
        &manifest,
        &SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: plan.task_ref.clone(),
            source_ref: plan.request.source_ref.clone(),
            start_line: plan.request.source_start_line,
            end_line: plan.request.source_end_line,
        },
    )?;
    if current_source != plan.source {
        return Err(change_error(
            "task source changed after the registration was planned",
        ));
    }
    let receipt = crate::workspace_graph::register_task(
        &base,
        &graph_ref,
        &manifest.project_id,
        &plan.expected_graph_revision,
        &plan.plan_digest,
        &plan.request,
        &plan.source,
    )?;
    persist_receipt(project_file, &receipt)?;
    Ok(receipt)
}

pub fn plan_history_registration(
    project_file: &Path,
    request: HistoryRegistrationRequest,
) -> Result<HistoryRegistrationPlan, ToolingError> {
    validate_history_registration(project_file, &request)?;
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let operation_id = format!("workspace/history/register/{}", request.history_path);
    let history_ref = format!("history/{}/record", request.history_path);
    let source = validated_source_locator(
        &base,
        &manifest,
        &SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: history_ref.clone(),
            source_ref: request.source_ref.clone(),
            start_line: request.source_start_line,
            end_line: request.source_end_line,
        },
    )?;
    let expected_graph_revision =
        crate::workspace_graph::revision(&base, &graph_ref, &manifest.project_id)?;
    let mut plan = HistoryRegistrationPlan {
        schema_version: SCHEMA_VERSION.into(),
        operation_id,
        history_ref,
        expected_graph_revision,
        request,
        source,
        plan_digest: String::new(),
    };
    plan.plan_digest = history_plan_digest(&plan)?;
    Ok(plan)
}

pub fn apply_history_registration(
    project_file: &Path,
    plan: &HistoryRegistrationPlan,
) -> Result<HistoryRegistrationReceipt, ToolingError> {
    validate_history_plan_shape(plan)?;
    if let Err(error) = validate_history_registration(project_file, &plan.request) {
        let exists = WorkspaceAnalyzer::load(project_file)?
            .analyze("")
            .projection
            .nodes
            .iter()
            .any(|node| node.id == plan.history_ref);
        if !exists {
            return Err(error);
        }
    }
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let current_source = validated_source_locator(
        &base,
        &manifest,
        &SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: plan.history_ref.clone(),
            source_ref: plan.request.source_ref.clone(),
            start_line: plan.request.source_start_line,
            end_line: plan.request.source_end_line,
        },
    )?;
    if current_source != plan.source {
        return Err(change_error(
            "history source changed after registration was planned",
        ));
    }
    let receipt = crate::workspace_graph::register_history(
        &base,
        &graph_ref,
        &manifest.project_id,
        &plan.expected_graph_revision,
        &plan.plan_digest,
        &plan.request,
        &plan.source,
    )?;
    persist_receipt(project_file, &receipt)?;
    Ok(receipt)
}

pub fn plan_source_binding(
    project_file: &Path,
    request: SourceBindingRequest,
) -> Result<SourceBindingPlan, ToolingError> {
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let analyzer = WorkspaceAnalyzer::load(project_file)?;
    if !analyzer
        .analyze("")
        .projection
        .nodes
        .iter()
        .any(|node| node.id == request.record_ref)
    {
        return Err(change_error("source binding record does not exist"));
    }
    let locator = validated_source_locator(&base, &manifest, &request)?;
    let expected_graph_revision =
        crate::workspace_graph::revision(&base, &graph_ref, &manifest.project_id)?;
    let operation_id = format!("workspace/source/bind/{}", request.record_ref);
    let mut plan = SourceBindingPlan {
        schema_version: SCHEMA_VERSION.into(),
        operation_id,
        expected_graph_revision,
        locator,
        request,
        plan_digest: String::new(),
    };
    plan.plan_digest = source_plan_digest(&plan)?;
    Ok(plan)
}

pub fn apply_source_binding(
    project_file: &Path,
    plan: &SourceBindingPlan,
) -> Result<SourceBindingReceipt, ToolingError> {
    if plan.schema_version != SCHEMA_VERSION
        || plan.operation_id != format!("workspace/source/bind/{}", plan.request.record_ref)
        || plan.plan_digest != source_plan_digest(plan)?
    {
        return Err(change_error("source binding plan is invalid"));
    }
    let (base, manifest, graph_ref) = project_context(project_file)?;
    if validated_source_locator(&base, &manifest, &plan.request)? != plan.locator {
        return Err(change_error("source changed after the binding was planned"));
    }
    let receipt = crate::workspace_graph::bind_source(
        &base,
        &graph_ref,
        &manifest.project_id,
        &plan.expected_graph_revision,
        &plan.plan_digest,
        &plan.request.record_ref,
        &plan.locator,
    )?;
    persist_receipt(project_file, &receipt)?;
    Ok(receipt)
}

/// Plan an atomic move of existing durable source locators to a new source
/// layout. The old locator is captured from the graph; callers cannot invent
/// or omit the state being replaced.
pub fn plan_source_migration(
    project_file: &Path,
    request: SourceMigrationRequest,
) -> Result<SourceMigrationPlan, ToolingError> {
    if request.schema_version != SCHEMA_VERSION || request.bindings.is_empty() {
        return Err(change_error("source migration input is invalid"));
    }
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let projection = crate::workspace_graph::load(&base, &graph_ref, &manifest.project_id)?;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::with_capacity(request.bindings.len());
    for binding in request.bindings {
        if !seen.insert(binding.record_ref.clone()) || binding.record_ref.starts_with("source/") {
            return Err(change_error(
                "source migration contains a duplicate or generated source record",
            ));
        }
        let previous = projection
            .presentation
            .records
            .iter()
            .find(|record| record.record_id == binding.record_ref)
            .and_then(|record| record.source.clone())
            .ok_or_else(|| change_error("source migration record is not currently bound"))?;
        let replacement = validated_source_locator(&base, &manifest, &binding)?;
        entries.push(SourceMigrationEntry {
            record_ref: binding.record_ref,
            previous,
            replacement,
        });
    }
    let replacements = entries
        .iter()
        .map(|entry| {
            (
                entry.record_ref.as_str(),
                entry.replacement.source_ref.as_str(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    ensure_unique_activity_sources(projection.presentation.records.iter().filter_map(|record| {
        if !record.record_id.starts_with("activity/") {
            return None;
        }
        replacements
            .get(record.record_id.as_str())
            .copied()
            .or_else(|| {
                record
                    .source
                    .as_ref()
                    .map(|source| source.source_ref.as_str())
            })
            .map(|source_ref| (record.record_id.as_str(), source_ref))
    }))?;
    entries.sort_by(|left, right| left.record_ref.cmp(&right.record_ref));
    let expected_graph_revision =
        crate::workspace_graph::revision(&base, &graph_ref, &manifest.project_id)?;
    let mut plan = SourceMigrationPlan {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: "workspace/source/migrate".into(),
        expected_graph_revision,
        entries,
        plan_digest: String::new(),
    };
    plan.plan_digest = migration_plan_digest(&plan)?;
    Ok(plan)
}

pub fn apply_source_migration(
    project_file: &Path,
    plan: &SourceMigrationPlan,
) -> Result<SourceMigrationReceipt, ToolingError> {
    if plan.schema_version != SCHEMA_VERSION
        || plan.operation_id != "workspace/source/migrate"
        || plan.entries.is_empty()
        || plan.plan_digest != migration_plan_digest(plan)?
    {
        return Err(change_error("source migration plan is invalid"));
    }
    let (base, manifest, graph_ref) = project_context(project_file)?;
    for entry in &plan.entries {
        let request = SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: entry.record_ref.clone(),
            source_ref: entry.replacement.source_ref.clone(),
            start_line: entry.replacement.start_line,
            end_line: entry.replacement.end_line,
        };
        if validated_source_locator(&base, &manifest, &request)? != entry.replacement {
            return Err(change_error(
                "source changed after the migration was planned",
            ));
        }
    }
    let receipt = crate::workspace_graph::migrate_sources(
        &base,
        &graph_ref,
        &manifest.project_id,
        &plan.expected_graph_revision,
        &plan.plan_digest,
        &plan.entries,
    )?;
    persist_receipt(project_file, &receipt)?;
    Ok(receipt)
}

pub fn plan_work_tracking(
    project_file: &Path,
    request: WorkTrackingRequest,
) -> Result<WorkTrackingPlan, ToolingError> {
    let resolved_record_sets = validate_work_tracking(project_file, &request)?;
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let source = validated_source_locator(
        &base,
        &manifest,
        &SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: "tracking/project/work/record".into(),
            source_ref: request.source_ref.clone(),
            start_line: request.source_start_line,
            end_line: request.source_end_line,
        },
    )?;
    let mut plan = WorkTrackingPlan {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: "workspace/work/tracking/replace".into(),
        expected_graph_revision: crate::workspace_graph::revision(
            &base,
            &graph_ref,
            &manifest.project_id,
        )?,
        request,
        resolved_record_sets,
        source,
        content_digest: String::new(),
        plan_digest: String::new(),
    };
    plan.content_digest = work_tracking_content_digest(&plan.request, &plan.source)?;
    plan.plan_digest = work_tracking_digest(&plan)?;
    Ok(plan)
}

pub fn apply_work_tracking(
    project_file: &Path,
    plan: &WorkTrackingPlan,
) -> Result<WorkTrackingReceipt, ToolingError> {
    if plan.schema_version != SCHEMA_VERSION
        || plan.operation_id != "workspace/work/tracking/replace"
        || plan.content_digest != work_tracking_content_digest(&plan.request, &plan.source)?
        || plan.plan_digest != work_tracking_digest(plan)?
    {
        return Err(change_error("work tracking plan is invalid"));
    }
    let resolved_record_sets = validate_work_tracking(project_file, &plan.request)?;
    if resolved_record_sets != plan.resolved_record_sets {
        return Err(change_error(
            "workspace records changed after work tracking was planned",
        ));
    }
    let (base, manifest, graph_ref) = project_context(project_file)?;
    let source = validated_source_locator(
        &base,
        &manifest,
        &SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: "tracking/project/work/record".into(),
            source_ref: plan.request.source_ref.clone(),
            start_line: plan.request.source_start_line,
            end_line: plan.request.source_end_line,
        },
    )?;
    if source != plan.source {
        return Err(change_error(
            "work tracking source changed after the operation was planned",
        ));
    }
    let receipt = crate::workspace_graph::replace_work_tracking(
        &base,
        &graph_ref,
        crate::workspace_graph::WorkTrackingReplacement {
            project_id: &manifest.project_id,
            expected_revision: &plan.expected_graph_revision,
            plan_digest: &plan.plan_digest,
            content_digest: &plan.content_digest,
            request: &plan.request,
            resolved_record_sets: &plan.resolved_record_sets,
            source_locator: &plan.source,
        },
    )?;
    persist_receipt(project_file, &receipt)?;
    Ok(receipt)
}

fn validate_work_tracking(
    project_file: &Path,
    request: &WorkTrackingRequest,
) -> Result<Vec<WorkspaceRecordSetPresentation>, ToolingError> {
    if request.schema_version != SCHEMA_VERSION
        || request.source_start_line > request.source_end_line
        || request.plans.is_empty()
    {
        return Err(change_error("work tracking input is invalid"));
    }
    let analyzer = WorkspaceAnalyzer::load(project_file)?;
    let analysis = analyzer.analyze("");
    let records = analysis
        .projection
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let activities = analysis
        .projection
        .nodes
        .iter()
        .filter(|node| node.kind == "activity")
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let valid_ref = |value: &str| zixcel_interaction::valid_reference(value);
    let mut plan_positions = BTreeSet::new();
    let mut planned_tasks = BTreeSet::new();
    let mut plan_parent = std::collections::BTreeMap::new();
    for entry in &request.plans {
        if !valid_ref(&entry.plan_ref)
            || !activities.contains(entry.task_ref.as_str())
            || entry
                .parent_task_ref
                .as_deref()
                .is_some_and(|parent| !activities.contains(parent))
            || entry.parent_task_ref.as_deref() == Some(entry.task_ref.as_str())
            || !plan_positions.insert((
                entry.plan_ref.as_str(),
                entry.parent_task_ref.as_deref(),
                entry.order,
            ))
            || !planned_tasks.insert((entry.plan_ref.as_str(), entry.task_ref.as_str()))
        {
            return Err(change_error("work plan entry is invalid or ambiguous"));
        }
        plan_parent.insert(
            (entry.plan_ref.as_str(), entry.task_ref.as_str()),
            entry.parent_task_ref.as_deref(),
        );
    }
    for entry in &request.plans {
        if let Some(parent) = entry.parent_task_ref.as_deref()
            && !planned_tasks.contains(&(entry.plan_ref.as_str(), parent))
        {
            return Err(change_error(
                "work plan parent is not part of the same plan",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(entry.task_ref.as_str());
        while let Some(task) = current {
            if !seen.insert(task) {
                return Err(change_error("work plan hierarchy contains a cycle"));
            }
            current = plan_parent
                .get(&(entry.plan_ref.as_str(), task))
                .copied()
                .flatten();
        }
    }
    let mut scheduled = BTreeSet::new();
    for entry in &request.timeline {
        if !valid_ref(&entry.time_ref)
            || !activities.contains(entry.activity_ref.as_str())
            || entry.starts_at >= entry.ends_at
            || !scheduled.insert((entry.time_ref.as_str(), entry.activity_ref.as_str()))
        {
            return Err(change_error("work timeline entry is invalid or duplicated"));
        }
    }
    let known_tracking_ref = |value: &str| {
        activities.contains(value)
            || request
                .milestones
                .iter()
                .any(|entry| entry.milestone_ref == value)
            || request
                .record_sets
                .iter()
                .any(|entry| entry.record_set_ref == value)
            || request
                .outputs
                .iter()
                .any(|entry| entry.output_ref == value)
    };
    let mut dependency_pairs = BTreeSet::new();
    for entry in &request.dependencies {
        if entry.prerequisite_ref == entry.dependent_ref
            || !known_tracking_ref(&entry.prerequisite_ref)
            || !known_tracking_ref(&entry.dependent_ref)
            || !dependency_pairs.insert((
                entry.prerequisite_ref.as_str(),
                entry.dependent_ref.as_str(),
            ))
        {
            return Err(change_error("work dependency is invalid or duplicated"));
        }
    }
    let mut milestones = BTreeSet::new();
    for entry in &request.milestones {
        if !valid_ref(&entry.milestone_ref)
            || !activities.contains(entry.activity_ref.as_str())
            || !valid_one_line(&entry.label, 256)
            || !matches!(
                entry.state.as_str(),
                "planned" | "active" | "achieved" | "missed" | "cancelled"
            )
            || !valid_one_line(&entry.target_value, 1_024)
            || entry
                .actual_value
                .as_deref()
                .is_some_and(|value| !valid_one_line(value, 1_024))
            || entry
                .variance
                .as_deref()
                .is_some_and(|value| !valid_one_line(value, 2_048))
            || entry.evidence_refs.iter().any(|value| !valid_ref(value))
            || !milestones.insert(entry.milestone_ref.as_str())
        {
            return Err(change_error("work milestone is invalid or duplicated"));
        }
    }
    let history = analyzer
        .workspace_presentation()
        .ok_or_else(|| change_error("work evidence collection requires a workspace graph"))?;
    let mut collections = BTreeSet::new();
    let mut resolved_record_sets = Vec::with_capacity(request.record_sets.len());
    for entry in &request.record_sets {
        if !valid_ref(&entry.record_set_ref)
            || !activities.contains(entry.activity_ref.as_str())
            || !valid_one_line(&entry.label, 256)
            || !matches!(
                entry.state.as_str(),
                "planned" | "collecting" | "historical" | "classified" | "reviewed"
            )
            || entry.period_start > entry.period_end
            || !collections.insert(entry.record_set_ref.as_str())
        {
            return Err(change_error(
                "work record set request is invalid or duplicated",
            ));
        }
        resolved_record_sets.push(WorkspaceRecordSetPresentation {
            record_set_ref: entry.record_set_ref.clone(),
            activity_ref: entry.activity_ref.clone(),
            label: entry.label.clone(),
            state: entry.state.clone(),
            period_start: entry.period_start.clone(),
            period_end: entry.period_end.clone(),
            record_refs: history.history_refs_in_period(&entry.period_start, &entry.period_end),
        });
    }
    let mut outputs = BTreeSet::new();
    for entry in &request.outputs {
        if !valid_ref(&entry.output_ref)
            || !activities.contains(entry.activity_ref.as_str())
            || !valid_one_line(&entry.label, 256)
            || !matches!(
                entry.state.as_str(),
                "planned" | "draft" | "reviewed" | "final"
            )
            || !valid_one_line(&entry.role, 128)
            || !valid_ref(&entry.operation_id)
            || entry.evidence_refs.iter().any(|value| !valid_ref(value))
            || !outputs.insert(entry.output_ref.as_str())
        {
            return Err(change_error("work output is invalid or duplicated"));
        }
    }
    let procedure_steps = analyzer
        .procedure_package_presentation()
        .iter()
        .flat_map(|package| &package.procedures)
        .map(|procedure| {
            (
                procedure.procedure_ref.as_str(),
                procedure
                    .steps
                    .iter()
                    .map(|step| (step.step_ref.as_str(), step))
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let data_roles = analyzer
        .procedure_package_presentation()
        .iter()
        .flat_map(|package| &package.data_roles)
        .map(|role| (role.data_role_ref.as_str(), role))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut runs = std::collections::BTreeMap::new();
    for run in &request.procedure_runs {
        let Some(steps) = procedure_steps.get(run.procedure_ref.as_str()) else {
            return Err(change_error(
                "procedure run refers to an unselected procedure",
            ));
        };
        if !valid_ref(&run.run_ref)
            || !activities.contains(run.activity_ref.as_str())
            || !matches!(
                run.state.as_str(),
                "planned" | "active" | "blocked" | "completed" | "cancelled"
            )
            || run
                .current_step_ref
                .as_deref()
                .is_some_and(|step| !steps.contains_key(step))
            || runs
                .insert(run.run_ref.as_str(), run.procedure_ref.as_str())
                .is_some()
        {
            return Err(change_error("procedure run is invalid or duplicated"));
        }
    }
    let mut step_runs = BTreeSet::new();
    for step_run in &request.step_runs {
        let Some(procedure_ref) = runs.get(step_run.procedure_run_ref.as_str()) else {
            return Err(change_error("step run has no procedure run"));
        };
        let Some(step) = procedure_steps[procedure_ref].get(step_run.step_ref.as_str()) else {
            return Err(change_error("step run refers to an unknown procedure step"));
        };
        let mut input_roles = BTreeSet::new();
        let mut output_roles = BTreeSet::new();
        if !valid_ref(&step_run.step_run_ref)
            || !matches!(
                step_run.state.as_str(),
                "pending" | "ready" | "active" | "blocked" | "completed" | "skipped"
            )
            || step_run
                .actor_ref
                .as_deref()
                .is_some_and(|value| !valid_ref(value))
            || step_run.input_bindings.iter().any(|binding| {
                !valid_ref(&binding.value_ref)
                    || !step.input_roles.contains(&binding.data_role_ref)
                    || !input_roles.insert(binding.data_role_ref.as_str())
            })
            || step_run.output_bindings.iter().any(|binding| {
                !valid_ref(&binding.value_ref)
                    || !step.output_roles.contains(&binding.data_role_ref)
                    || !output_roles.insert(binding.data_role_ref.as_str())
            })
            || step_run.blocking_refs.iter().any(|value| !valid_ref(value))
            || matches!(step_run.state.as_str(), "active" | "blocked" | "completed")
                && step.input_roles.iter().any(|role_ref| {
                    data_roles
                        .get(role_ref.as_str())
                        .is_some_and(|role| role.required)
                        && !input_roles.contains(role_ref.as_str())
                })
            || step_run.state == "completed"
                && step.output_roles.iter().any(|role_ref| {
                    data_roles
                        .get(role_ref.as_str())
                        .is_some_and(|role| role.required)
                        && !output_roles.contains(role_ref.as_str())
                })
            || !step_runs.insert((
                step_run.procedure_run_ref.as_str(),
                step_run.step_ref.as_str(),
            ))
        {
            return Err(change_error("step run is invalid or duplicated"));
        }
    }
    for run in &request.procedure_runs {
        if let Some(current) = &run.current_step_ref
            && !step_runs.contains(&(run.run_ref.as_str(), current.as_str()))
        {
            return Err(change_error(
                "procedure current step has no corresponding step run",
            ));
        }
    }
    // Plan and time owners must already exist. Evidence and output references
    // may intentionally identify opaque external material, but never payloads.
    if request
        .plans
        .iter()
        .any(|entry| !records.contains(entry.plan_ref.as_str()))
        || request
            .timeline
            .iter()
            .any(|entry| !records.contains(entry.time_ref.as_str()))
    {
        return Err(change_error(
            "work tracking plan or time owner is not declared",
        ));
    }
    Ok(resolved_record_sets)
}

fn validate_task_registration(
    project_file: &Path,
    request: &TaskRegistrationRequest,
) -> Result<(), ToolingError> {
    if request.schema_version != SCHEMA_VERSION
        || !valid_task_path(&request.task_path)
        || !valid_one_line(&request.title, 256)
        || !valid_one_line(&request.next_instruction, 1_024)
        || request.source_start_line > request.source_end_line
        || !zixcel_interaction::valid_reference(&request.owner_ref)
        || !zixcel_interaction::valid_reference(&request.method_ref)
        || !matches!(
            request.activity_state,
            ActivityState::Planned | ActivityState::Active
        )
    {
        return Err(change_error("task registration input is invalid"));
    }
    let mut referenced = request.authority_refs.clone();
    referenced.extend(request.reference_refs.iter().cloned());
    if referenced
        .iter()
        .any(|value| !zixcel_interaction::valid_reference(value))
    {
        return Err(change_error(
            "task registration contains an invalid reference",
        ));
    }
    let analyzer = WorkspaceAnalyzer::load(project_file)?;
    let analysis = analyzer.analyze("");
    let task_ref = format!("activity/{}/task", request.task_path);
    if analysis
        .projection
        .nodes
        .iter()
        .any(|node| node.id == task_ref)
    {
        return Err(change_error("task is already registered"));
    }
    ensure_unique_activity_sources(
        analyzer
            .workspace_presentation()
            .into_iter()
            .flat_map(|presentation| &presentation.records)
            .filter_map(|record| {
                record
                    .record_id
                    .starts_with("activity/")
                    .then_some(record.source.as_ref())
                    .flatten()
                    .map(|source| (record.record_id.as_str(), source.source_ref.as_str()))
            })
            .chain(std::iter::once((
                task_ref.as_str(),
                request.source_ref.as_str(),
            ))),
    )?;
    let kind = |id: &str| {
        analysis
            .projection
            .nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.kind.as_str())
    };
    if kind(&request.owner_ref) != Some("subject") {
        return Err(change_error("task owner is not a declared subject"));
    }
    let selected_procedure = analyzer
        .procedure_package_presentation()
        .iter()
        .flat_map(|package| &package.procedures)
        .any(|procedure| procedure.procedure_ref == request.method_ref);
    if kind(&request.method_ref) != Some("method") && !selected_procedure {
        return Err(change_error("task method is not a declared method"));
    }
    if request
        .authority_refs
        .iter()
        .any(|value| kind(value) != Some("authority"))
        || request
            .reference_refs
            .iter()
            .any(|value| kind(value) != Some("reference"))
    {
        return Err(change_error("task support reference has the wrong kind"));
    }
    Ok(())
}

fn validate_history_registration(
    project_file: &Path,
    request: &HistoryRegistrationRequest,
) -> Result<(), ToolingError> {
    let exact_digest = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if request.schema_version != SCHEMA_VERSION
        || !valid_task_path(&request.history_path)
        || !valid_one_line(&request.title, 256)
        || !valid_one_line(&request.outcome, 64)
        || !exact_digest(&request.source_revision)
        || request.source_start_line > request.source_end_line
        || !zixcel_interaction::valid_reference(&request.former_activity_ref)
        || !zixcel_interaction::valid_reference(&request.owner_ref)
        || request
            .evidence_refs
            .iter()
            .any(|value| !zixcel_interaction::valid_reference(value))
    {
        return Err(change_error("history registration input is invalid"));
    }
    let analyzer = WorkspaceAnalyzer::load(project_file)?;
    let analysis = analyzer.analyze("");
    let history_ref = format!("history/{}/record", request.history_path);
    if analysis
        .projection
        .nodes
        .iter()
        .any(|node| node.id == history_ref)
    {
        return Err(change_error("history is already registered"));
    }
    let graph_subject = analysis
        .projection
        .nodes
        .iter()
        .find(|node| node.id == request.owner_ref)
        .map(|node| node.kind.as_str())
        == Some("subject");
    let package_subject = analyzer
        .semantic_module_presentation()
        .iter()
        .flat_map(|module| &module.subjects)
        .any(|subject| subject.subject_ref == request.owner_ref);
    if !graph_subject && !package_subject {
        return Err(change_error("history owner is not a declared subject"));
    }
    Ok(())
}

fn validate_history_plan_shape(plan: &HistoryRegistrationPlan) -> Result<(), ToolingError> {
    let expected_path = plan
        .history_ref
        .strip_prefix("history/")
        .and_then(|value| value.strip_suffix("/record"));
    if plan.schema_version != SCHEMA_VERSION
        || expected_path != Some(plan.request.history_path.as_str())
        || plan.operation_id != format!("workspace/history/register/{}", plan.request.history_path)
        || plan.source.source_ref != plan.request.source_ref
        || plan.source.start_line != plan.request.source_start_line
        || plan.source.end_line != plan.request.source_end_line
        || plan.plan_digest != history_plan_digest(plan)?
    {
        return Err(change_error("history registration plan is invalid"));
    }
    Ok(())
}

fn validate_plan_shape(plan: &TaskRegistrationPlan) -> Result<(), ToolingError> {
    let expected_path = plan
        .task_ref
        .strip_prefix("activity/")
        .and_then(|value| value.strip_suffix("/task"));
    if plan.schema_version != SCHEMA_VERSION
        || expected_path != Some(plan.request.task_path.as_str())
        || plan.operation_id != format!("workspace/task/register/{}", plan.request.task_path)
        || plan.source.source_ref != plan.request.source_ref
        || plan.source.start_line != plan.request.source_start_line
        || plan.source.end_line != plan.request.source_end_line
        || plan.plan_digest != plan_digest(plan)?
    {
        return Err(change_error("task registration plan is invalid"));
    }
    Ok(())
}

fn plan_digest(plan: &TaskRegistrationPlan) -> Result<String, ToolingError> {
    let mut material = plan.clone();
    material.plan_digest.clear();
    Ok(Digest::document_digest(&serde_json::to_vec(&material)?).to_hex())
}

fn history_plan_digest(plan: &HistoryRegistrationPlan) -> Result<String, ToolingError> {
    let mut material = plan.clone();
    material.plan_digest.clear();
    Ok(Digest::document_digest(&serde_json::to_vec(&material)?).to_hex())
}

fn source_plan_digest(plan: &SourceBindingPlan) -> Result<String, ToolingError> {
    let mut material = plan.clone();
    material.plan_digest.clear();
    Ok(Digest::document_digest(&serde_json::to_vec(&material)?).to_hex())
}

fn migration_plan_digest(plan: &SourceMigrationPlan) -> Result<String, ToolingError> {
    let mut material = plan.clone();
    material.plan_digest.clear();
    Ok(Digest::document_digest(&serde_json::to_vec(&material)?).to_hex())
}

fn work_tracking_digest(plan: &WorkTrackingPlan) -> Result<String, ToolingError> {
    let mut material = plan.clone();
    material.plan_digest.clear();
    Ok(Digest::document_digest(&serde_json::to_vec(&material)?).to_hex())
}

fn work_tracking_content_digest(
    request: &WorkTrackingRequest,
    source: &WorkspaceSourceLocator,
) -> Result<String, ToolingError> {
    Ok(Digest::from_serializable(1, &(request, source))
        .map_err(|error| change_error(format!("work tracking content is invalid: {error}")))?
        .to_hex())
}

fn validated_source_locator(
    base: &Path,
    manifest: &ProjectManifest,
    request: &SourceBindingRequest,
) -> Result<WorkspaceSourceLocator, ToolingError> {
    let required_role = if request.record_ref.starts_with("history/") {
        Some(ProjectSourceRole::Records)
    } else if request.record_ref.starts_with("activity/")
        || request.record_ref == "tracking/project/work/record"
    {
        Some(ProjectSourceRole::Work)
    } else {
        None
    };
    let declared_source = manifest
        .source_documents
        .iter()
        .find(|source| source.path == request.source_ref);
    if request.schema_version != SCHEMA_VERSION
        || !zixcel_interaction::valid_reference(&request.record_ref)
        || declared_source.is_none()
        || required_role.is_some_and(|role| declared_source.map(|source| source.role) != Some(role))
        || request.start_line > request.end_line
    {
        return Err(change_error("source binding input is invalid"));
    }
    let source = base.join(&request.source_ref);
    let metadata = std::fs::symlink_metadata(&source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES
    {
        return Err(change_error(
            "source binding file is not a bounded physical file",
        ));
    }
    let canonical_base = std::fs::canonicalize(base)?;
    if std::fs::canonicalize(&source)?
        .strip_prefix(canonical_base)
        .is_err()
    {
        return Err(change_error("source binding file escaped the project"));
    }
    let bytes = std::fs::read(&source)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| change_error("source binding file is not UTF-8"))?;
    let lines = text.lines().collect::<Vec<_>>();
    if request.end_line as usize >= lines.len()
        || lines[request.start_line as usize..=request.end_line as usize]
            .iter()
            .all(|line| line.trim().is_empty() || line.trim().starts_with('#'))
    {
        return Err(change_error(
            "source binding range has no authored instruction",
        ));
    }
    Ok(WorkspaceSourceLocator {
        semantic_unit_ref: request.record_ref.clone(),
        source_ref: request.source_ref.clone(),
        document_digest: Digest::document_digest(&bytes).to_hex(),
        unit_digest: Digest::document_digest(
            lines[request.start_line as usize..=request.end_line as usize]
                .join("\n")
                .as_bytes(),
        )
        .to_hex(),
        start_line: request.start_line,
        end_line: request.end_line,
    })
}

fn project_context(
    project_file: &Path,
) -> Result<(PathBuf, ProjectManifest, String), ToolingError> {
    if project_file.file_name().and_then(|value| value.to_str())
        != Some(crate::project_source::PROJECT_SOURCE_PATH)
    {
        return Err(ToolingError::Project(
            "the canonical project definition must be named project.sem".into(),
        ));
    }
    let bytes = std::fs::read(project_file)?;
    if bytes.len() > 1_048_576 {
        return Err(ToolingError::Project(
            "project.sem exceeds the bounded source size".into(),
        ));
    }
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| ToolingError::Project("project.sem must be UTF-8".into()))?;
    let manifest = crate::project_source::compile(source)?.manifest;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(ToolingError::Version("project.sem".into()));
    }
    crate::validate_project_layout(&manifest)?;
    let graph = manifest
        .workspace_graph
        .clone()
        .ok_or_else(|| change_error("project has no workspace graph"))?;
    Ok((crate::project_directory(project_file)?, manifest, graph))
}

fn valid_task_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 48
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn valid_one_line(value: &str, maximum: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= maximum
        && !trimmed.contains(['\r', '\n'])
        && trimmed == value
}

fn ensure_unique_activity_sources<'a>(
    bindings: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), ToolingError> {
    let mut activities_by_source = BTreeMap::new();
    for (activity_ref, source_ref) in bindings {
        if let Some(previous) = activities_by_source.insert(source_ref, activity_ref)
            && previous != activity_ref
        {
            return Err(change_error(
                "one work source cannot specify more than one WBS activity",
            ));
        }
    }
    Ok(())
}

fn change_error(message: impl Into<String>) -> ToolingError {
    ToolingError::Capability(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sem-lang-graph-mutation-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        crate::project::initialize(&root, "person/mutation_test", "en-US").unwrap();
        root
    }

    #[test]
    fn plan_digest_rejects_tampering() {
        let request = TaskRegistrationRequest {
            schema_version: SCHEMA_VERSION.into(),
            task_path: "development/review".into(),
            activity_state: ActivityState::Planned,
            owner_ref: "subject/person/owner/subject".into(),
            method_ref: "method/manual/review/procedure".into(),
            title: "Review".into(),
            next_instruction: "Select evidence".into(),
            source_ref: "work/development/review.sem".into(),
            source_start_line: 1,
            source_end_line: 2,
            authority_refs: Vec::new(),
            reference_refs: Vec::new(),
        };
        let mut plan = TaskRegistrationPlan {
            schema_version: SCHEMA_VERSION.into(),
            operation_id: "workspace/task/register/development/review".into(),
            task_ref: "activity/development/review/task".into(),
            expected_graph_revision: WorkspaceGraphRevision {
                sequence: 0,
                commit: None,
            },
            request,
            source: WorkspaceSourceLocator {
                semantic_unit_ref: "activity/development/review/task".into(),
                source_ref: "work/development/review.sem".into(),
                document_digest: "source/digest".into(),
                unit_digest: "unit/digest".into(),
                start_line: 1,
                end_line: 2,
            },
            plan_digest: String::new(),
        };
        plan.plan_digest = plan_digest(&plan).unwrap();
        assert!(validate_plan_shape(&plan).is_ok());
        plan.request.title = "Changed".into();
        assert!(validate_plan_shape(&plan).is_err());
    }

    #[test]
    fn one_work_source_cannot_specify_multiple_wbs_activities() {
        assert!(
            ensure_unique_activity_sources([
                ("activity/review/one/task", "work/review/one.sem"),
                ("activity/review/two/task", "work/review/two.sem"),
            ])
            .is_ok()
        );
        assert!(
            ensure_unique_activity_sources([
                ("activity/review/one/task", "work/review.sem"),
                ("activity/review/two/task", "work/review.sem"),
            ])
            .is_err()
        );
    }

    #[test]
    fn new_project_graph_is_readable() {
        let root = project("empty");
        let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
        assert!(analyzer.analyze("").projection.nodes.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_roles_separate_work_from_record_retention() {
        let root = project("source-roles");
        let project_file = root.join(crate::project::PROJECT_SOURCE_PATH);
        fs::create_dir_all(root.join("work")).unwrap();
        fs::create_dir_all(root.join("records")).unwrap();
        fs::write(root.join("work/example.sem"), "# Request\n  work\n").unwrap();
        fs::write(root.join("records/example.sem"), "# Statement\n  record\n").unwrap();
        let project_text = fs::read_to_string(&project_file).unwrap();
        let mut manifest = crate::project_source::compile(&project_text)
            .unwrap()
            .manifest;
        manifest
            .source_documents
            .push(crate::ProjectSourceDocument {
                path: "work/example.sem".into(),
                role: ProjectSourceRole::Work,
            });
        manifest
            .source_documents
            .push(crate::ProjectSourceDocument {
                path: "records/example.sem".into(),
                role: ProjectSourceRole::Records,
            });
        fs::write(
            &project_file,
            crate::project_source::render(&manifest, "").unwrap(),
        )
        .unwrap();

        let task = |source_ref: &str| SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: "activity/example/task".into(),
            source_ref: source_ref.into(),
            start_line: 1,
            end_line: 1,
        };
        let history = |source_ref: &str| SourceBindingRequest {
            schema_version: SCHEMA_VERSION.into(),
            record_ref: "history/example/record".into(),
            source_ref: source_ref.into(),
            start_line: 1,
            end_line: 1,
        };
        assert!(validated_source_locator(&root, &manifest, &task("work/example.sem")).is_ok());
        assert!(validated_source_locator(&root, &manifest, &task("records/example.sem")).is_err());
        assert!(
            validated_source_locator(&root, &manifest, &history("records/example.sem")).is_ok()
        );
        assert!(validated_source_locator(&root, &manifest, &history("work/example.sem")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn receipt_publication_is_immutable_and_idempotent() {
        let root = project("receipt");
        let project_file = root.join(crate::project::PROJECT_SOURCE_PATH);
        let digest = "a".repeat(64);
        let receipt = WorkTrackingReceipt {
            schema_version: SCHEMA_VERSION.into(),
            operation_id: "workspace/work/tracking/replace".into(),
            plan_digest: digest.clone(),
            graph_revision: 7,
        };

        persist_receipt(&project_file, &receipt).unwrap();
        persist_receipt(&project_file, &receipt).unwrap();

        let mut expected_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
        expected_bytes.push(b'\n');
        let receipt_digest = Digest::document_digest(&expected_bytes).to_hex();
        let path = root
            .join(crate::project::RUNTIME_RECEIPTS_PATH)
            .join(format!("{receipt_digest}.json"));
        let stored: WorkTrackingReceipt =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored.operation_id, receipt.operation_id);
        assert_eq!(stored.plan_digest, receipt.plan_digest);
        assert_eq!(stored.graph_revision, receipt.graph_revision);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let later_observation = WorkTrackingReceipt {
            graph_revision: 8,
            ..receipt
        };
        persist_receipt(&project_file, &later_observation).unwrap();
        let receipts = fs::read_dir(root.join(crate::project::RUNTIME_RECEIPTS_PATH))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(receipts.len(), 2);
        let original: WorkTrackingReceipt =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(original.graph_revision, 7);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn receipt_publication_rejects_noncanonical_digest_names() {
        let root = project("receipt-invalid-digest");
        let receipt = WorkTrackingReceipt {
            schema_version: SCHEMA_VERSION.into(),
            operation_id: "workspace/work/tracking/replace".into(),
            plan_digest: "A".repeat(64),
            graph_revision: 1,
        };
        assert!(persist_receipt(&root.join("project.sem"), &receipt).is_err());
        assert!(
            fs::read_dir(root.join(crate::project::RUNTIME_RECEIPTS_PATH))
                .unwrap()
                .next()
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_receipt_publication_converges_on_one_file() {
        use std::sync::{Arc, Barrier};

        let root = project("receipt-concurrent");
        let project_file = root.join(crate::project::PROJECT_SOURCE_PATH);
        let receipt = WorkTrackingReceipt {
            schema_version: SCHEMA_VERSION.into(),
            operation_id: "workspace/work/tracking/replace".into(),
            plan_digest: "b".repeat(64),
            graph_revision: 9,
        };
        let barrier = Arc::new(Barrier::new(8));
        let workers = (0..8)
            .map(|_| {
                let project_file = project_file.clone();
                let receipt = receipt.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    persist_receipt(&project_file, &receipt)
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        let entries = fs::read_dir(root.join(crate::project::RUNTIME_RECEIPTS_PATH))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let stored: WorkTrackingReceipt =
            serde_json::from_slice(&fs::read(entries[0].path()).unwrap()).unwrap();
        assert_eq!(stored, receipt);
        fs::remove_dir_all(root).unwrap();
    }
}
