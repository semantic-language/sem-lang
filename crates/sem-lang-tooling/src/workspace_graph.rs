//! sem-lang project-state mapping over the generic Zixcel graph database.
//!
//! Zixcel owns transactions, revisions, and durable storage.  This module owns
//! the sem-lang domain mapping.  Neither the editor nor project code reads the
//! database schema directly.

use crate::workspace::{
    WorkspaceActivityPresentation, WorkspaceHistoryPresentation, WorkspacePresentation,
    WorkspaceProjection, WorkspaceRecordPresentation, WorkspaceSourceLocator,
};
use crate::workspace_change::{
    HistoryRegistrationReceipt, SourceBindingReceipt, SourceMigrationReceipt,
    TaskRegistrationReceipt, WorkTrackingReceipt, WorkspaceGraphRevision,
};

/// Append one exact, source-bound work outcome. Reapplying the same plan is
/// idempotent; reusing its identity for different content is rejected.
pub fn register_history(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
    expected_revision: &WorkspaceGraphRevision,
    plan_digest: &str,
    request: &crate::workspace_change::HistoryRegistrationRequest,
    source_locator: &WorkspaceSourceLocator,
) -> Result<HistoryRegistrationReceipt, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let history_ref = format!("history/{}/record", request.history_path);
    let receipt = HistoryRegistrationReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: format!("workspace/history/register/{}", request.history_path),
        history_ref: history_ref.clone(),
        plan_digest: plan_digest.into(),
        graph_revision: next_sequence(expected_revision)?,
    };
    publish_exact(
        &graph,
        &space,
        expected_revision,
        ("workspace/history/register", plan_digest),
        &(request, source_locator),
        &receipt,
        |read| {
            if let Some(existing) = read.get_node(&history_ref) {
                return match string_property(&existing, "registration_digest") {
                    Some(value) if value == plan_digest => Ok(GraphChanges::default()),
                    _ => Err(ToolingError::Graph(
                        "history identity already exists with different content".into(),
                    )),
                };
            }
            let semantic_revision = Digest::document_digest(plan_digest.as_bytes()).to_hex();
            let history = WorkspaceHistoryPresentation {
                history_ref: history_ref.clone(),
                former_activity_ref: request.former_activity_ref.clone(),
                outcome: request.outcome.clone(),
                occurred_at: request.occurred_at.clone(),
                source_revision: request.source_revision.clone(),
                evidence_refs: request.evidence_refs.clone(),
            };
            let mut node = projection_node(&ProjectionNode {
                id: history_ref.clone(),
                kind: "history".into(),
                label: request.title.clone(),
                state: request.outcome.clone(),
                owner: request.owner_ref.clone(),
                revision: semantic_revision,
            })?;
            node = node
                .with_property(
                    "registration_digest",
                    PropertyValue::String(plan_digest.into()),
                )
                .map_err(graph_error)?;
            node = json_property(node, HISTORY, &history)?;
            node = json_property(node, SOURCE_LOCATOR, source_locator)?;
            let source_id = source_node_id(source_locator)?;
            let mut source = projection_node(&ProjectionNode {
                id: source_id.clone(),
                kind: "source".into(),
                label: format!(
                    "{}:{}-{}",
                    source_locator.source_ref,
                    source_locator.start_line + 1,
                    source_locator.end_line + 1
                ),
                state: "authored".into(),
                owner: project_id.into(),
                revision: source_locator.unit_digest.clone(),
            })?;
            source = json_property(source, SOURCE_LOCATOR, source_locator)?;
            let relations = [
                ProjectionEdge {
                    source: history_ref.clone(),
                    relation: "formerly".into(),
                    target: request.former_activity_ref.clone(),
                },
                ProjectionEdge {
                    source: history_ref.clone(),
                    relation: "owned-by".into(),
                    target: request.owner_ref.clone(),
                },
                ProjectionEdge {
                    source: history_ref.clone(),
                    relation: "specified-by".into(),
                    target: source_id.clone(),
                },
            ];
            let created = BTreeSet::from([history_ref.clone(), source_id]);
            let mut changes = GraphChanges {
                nodes: vec![node, source],
                ..GraphChanges::default()
            };
            for relation in relations {
                if read.get_node(&relation.target).is_none() && !created.contains(&relation.target)
                {
                    changes
                        .nodes
                        .push(Node::new(&relation.target, [EXTERNAL_LABEL]).map_err(graph_error)?);
                }
                changes.edges.push(projection_edge(&relation)?);
            }
            Ok(changes)
        },
    )
}
use crate::{
    Diagnostic, ProjectCompilation, ProjectCompilationReceipt, ProjectCompilationRetirementReceipt,
    ProjectionEdge, ProjectionNode, SCHEMA_VERSION, ToolingError, project_compilation_digest,
};
use sem_lang_core::Digest;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use zixcel_graph::{
    Edge, Graph, GraphChanges, GraphPublication, GraphSpace, Node, PropertyValue,
    PublicationIntent, ReadTransaction,
};
use zixcel_revision::{CommitOutcome, RevisionRef};

const RECORD_LABEL: &str = "sem_lang_record";
const EXTERNAL_LABEL: &str = "sem_lang_external_reference";
const RELATION_LABEL: &str = "sem_lang_relation";
const MAX_RECORDS: usize = 4_096;
const MAX_RELATIONS: usize = 16_384;

const KIND: &str = "semantic_kind";
const LABEL: &str = "display_label";
const STATE: &str = "lifecycle_state";
const OWNER: &str = "owner_ref";
const SEMANTIC_REVISION: &str = "semantic_revision";
const RECORD: &str = "record";
const ACTIVITY: &str = "activity_presentation";
const HISTORY: &str = "history_presentation";
const CONNECTION: &str = "connection_presentation";
const PLANS: &str = "plan_presentations";
const TIMELINE: &str = "timeline_presentations";
const DEPENDENCIES: &str = "dependency_presentations";
const MILESTONES: &str = "milestone_presentations";
const RECORD_SETS: &str = "record_set_presentations";
const OUTPUTS: &str = "output_presentations";
const PROCEDURE_RUNS: &str = "procedure_run_presentations";
const STEP_RUNS: &str = "step_run_presentations";
const TRACKING_DIGEST: &str = "tracking_digest";
const SOURCE_LOCATOR: &str = "source_locator";
const RELATION: &str = "semantic_relation";
const PROJECT_COMPILATION_LABEL: &str = "sem_lang_project_compilation";
const PROJECT_COMPILATION_CURRENT: &str = "project/compilation/current";
const PROJECT_COMPILATION_ARTIFACT: &str = "project_compilation_artifact";
const PROJECT_COMPILATION_RECEIPT: &str = "project_compilation_receipt";

/// Create the durable database for a new project. Creation is explicit and
/// never used as a fallback by the read path.
pub fn create(base: &Path, graph_ref: &str) -> Result<(), ToolingError> {
    let path = checked_new_path(base, graph_ref)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Graph::create(&path).map_err(graph_error)?;
    Ok(())
}

/// Return the current graph revision for immutable mutation planning.
pub fn revision(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
) -> Result<WorkspaceGraphRevision, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let revision = graph.commit_revision(&space).map_err(graph_error)?;
    Ok(WorkspaceGraphRevision {
        sequence: revision.sequence,
        commit: revision.commit,
    })
}

/// Load one immutable project snapshot from the graph.
pub fn load(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
) -> Result<WorkspaceProjection, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let read = graph.read(&space).map_err(graph_error)?;
    decode(&read, graph_ref)
}

/// Read the exact currently adopted project compilation without compiling or
/// modifying source. The artifact is rejected if its content-derived identity
/// or receipt binding is inconsistent.
pub fn load_project_compilation(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
) -> Result<Option<(ProjectCompilation, ProjectCompilationReceipt)>, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let read = graph.read(&space).map_err(graph_error)?;
    decode_project_compilation(&read)
}

/// Explicitly retire only rebuildable project-compilation artifacts before a
/// source-contract migration. Domain records and their relations are not
/// selected by this operation and therefore cannot be removed by it.
pub fn retire_project_compilation(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
) -> Result<ProjectCompilationRetirementReceipt, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let revision = graph.commit_revision(&space).map_err(graph_error)?;
    let expected = WorkspaceGraphRevision {
        sequence: revision.sequence,
        commit: revision.commit,
    };
    let read = graph.read(&space).map_err(graph_error)?;
    let artifacts = read.nodes_by_label(PROJECT_COMPILATION_LABEL, 4_096);
    if artifacts.is_empty() {
        return Ok(ProjectCompilationRetirementReceipt {
            schema_version: SCHEMA_VERSION.into(),
            operation_id: "project/compilation/retire/semantic-package-set-v1".into(),
            project_id: project_id.into(),
            removed_artifacts: 0,
            graph_revision: expected.sequence,
        });
    }
    let plan_digest = Digest::from_serializable(
        1,
        &(
            project_id,
            "semantic-package-set-v1",
            &artifacts.iter().map(|node| &node.id).collect::<Vec<_>>(),
        ),
    )
    .map_err(graph_error)?
    .to_hex();
    let receipt = ProjectCompilationRetirementReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: format!("project/compilation/retire/{plan_digest}"),
        project_id: project_id.into(),
        removed_artifacts: artifacts.len(),
        graph_revision: next_sequence(&expected)?,
    };
    publish_exact(
        &graph,
        &space,
        &expected,
        ("project/compilation/retire", &plan_digest),
        &(project_id, "semantic-package-set-v1"),
        &receipt,
        |read| {
            let remove_nodes = read
                .nodes_by_label(PROJECT_COMPILATION_LABEL, 4_096)
                .into_iter()
                .map(|node| node.id)
                .collect();
            Ok(GraphChanges {
                remove_nodes,
                ..GraphChanges::default()
            })
        },
    )
}

/// Publish one successful deterministic project compilation. The authored
/// source remains `project.sem`; the graph stores an immutable derived
/// artifact plus one current pointer. Exact retries perform no write.
pub fn publish_project_compilation(
    base: &Path,
    graph_ref: &str,
    compilation: &ProjectCompilation,
) -> Result<ProjectCompilationReceipt, ToolingError> {
    if compilation.schema_version != SCHEMA_VERSION
        || compilation.project_id.is_empty()
        || compilation.artifact_digest != project_compilation_digest(compilation)?
    {
        return Err(ToolingError::Project(
            "project compilation artifact is invalid".into(),
        ));
    }
    let (graph, space) = open(base, graph_ref, &compilation.project_id)?;
    {
        let read = graph.read(&space).map_err(graph_error)?;
        if let Some((current, receipt)) = decode_project_compilation(&read)?
            && current.artifact_digest == compilation.artifact_digest
        {
            if current != *compilation {
                return Err(ToolingError::Graph(
                    "project compilation digest aliases different content".into(),
                ));
            }
            return Ok(receipt);
        }
    }
    let revision = graph.commit_revision(&space).map_err(graph_error)?;
    let expected = WorkspaceGraphRevision {
        sequence: revision.sequence,
        commit: revision.commit,
    };
    let receipt = ProjectCompilationReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: format!("project/compile/{}", compilation.artifact_digest),
        project_id: compilation.project_id.clone(),
        source_digest: compilation.source_digest.clone(),
        artifact_digest: compilation.artifact_digest.clone(),
        graph_revision: next_sequence(&expected)?,
    };
    let artifact_json = serde_json::to_string(compilation)?;
    let receipt_json = serde_json::to_string(&receipt)?;
    publish_exact(
        &graph,
        &space,
        &expected,
        ("project/compile", &compilation.artifact_digest),
        compilation,
        &receipt,
        |read| {
            if let Some((current, _)) = decode_project_compilation(read)?
                && current.artifact_digest == compilation.artifact_digest
            {
                return if current == *compilation {
                    Ok(GraphChanges::default())
                } else {
                    Err(ToolingError::Graph(
                        "project compilation digest aliases different content".into(),
                    ))
                };
            }
            let immutable_id = format!(
                "project/compilation/revision/{}",
                compilation.artifact_digest
            );
            if let Some(existing) = read.get_node(&immutable_id) {
                let existing_artifact = required_string(&existing, PROJECT_COMPILATION_ARTIFACT)?;
                if existing_artifact != artifact_json {
                    return Err(ToolingError::Graph(
                        "immutable project compilation was reused with different content".into(),
                    ));
                }
            }
            let immutable = Node::new(&immutable_id, [PROJECT_COMPILATION_LABEL])
                .and_then(|node| {
                    node.with_property(
                        PROJECT_COMPILATION_ARTIFACT,
                        PropertyValue::String(artifact_json.clone()),
                    )
                })
                .map_err(graph_error)?;
            let current = Node::new(PROJECT_COMPILATION_CURRENT, [PROJECT_COMPILATION_LABEL])
                .and_then(|node| {
                    node.with_property(
                        PROJECT_COMPILATION_ARTIFACT,
                        PropertyValue::String(artifact_json.clone()),
                    )
                })
                .and_then(|node| {
                    node.with_property(
                        PROJECT_COMPILATION_RECEIPT,
                        PropertyValue::String(receipt_json.clone()),
                    )
                })
                .map_err(graph_error)?;
            Ok(GraphChanges {
                nodes: vec![immutable, current],
                ..GraphChanges::default()
            })
        },
    )
}

fn decode_project_compilation(
    read: &ReadTransaction,
) -> Result<Option<(ProjectCompilation, ProjectCompilationReceipt)>, ToolingError> {
    let Some(node) = read.get_node(PROJECT_COMPILATION_CURRENT) else {
        return Ok(None);
    };
    if !node.labels.contains(PROJECT_COMPILATION_LABEL) {
        return Err(ToolingError::Graph(
            "current project compilation has an invalid graph label".into(),
        ));
    }
    let compilation: ProjectCompilation =
        serde_json::from_str(&required_string(&node, PROJECT_COMPILATION_ARTIFACT)?)?;
    let receipt: ProjectCompilationReceipt =
        serde_json::from_str(&required_string(&node, PROJECT_COMPILATION_RECEIPT)?)?;
    if compilation.schema_version != SCHEMA_VERSION
        || compilation.artifact_digest != project_compilation_digest(&compilation)?
        || receipt.schema_version != SCHEMA_VERSION
        || receipt.project_id != compilation.project_id
        || receipt.source_digest != compilation.source_digest
        || receipt.artifact_digest != compilation.artifact_digest
    {
        return Err(ToolingError::Graph(
            "stored project compilation is inconsistent".into(),
        ));
    }
    Ok(Some((compilation, receipt)))
}

/// Register an actionable task and its two human-readable references in one
/// graph transaction. Repeating the exact plan converges without duplication.
pub fn register_task(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
    expected_revision: &WorkspaceGraphRevision,
    plan_digest: &str,
    task: &crate::workspace_change::TaskRegistrationRequest,
    source_locator: &WorkspaceSourceLocator,
) -> Result<TaskRegistrationReceipt, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let task_ref = format!("activity/{}/task", task.task_path);
    let receipt = TaskRegistrationReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: format!("workspace/task/register/{}", task.task_path),
        task_ref: task_ref.clone(),
        plan_digest: plan_digest.into(),
        graph_revision: next_sequence(expected_revision)?,
    };
    publish_exact(
        &graph,
        &space,
        expected_revision,
        ("workspace/task/register", plan_digest),
        &(task, source_locator),
        &receipt,
        |read| {
            if let Some(existing) = read.get_node(&task_ref) {
                return match string_property(&existing, "registration_digest") {
                    Some(value) if value == plan_digest => Ok(GraphChanges::default()),
                    _ => Err(ToolingError::Graph(
                        "task identity already exists with different content".into(),
                    )),
                };
            }
            let display_ref = format!("reference/activity/{}/title/reference", task.task_path);
            let next_ref = format!("reference/activity/{}/next/reference", task.task_path);
            let semantic_revision = Digest::document_digest(plan_digest.as_bytes()).to_hex();
            let reference_node = |id: &str, text: &str| -> Result<Node, ToolingError> {
                let value = serde_json::json!({"text": text});
                projection_node(&ProjectionNode {
                    id: id.into(),
                    kind: "reference".into(),
                    label: text.into(),
                    state: "declared".into(),
                    owner: task.owner_ref.clone(),
                    revision: semantic_revision.clone(),
                })?
                .with_property(RECORD, PropertyValue::String(value.to_string()))
                .map_err(graph_error)
            };
            let activity = WorkspaceActivityPresentation {
                activity_ref: task_ref.clone(),
                next_ref: next_ref.clone(),
                method_ref: task.method_ref.clone(),
                authority_refs: task.authority_refs.clone(),
                completion_condition: "owner_confirmed".into(),
                history_required: true,
            };
            let record = serde_json::json!({
                "schema_version": SCHEMA_VERSION,
                "activity_state": task.activity_state,
                "owner_ref": task.owner_ref,
                "display_ref": display_ref,
                "next_ref": next_ref,
                "method_ref": task.method_ref,
                "authority_refs": task.authority_refs,
                "reference_refs": task.reference_refs,
                "completion": {"condition": "owner_confirmed", "history_required": true}
            });
            let mut task_node = projection_node(&ProjectionNode {
                id: task_ref.clone(),
                kind: "activity".into(),
                label: task.title.clone(),
                state: serde_json::to_value(task.activity_state)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "planned".into()),
                owner: task.owner_ref.clone(),
                revision: semantic_revision.clone(),
            })?;
            task_node = task_node
                .with_property(RECORD, PropertyValue::String(record.to_string()))
                .and_then(|node| {
                    node.with_property(
                        "registration_digest",
                        PropertyValue::String(plan_digest.into()),
                    )
                })
                .map_err(graph_error)?;
            task_node = json_property(task_node, ACTIVITY, &activity)?;
            task_node = json_property(task_node, SOURCE_LOCATOR, source_locator)?;

            let source_id = source_node_id(source_locator)?;
            let mut source = projection_node(&ProjectionNode {
                id: source_id.clone(),
                kind: "source".into(),
                label: format!(
                    "{}:{}-{}",
                    source_locator.source_ref,
                    source_locator.start_line + 1,
                    source_locator.end_line + 1
                ),
                state: "authored".into(),
                owner: project_id.into(),
                revision: source_locator.unit_digest.clone(),
            })?;
            source = json_property(source, SOURCE_LOCATOR, source_locator)?;
            let mut display = reference_node(&display_ref, &task.title)?;
            display = json_property(display, SOURCE_LOCATOR, source_locator)?;
            let mut next = reference_node(&next_ref, &task.next_instruction)?;
            next = json_property(next, SOURCE_LOCATOR, source_locator)?;
            let mut changes = GraphChanges {
                nodes: vec![display, next, task_node, source],
                ..GraphChanges::default()
            };
            let mut relations = vec![
                ProjectionEdge {
                    source: task_ref.clone(),
                    relation: "displayed-by".into(),
                    target: display_ref.clone(),
                },
                ProjectionEdge {
                    source: task_ref.clone(),
                    relation: "next".into(),
                    target: next_ref.clone(),
                },
                ProjectionEdge {
                    source: task_ref.clone(),
                    relation: "owned-by".into(),
                    target: task.owner_ref.clone(),
                },
                ProjectionEdge {
                    source: task_ref.clone(),
                    relation: "uses".into(),
                    target: task.method_ref.clone(),
                },
            ];
            relations.extend(
                task.authority_refs
                    .iter()
                    .cloned()
                    .map(|target| ProjectionEdge {
                        source: task_ref.clone(),
                        relation: "authorized-by".into(),
                        target,
                    }),
            );
            relations.extend(
                task.reference_refs
                    .iter()
                    .cloned()
                    .map(|target| ProjectionEdge {
                        source: task_ref.clone(),
                        relation: "supported-by".into(),
                        target,
                    }),
            );
            relations.extend(
                [task_ref.as_str(), display_ref.as_str(), next_ref.as_str()]
                    .into_iter()
                    .map(|record| ProjectionEdge {
                        source: record.into(),
                        relation: "specified-by".into(),
                        target: source_id.clone(),
                    }),
            );
            let created_records = BTreeSet::from([
                task_ref.clone(),
                display_ref.clone(),
                next_ref.clone(),
                source_id,
            ]);
            for relation in &relations {
                if read.get_node(&relation.target).is_none()
                    && !created_records.contains(&relation.target)
                {
                    changes
                        .nodes
                        .push(Node::new(&relation.target, [EXTERNAL_LABEL]).map_err(graph_error)?);
                }
                changes.edges.push(projection_edge(relation)?);
            }
            Ok(changes)
        },
    )
}

/// Attach one exact authored-source locator to an existing record. The source
/// bytes stay in the `.sem` file; only its digest and bounded line range enter
/// the graph. Repeating the exact binding converges without another revision.
pub fn bind_source(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
    expected_revision: &WorkspaceGraphRevision,
    plan_digest: &str,
    record_ref: &str,
    locator: &WorkspaceSourceLocator,
) -> Result<SourceBindingReceipt, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let receipt = SourceBindingReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: format!("workspace/source/bind/{record_ref}"),
        record_ref: record_ref.into(),
        plan_digest: plan_digest.into(),
        graph_revision: next_sequence(expected_revision)?,
    };
    publish_exact(
        &graph,
        &space,
        expected_revision,
        ("workspace/source/bind", plan_digest),
        &(record_ref, locator),
        &receipt,
        |read| {
            let mut record = read
                .get_node(record_ref)
                .filter(|node| node.labels.contains(RECORD_LABEL))
                .ok_or_else(|| {
                    ToolingError::Graph("source binding record does not exist".into())
                })?;
            if let Some(existing) = string_property(&record, SOURCE_LOCATOR) {
                let existing: WorkspaceSourceLocator = serde_json::from_str(existing)?;
                return if existing == *locator {
                    Ok(GraphChanges::default())
                } else {
                    Err(ToolingError::Graph(
                        "record is already bound to a different source locator".into(),
                    ))
                };
            }
            let source_id = source_node_id(locator)?;
            let source_label = format!(
                "{}:{}-{}",
                locator.source_ref,
                locator.start_line + 1,
                locator.end_line + 1
            );
            let mut source = projection_node(&ProjectionNode {
                id: source_id.clone(),
                kind: "source".into(),
                label: source_label,
                state: "authored".into(),
                owner: project_id.into(),
                revision: locator.unit_digest.clone(),
            })?;
            source = json_property(source, SOURCE_LOCATOR, locator)?;
            record = json_property(record, SOURCE_LOCATOR, locator)?;
            let relation = ProjectionEdge {
                source: record_ref.into(),
                relation: "specified-by".into(),
                target: source_id,
            };
            Ok(GraphChanges {
                nodes: vec![record, source],
                edges: vec![projection_edge(&relation)?],
                ..GraphChanges::default()
            })
        },
    )
}

/// Replace a complete set of source locators in one graph transaction and
/// reclaim generated source nodes that no longer have incoming relations.
/// An exact retry converges on the committed replacement set.
pub fn migrate_sources(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
    expected_revision: &WorkspaceGraphRevision,
    plan_digest: &str,
    entries: &[crate::workspace_change::SourceMigrationEntry],
) -> Result<SourceMigrationReceipt, ToolingError> {
    let (graph, space) = open(base, graph_ref, project_id)?;
    let receipt = SourceMigrationReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: "workspace/source/migrate".into(),
        migrated_records: entries.len(),
        plan_digest: plan_digest.into(),
        graph_revision: next_sequence(expected_revision)?,
    };
    publish_exact(
        &graph,
        &space,
        expected_revision,
        ("workspace/source/migrate", plan_digest),
        &entries,
        &receipt,
        |read| {
            let states = entries
                .iter()
                .map(|entry| {
                    let record = read
                        .get_node(&entry.record_ref)
                        .filter(|node| node.labels.contains(RECORD_LABEL))
                        .ok_or_else(|| {
                            ToolingError::Graph("source migration record does not exist".into())
                        })?;
                    let current = string_property(&record, SOURCE_LOCATOR)
                        .map(serde_json::from_str::<WorkspaceSourceLocator>)
                        .transpose()?
                        .ok_or_else(|| {
                            ToolingError::Graph("source migration record is not bound".into())
                        })?;
                    Ok((record, current))
                })
                .collect::<Result<Vec<_>, ToolingError>>()?;
            if states
                .iter()
                .zip(entries)
                .all(|((_, current), entry)| current == &entry.replacement)
            {
                let stale_edges = read
                    .edges_by_label(RELATION_LABEL, MAX_RELATIONS)
                    .into_iter()
                    .filter(|edge| {
                        edge.properties
                            .get(RELATION)
                            .and_then(property_string)
                            .as_deref()
                            == Some("specified-by")
                    })
                    .filter_map(|edge| {
                        let record = read.get_node(&edge.from)?;
                        let locator =
                            string_property(&record, SOURCE_LOCATOR).and_then(|value| {
                                serde_json::from_str::<WorkspaceSourceLocator>(value).ok()
                            })?;
                        (source_node_id(&locator).ok()? != edge.to).then_some(edge.id)
                    })
                    .collect::<BTreeSet<_>>();
                let orphans = read
                    .nodes_by_label("sem_lang_kind_source", MAX_RECORDS)
                    .into_iter()
                    .filter(|source| {
                        read.incoming_edges(&source.id, Some(RELATION_LABEL))
                            .into_iter()
                            .all(|edge| stale_edges.contains(&edge.id))
                    })
                    .map(|source| source.id)
                    .collect::<Vec<_>>();
                return Ok(GraphChanges {
                    remove_nodes: orphans,
                    remove_edges: stale_edges.into_iter().collect(),
                    ..GraphChanges::default()
                });
            }
            if states
                .iter()
                .zip(entries)
                .any(|((_, current), entry)| current != &entry.previous)
            {
                return Err(ToolingError::Graph(
                    "source migration previous locator is stale".into(),
                ));
            }

            let mut changes = GraphChanges::default();
            let mut old_sources = BTreeSet::new();
            let mut new_sources = BTreeSet::new();
            let mut removed_edges = BTreeSet::new();
            for ((mut record, current), entry) in states.into_iter().zip(entries) {
                // A complete migration request may include records that already
                // point at their desired source definition. They are part of the
                // verified request set but must not be removed and re-added in
                // the same graph change set.
                if current == entry.replacement {
                    continue;
                }
                let old_source_id = source_node_id(&entry.previous)?;
                let new_source_id = source_node_id(&entry.replacement)?;
                let create_source = new_sources.insert(new_source_id.clone());
                // Moving unchanged authored bytes changes the physical locator
                // but intentionally preserves the semantic source identity.
                // Updating and removing that same node in one transaction is
                // invalid; retain its edge and atomically replace only the
                // record/source locator projections.
                if old_source_id == new_source_id {
                    record = json_property(record, SOURCE_LOCATOR, &entry.replacement)?;
                    changes.nodes.push(record);
                    if create_source {
                        let mut source = projection_node(&ProjectionNode {
                            id: new_source_id,
                            kind: "source".into(),
                            label: format!(
                                "{}:{}-{}",
                                entry.replacement.source_ref,
                                entry.replacement.start_line + 1,
                                entry.replacement.end_line + 1
                            ),
                            state: "authored".into(),
                            owner: project_id.into(),
                            revision: entry.replacement.unit_digest.clone(),
                        })?;
                        source = json_property(source, SOURCE_LOCATOR, &entry.replacement)?;
                        changes.nodes.push(source);
                    }
                    continue;
                }
                let old_relation = ProjectionEdge {
                    source: entry.record_ref.clone(),
                    relation: "specified-by".into(),
                    target: old_source_id.clone(),
                };
                let old_edge = projection_edge(&old_relation)?;
                removed_edges.insert(old_edge.id.clone());
                changes.remove_edges.push(old_edge.id);
                old_sources.insert(old_source_id);

                record = json_property(record, SOURCE_LOCATOR, &entry.replacement)?;
                changes.nodes.push(record);
                if create_source {
                    let mut source = projection_node(&ProjectionNode {
                        id: new_source_id.clone(),
                        kind: "source".into(),
                        label: format!(
                            "{}:{}-{}",
                            entry.replacement.source_ref,
                            entry.replacement.start_line + 1,
                            entry.replacement.end_line + 1
                        ),
                        state: "authored".into(),
                        owner: project_id.into(),
                        revision: entry.replacement.unit_digest.clone(),
                    })?;
                    source = json_property(source, SOURCE_LOCATOR, &entry.replacement)?;
                    changes.nodes.push(source);
                }
                changes.edges.push(projection_edge(&ProjectionEdge {
                    source: entry.record_ref.clone(),
                    relation: "specified-by".into(),
                    target: new_source_id,
                })?);
            }
            for source_id in old_sources {
                let retained = read
                    .incoming_edges(&source_id, Some(RELATION_LABEL))
                    .into_iter()
                    .any(|edge| !removed_edges.contains(&edge.id));
                if !retained {
                    changes.remove_nodes.push(source_id);
                }
            }
            // A previous interrupted migration or an older writer may have left a
            // generated source node without a `specified-by` relation. It has no
            // semantic owner of its own and must not keep an obsolete source digest
            // alive. Reclaim it in the same owner transaction while protecting every
            // source created by this plan.
            for source in read.nodes_by_label("sem_lang_kind_source", MAX_RECORDS) {
                if new_sources.contains(&source.id) {
                    continue;
                }
                let retained = read
                    .incoming_edges(&source.id, Some(RELATION_LABEL))
                    .into_iter()
                    .any(|edge| !removed_edges.contains(&edge.id));
                if !retained && !changes.remove_nodes.contains(&source.id) {
                    changes.remove_nodes.push(source.id);
                }
            }
            Ok(changes)
        },
    )
}

/// Replace the complete typed work-tracking projection in one transaction.
/// The operation owns planning, scheduling, dependencies, milestones,
/// evidence collections, and output state; it never copies evidence bodies.
/// Repeating an exact plan converges on the committed revision.
pub(crate) struct WorkTrackingReplacement<'a> {
    pub project_id: &'a str,
    pub expected_revision: &'a WorkspaceGraphRevision,
    pub plan_digest: &'a str,
    pub content_digest: &'a str,
    pub request: &'a crate::workspace_change::WorkTrackingRequest,
    pub resolved_record_sets: &'a [crate::workspace::WorkspaceRecordSetPresentation],
    pub source_locator: &'a WorkspaceSourceLocator,
}

pub(crate) fn replace_work_tracking(
    base: &Path,
    graph_ref: &str,
    replacement: WorkTrackingReplacement<'_>,
) -> Result<WorkTrackingReceipt, ToolingError> {
    let WorkTrackingReplacement {
        project_id,
        expected_revision,
        plan_digest,
        content_digest,
        request,
        resolved_record_sets,
        source_locator,
    } = replacement;
    const TRACKING_REF: &str = "tracking/project/work/record";
    let (graph, space) = open(base, graph_ref, project_id)?;
    let receipt = WorkTrackingReceipt {
        schema_version: SCHEMA_VERSION.into(),
        operation_id: "workspace/work/tracking/replace".into(),
        plan_digest: plan_digest.into(),
        graph_revision: next_sequence(expected_revision)?,
    };
    publish_exact(
        &graph,
        &space,
        expected_revision,
        ("workspace/work/tracking/replace", plan_digest),
        &(content_digest, request, source_locator),
        &receipt,
        |read| {
            if let Some(existing) = read.get_node(TRACKING_REF)
                && string_property(&existing, TRACKING_DIGEST) == Some(content_digest)
            {
                return Ok(GraphChanges::default());
            }
            let semantic_revision = Digest::document_digest(plan_digest.as_bytes()).to_hex();
            let mut tracking = projection_node(&ProjectionNode {
                id: TRACKING_REF.into(),
                kind: "tracking".into(),
                label: "Project work tracking".into(),
                state: "declared".into(),
                owner: project_id.into(),
                revision: semantic_revision.clone(),
            })?;
            tracking = tracking
                .with_property(
                    TRACKING_DIGEST,
                    PropertyValue::String(content_digest.into()),
                )
                .map_err(graph_error)?;
            tracking = json_property(tracking, PLANS, &request.plans)?;
            tracking = json_property(tracking, TIMELINE, &request.timeline)?;
            tracking = json_property(tracking, DEPENDENCIES, &request.dependencies)?;
            tracking = json_property(tracking, MILESTONES, &request.milestones)?;
            tracking = json_property(tracking, RECORD_SETS, resolved_record_sets)?;
            tracking = json_property(tracking, OUTPUTS, &request.outputs)?;
            tracking = json_property(tracking, PROCEDURE_RUNS, &request.procedure_runs)?;
            tracking = json_property(tracking, STEP_RUNS, &request.step_runs)?;
            tracking = json_property(tracking, SOURCE_LOCATOR, source_locator)?;

            let source_id = source_node_id(source_locator)?;
            let mut source = projection_node(&ProjectionNode {
                id: source_id.clone(),
                kind: "source".into(),
                label: format!(
                    "{}:{}-{}",
                    source_locator.source_ref,
                    source_locator.start_line + 1,
                    source_locator.end_line + 1
                ),
                state: "authored".into(),
                owner: project_id.into(),
                revision: source_locator.unit_digest.clone(),
            })?;
            source = json_property(source, SOURCE_LOCATOR, source_locator)?;
            let mut changes = GraphChanges {
                nodes: vec![tracking, source],
                ..GraphChanges::default()
            };
            // Retire the former distributed presentation ownership. The tracking
            // record is the only writer of these complete sets after this commit.
            for mut node in read.nodes_by_label(RECORD_LABEL, MAX_RECORDS) {
                let mut changed = false;
                for key in [
                    PLANS,
                    TIMELINE,
                    DEPENDENCIES,
                    MILESTONES,
                    RECORD_SETS,
                    OUTPUTS,
                    PROCEDURE_RUNS,
                    STEP_RUNS,
                ] {
                    changed |= node.properties.remove(key).is_some();
                }
                if changed && node.id != TRACKING_REF {
                    changes.nodes.push(node);
                }
            }

            let managed_relations = BTreeSet::from([
                "contains-task",
                "scheduled-by",
                "schedules",
                "depends-on",
                "observes",
                "selects-records",
                "produces",
                "runs-procedure",
                "applies-to",
                "has-step-run",
                "instantiates-step",
                "performed-by",
                "uses-input",
                "produces-output",
                "blocked-by",
            ]);
            for edge in read.edges_by_label(RELATION_LABEL, MAX_RELATIONS) {
                if string_property_from_edge(&edge, RELATION)
                    .is_some_and(|relation| managed_relations.contains(relation))
                {
                    changes.remove_edges.push(edge.id);
                }
            }

            let mut relations = vec![ProjectionEdge {
                source: TRACKING_REF.into(),
                relation: "specified-by".into(),
                target: source_id,
            }];
            for entry in &request.plans {
                relations.push(ProjectionEdge {
                    source: entry.plan_ref.clone(),
                    relation: "contains-task".into(),
                    target: entry.task_ref.clone(),
                });
            }
            for entry in &request.timeline {
                relations.push(ProjectionEdge {
                    source: entry.time_ref.clone(),
                    relation: "schedules".into(),
                    target: entry.activity_ref.clone(),
                });
                relations.push(ProjectionEdge {
                    source: entry.activity_ref.clone(),
                    relation: "scheduled-by".into(),
                    target: entry.time_ref.clone(),
                });
            }
            for entry in &request.dependencies {
                relations.push(ProjectionEdge {
                    source: entry.dependent_ref.clone(),
                    relation: "depends-on".into(),
                    target: entry.prerequisite_ref.clone(),
                });
            }
            for entry in &request.milestones {
                changes.nodes.push(tracking_projection_node(
                    &entry.milestone_ref,
                    "milestone",
                    &entry.label,
                    &entry.state,
                    project_id,
                    &semantic_revision,
                    source_locator,
                )?);
                relations.push(ProjectionEdge {
                    source: entry.milestone_ref.clone(),
                    relation: "observes".into(),
                    target: entry.activity_ref.clone(),
                });
            }
            for entry in resolved_record_sets {
                changes.nodes.push(tracking_projection_node(
                    &entry.record_set_ref,
                    "record_set",
                    &entry.label,
                    &entry.state,
                    project_id,
                    &semantic_revision,
                    source_locator,
                )?);
                relations.push(ProjectionEdge {
                    source: entry.activity_ref.clone(),
                    relation: "selects-records".into(),
                    target: entry.record_set_ref.clone(),
                });
            }
            for entry in &request.outputs {
                changes.nodes.push(tracking_projection_node(
                    &entry.output_ref,
                    "output",
                    &entry.label,
                    &entry.state,
                    project_id,
                    &semantic_revision,
                    source_locator,
                )?);
                relations.push(ProjectionEdge {
                    source: entry.activity_ref.clone(),
                    relation: "produces".into(),
                    target: entry.output_ref.clone(),
                });
            }
            for entry in &request.procedure_runs {
                changes.nodes.push(tracking_projection_node(
                    &entry.run_ref,
                    "procedure_run",
                    &entry.procedure_ref,
                    &entry.state,
                    project_id,
                    &semantic_revision,
                    source_locator,
                )?);
                relations.push(ProjectionEdge {
                    source: entry.run_ref.clone(),
                    relation: "runs-procedure".into(),
                    target: entry.procedure_ref.clone(),
                });
                relations.push(ProjectionEdge {
                    source: entry.run_ref.clone(),
                    relation: "applies-to".into(),
                    target: entry.activity_ref.clone(),
                });
            }
            for entry in &request.step_runs {
                changes.nodes.push(tracking_projection_node(
                    &entry.step_run_ref,
                    "step_run",
                    &entry.step_ref,
                    &entry.state,
                    project_id,
                    &semantic_revision,
                    source_locator,
                )?);
                relations.push(ProjectionEdge {
                    source: entry.procedure_run_ref.clone(),
                    relation: "has-step-run".into(),
                    target: entry.step_run_ref.clone(),
                });
                relations.push(ProjectionEdge {
                    source: entry.step_run_ref.clone(),
                    relation: "instantiates-step".into(),
                    target: entry.step_ref.clone(),
                });
                if let Some(actor) = &entry.actor_ref {
                    relations.push(ProjectionEdge {
                        source: entry.step_run_ref.clone(),
                        relation: "performed-by".into(),
                        target: actor.clone(),
                    });
                }
                relations.extend(entry.input_bindings.iter().map(|binding| ProjectionEdge {
                    source: entry.step_run_ref.clone(),
                    relation: "uses-input".into(),
                    target: binding.value_ref.clone(),
                }));
                relations.extend(entry.output_bindings.iter().map(|binding| ProjectionEdge {
                    source: entry.step_run_ref.clone(),
                    relation: "produces-output".into(),
                    target: binding.value_ref.clone(),
                }));
                relations.extend(entry.blocking_refs.iter().cloned().map(|target| {
                    ProjectionEdge {
                        source: entry.step_run_ref.clone(),
                        relation: "blocked-by".into(),
                        target,
                    }
                }));
            }
            let mut created_targets = changes
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .collect::<BTreeSet<_>>();
            let mut relation_keys = BTreeSet::new();
            for relation in relations {
                if !relation_keys.insert((
                    relation.source.clone(),
                    relation.relation.clone(),
                    relation.target.clone(),
                )) {
                    continue;
                }
                if read.get_node(&relation.target).is_none()
                    && created_targets.insert(relation.target.clone())
                {
                    changes
                        .nodes
                        .push(Node::new(&relation.target, [EXTERNAL_LABEL]).map_err(graph_error)?);
                }
                let edge = projection_edge(&relation)?;
                if read.get_edge(&edge.id).is_some() {
                    changes.remove_edges.retain(|value| value != &edge.id);
                } else {
                    changes.edges.push(edge);
                }
            }
            Ok(changes)
        },
    )
}

fn next_sequence(revision: &WorkspaceGraphRevision) -> Result<u64, ToolingError> {
    revision
        .sequence
        .checked_add(1)
        .ok_or_else(|| ToolingError::Graph("workspace graph revision is exhausted".into()))
}

fn publish_exact<T, I, C>(
    graph: &Graph,
    space: &GraphSpace,
    expected: &WorkspaceGraphRevision,
    operation: (&str, &str),
    input: &I,
    receipt: &T,
    calculate: C,
) -> Result<T, ToolingError>
where
    T: Serialize + DeserializeOwned + Clone + Eq,
    I: Serialize,
    C: FnOnce(&ReadTransaction) -> Result<GraphChanges, ToolingError>,
{
    let (operation_kind, plan_digest) = operation;
    let exact_revision = RevisionRef {
        sequence: expected.sequence,
        commit: expected.commit.clone(),
    };
    let intent = PublicationIntent {
        operation_id: format!("{operation_kind}/{plan_digest}"),
        expected_commit_revision: exact_revision,
    };
    let request = intent.request(operation_kind, input).map_err(graph_error)?;
    let output = serde_json::to_vec(receipt)?;
    let publication = graph
        .publish(
            space,
            &request,
            |read| {
                let mut changes = calculate(read).map_err(tooling_graph_error)?;
                changes.output = output.clone();
                Ok(changes)
            },
            || Ok(()),
        )
        .map_err(graph_error)?;
    accepted_output(publication, receipt)
}

fn accepted_output<T>(publication: GraphPublication, expected: &T) -> Result<T, ToolingError>
where
    T: DeserializeOwned + Eq,
{
    let committed_revision = match publication.outcome {
        CommitOutcome::Committed(receipt) | CommitOutcome::NoChange(receipt) => {
            receipt.committed_revision.sequence
        }
        outcome => {
            return Err(ToolingError::Graph(format!(
                "workspace publication was not accepted: {outcome:?}"
            )));
        }
    };
    let output = publication
        .output
        .ok_or_else(|| ToolingError::Graph("workspace publication output is missing".into()))?;
    let receipt: T = serde_json::from_slice(&output)?;
    if &receipt != expected {
        return Err(ToolingError::Graph(format!(
            "workspace publication replay did not match its canonical output at revision {committed_revision}"
        )));
    }
    Ok(receipt)
}

fn tooling_graph_error(error: ToolingError) -> zixcel_graph::GraphError {
    zixcel_graph::GraphError::Invalid(error.to_string())
}

fn tracking_projection_node(
    id: &str,
    kind: &str,
    label: &str,
    state: &str,
    owner: &str,
    revision: &str,
    source: &WorkspaceSourceLocator,
) -> Result<Node, ToolingError> {
    let node = projection_node(&ProjectionNode {
        id: id.into(),
        kind: kind.into(),
        label: label.into(),
        state: state.into(),
        owner: owner.into(),
        revision: revision.into(),
    })?;
    json_property(node, SOURCE_LOCATOR, source)
}

fn source_node_id(locator: &WorkspaceSourceLocator) -> Result<String, ToolingError> {
    Ok(format!(
        "source/{}",
        Digest::from_serializable(1, &(&locator.semantic_unit_ref, &locator.unit_digest))
            .map_err(graph_error)?
            .to_hex()
    ))
}

fn decode(read: &ReadTransaction, graph_ref: &str) -> Result<WorkspaceProjection, ToolingError> {
    let mut nodes = read
        .nodes_by_label(RECORD_LABEL, MAX_RECORDS)
        .into_iter()
        .map(|node| decode_node(&node))
        .collect::<Result<Vec<_>, _>>()?;
    let record_ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::<Diagnostic>::new();
    let mut edges = read
        .edges_by_label(RELATION_LABEL, MAX_RELATIONS)
        .into_iter()
        .map(|edge| {
            let relation = edge
                .properties
                .get(RELATION)
                .and_then(property_string)
                .unwrap_or_else(|| edge.label.clone());
            ProjectionEdge {
                source: edge.from,
                relation,
                target: edge.to,
            }
        })
        .filter(|edge| record_ids.contains(&edge.source) && read.get_node(&edge.target).is_some())
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| {
        (&left.source, &left.relation, &left.target).cmp(&(
            &right.source,
            &right.relation,
            &right.target,
        ))
    });

    let graph_nodes = read.nodes_by_label(RECORD_LABEL, MAX_RECORDS);
    let mut activities = Vec::new();
    let mut history = Vec::new();
    let mut connections = Vec::new();
    let mut plans = Vec::new();
    let mut timeline = Vec::new();
    let mut dependencies = Vec::new();
    let mut milestones = Vec::new();
    let mut record_sets = Vec::new();
    let mut outputs = Vec::new();
    let mut procedure_runs = Vec::new();
    let mut step_runs = Vec::new();
    for node in &graph_nodes {
        extend_property(node, ACTIVITY, &mut activities)?;
        extend_property(node, HISTORY, &mut history)?;
        extend_property(node, CONNECTION, &mut connections)?;
        extend_vec_property(node, PLANS, &mut plans)?;
        extend_vec_property(node, TIMELINE, &mut timeline)?;
        extend_vec_property(node, DEPENDENCIES, &mut dependencies)?;
        extend_vec_property(node, MILESTONES, &mut milestones)?;
        extend_vec_property(node, RECORD_SETS, &mut record_sets)?;
        extend_vec_property(node, OUTPUTS, &mut outputs)?;
        extend_vec_property(node, PROCEDURE_RUNS, &mut procedure_runs)?;
        extend_vec_property(node, STEP_RUNS, &mut step_runs)?;
    }
    diagnostics
        .sort_by(|left, right| (&left.code, &left.message).cmp(&(&right.code, &right.message)));
    Ok(WorkspaceProjection {
        presentation: WorkspacePresentation {
            records: nodes
                .iter()
                .map(|node| -> Result<_, serde_json::Error> {
                    Ok(WorkspaceRecordPresentation {
                        record_id: node.id.clone(),
                        source: graph_nodes
                            .iter()
                            .find(|graph_node| graph_node.id == node.id)
                            .and_then(|graph_node| string_property(graph_node, SOURCE_LOCATOR))
                            .map(serde_json::from_str)
                            .transpose()?,
                    })
                })
                .collect::<Result<Vec<_>, serde_json::Error>>()?,
            activities,
            history,
            connections,
            plans,
            timeline,
            dependencies,
            milestones,
            record_sets,
            outputs,
            procedure_runs,
            step_runs,
        },
        nodes,
        edges,
        diagnostics,
        input_roots: vec![graph_ref.into()],
    })
}

fn projection_node(source: &ProjectionNode) -> Result<Node, ToolingError> {
    Node::new(
        &source.id,
        [RECORD_LABEL, &format!("sem_lang_kind_{}", source.kind)],
    )
    .and_then(|node| node.with_property(KIND, PropertyValue::String(source.kind.clone())))
    .and_then(|node| node.with_property(LABEL, PropertyValue::String(source.label.clone())))
    .and_then(|node| node.with_property(STATE, PropertyValue::String(source.state.clone())))
    .and_then(|node| node.with_property(OWNER, PropertyValue::String(source.owner.clone())))
    .and_then(|node| {
        node.with_property(
            SEMANTIC_REVISION,
            PropertyValue::String(source.revision.clone()),
        )
    })
    .map_err(graph_error)
}

fn decode_node(node: &Node) -> Result<ProjectionNode, ToolingError> {
    Ok(ProjectionNode {
        id: node.id.clone(),
        kind: required_string(node, KIND)?,
        label: required_string(node, LABEL)?,
        state: required_string(node, STATE)?,
        owner: required_string(node, OWNER)?,
        revision: required_string(node, SEMANTIC_REVISION)?,
    })
}

fn projection_edge(source: &ProjectionEdge) -> Result<Edge, ToolingError> {
    let digest = Digest::document_digest(
        format!("{}\0{}\0{}", source.source, source.relation, source.target).as_bytes(),
    )
    .to_hex();
    Edge::directed(
        format!("relation/{digest}"),
        &source.source,
        &source.target,
        RELATION_LABEL,
    )
    .and_then(|edge| edge.with_property(RELATION, PropertyValue::String(source.relation.clone())))
    .map_err(graph_error)
}

fn open(
    base: &Path,
    graph_ref: &str,
    project_id: &str,
) -> Result<(Graph, GraphSpace), ToolingError> {
    let path = checked_existing_path(base, graph_ref)?;
    Ok((
        Graph::open_existing(path).map_err(graph_error)?,
        graph_space(project_id)?,
    ))
}

fn graph_space(project_id: &str) -> Result<GraphSpace, ToolingError> {
    GraphSpace::new(project_id).map_err(graph_error)
}

fn checked_relative(base: &Path, graph_ref: &str) -> Result<PathBuf, ToolingError> {
    let relative = Path::new(graph_ref);
    if graph_ref.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ToolingError::Graph(
            "workspace graph path is not project-relative".into(),
        ));
    }
    Ok(base.join(relative))
}

fn checked_new_path(base: &Path, graph_ref: &str) -> Result<PathBuf, ToolingError> {
    let path = checked_relative(base, graph_ref)?;
    if path.exists() {
        return Err(ToolingError::Graph("workspace graph already exists".into()));
    }
    Ok(path)
}

fn checked_existing_path(base: &Path, graph_ref: &str) -> Result<PathBuf, ToolingError> {
    let path = checked_relative(base, graph_ref)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ToolingError::Graph(
            "workspace graph is not a physical file".into(),
        ));
    }
    let canonical_base = fs::canonicalize(base)?;
    if fs::canonicalize(&path)?
        .strip_prefix(canonical_base)
        .is_err()
    {
        return Err(ToolingError::Graph(
            "workspace graph escaped the project".into(),
        ));
    }
    Ok(path)
}

fn json_property<T: Serialize + ?Sized>(
    node: Node,
    key: &str,
    value: &T,
) -> Result<Node, ToolingError> {
    node.with_property(key, PropertyValue::String(serde_json::to_string(value)?))
        .map_err(graph_error)
}

fn property_string(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

fn string_property<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    match node.properties.get(key) {
        Some(PropertyValue::String(value)) => Some(value),
        _ => None,
    }
}

fn string_property_from_edge<'a>(edge: &'a Edge, key: &str) -> Option<&'a str> {
    match edge.properties.get(key) {
        Some(PropertyValue::String(value)) => Some(value),
        _ => None,
    }
}

fn required_string(node: &Node, key: &str) -> Result<String, ToolingError> {
    string_property(node, key)
        .map(str::to_owned)
        .ok_or_else(|| ToolingError::Graph(format!("record {} is missing property {key}", node.id)))
}

fn extend_property<T: DeserializeOwned>(
    node: &Node,
    key: &str,
    output: &mut Vec<T>,
) -> Result<(), ToolingError> {
    if let Some(value) = string_property(node, key) {
        output.push(serde_json::from_str(value)?);
    }
    Ok(())
}

fn extend_vec_property<T: DeserializeOwned>(
    node: &Node,
    key: &str,
    output: &mut Vec<T>,
) -> Result<(), ToolingError> {
    if let Some(value) = string_property(node, key) {
        output.extend(serde_json::from_str::<Vec<T>>(value)?);
    }
    Ok(())
}

fn graph_error(error: impl std::fmt::Display + std::fmt::Debug) -> ToolingError {
    ToolingError::Graph(format!("{error} ({error:?})"))
}

#[cfg(test)]
#[path = "workspace_graph_tests.rs"]
mod tests;
