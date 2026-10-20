use super::*;

#[test]
fn durable_graph_commits_and_reopens() {
    let root =
        std::env::temp_dir().join(format!("sem-lang-workspace-graph-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    create(&root, "data/workspace.redb").unwrap();
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "subject/person/owner/subject".into(),
            kind: "subject".into(),
            label: "Owner".into(),
            state: "active".into(),
            owner: "subject/person/owner/subject".into(),
            revision: "revision/one".into(),
        })
        .unwrap(),
    );
    assert_eq!(write.commit().unwrap(), 1);
    drop(graph);
    let loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert_eq!(loaded.nodes.len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retiring_a_project_compilation_preserves_domain_records() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-project-compilation-retirement-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        crate::project::initialize(&root, "test/semantic-package-migration", "en-US").unwrap();
    let (graph, space) = open(
        &root,
        crate::project::WORKSPACE_GRAPH_PATH,
        "test/semantic-package-migration",
    )
    .unwrap();
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "activity/review/task".into(),
            kind: "activity".into(),
            label: "Review".into(),
            state: "active".into(),
            owner: "subject/person/owner/subject".into(),
            revision: "revision/one".into(),
        })
        .unwrap(),
    );
    write.commit().unwrap();
    drop(graph);

    let retired = crate::project::retire_compilation(&layout.project).unwrap();
    assert!(retired.removed_artifacts >= 2);
    assert!(
        load_project_compilation(
            &root,
            crate::project::WORKSPACE_GRAPH_PATH,
            "test/semantic-package-migration",
        )
        .unwrap()
        .is_none()
    );
    let retained = load(
        &root,
        crate::project::WORKSPACE_GRAPH_PATH,
        "test/semantic-package-migration",
    )
    .unwrap();
    assert!(
        retained
            .nodes
            .iter()
            .any(|node| node.id == "activity/review/task")
    );

    let source = fs::read_to_string(&layout.project).unwrap();
    let compiled = crate::project_source::compile(&source).unwrap();
    let mut manifest = compiled.manifest;
    manifest.time_zone = Some("Etc/UTC".into());
    fs::write(
        &layout.project,
        crate::project_source::render(&manifest, &compiled.analysis_source).unwrap(),
    )
    .unwrap();
    crate::compile_project(&layout.project).unwrap();
    assert!(
        load_project_compilation(
            &root,
            crate::project::WORKSPACE_GRAPH_PATH,
            "test/semantic-package-migration",
        )
        .unwrap()
        .is_some()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_binding_is_exact_and_idempotent() {
    let root =
        std::env::temp_dir().join(format!("sem-lang-workspace-source-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    create(&root, "data/workspace.redb").unwrap();
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "activity/review/task".into(),
            kind: "activity".into(),
            label: "Review".into(),
            state: "active".into(),
            owner: "subject/person/owner/subject".into(),
            revision: "revision/one".into(),
        })
        .unwrap(),
    );
    write.commit().unwrap();
    drop(graph);
    let expected = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let locator = WorkspaceSourceLocator {
        semantic_unit_ref: "activity/review/task".into(),
        source_ref: "source/work.sem".into(),
        document_digest: "source/digest".into(),
        unit_digest: "unit/digest".into(),
        start_line: 1,
        end_line: 1,
    };
    let first_receipt = bind_source(
        &root,
        "data/workspace.redb",
        "test/project",
        &expected,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "activity/review/task",
        &locator,
    )
    .unwrap();

    // A retry must return the original committed owner output even after
    // another operation advances the graph. Current graph state must not
    // be mistaken for the result of the earlier plan.
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "reference/unrelated/reference".into(),
            kind: "reference".into(),
            label: "Unrelated".into(),
            state: "declared".into(),
            owner: "test/project".into(),
            revision: "revision/two".into(),
        })
        .unwrap(),
    );
    write.commit().unwrap();
    drop(graph);

    assert_eq!(
        bind_source(
            &root,
            "data/workspace.redb",
            "test/project",
            &expected,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "activity/review/task",
            &locator,
        )
        .unwrap(),
        first_receipt
    );
    assert!(
        revision(&root, "data/workspace.redb", "test/project")
            .unwrap()
            .sequence
            > first_receipt.graph_revision
    );
    let loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert_eq!(loaded.nodes.len(), 3);
    assert_eq!(
        loaded
            .presentation
            .records
            .iter()
            .find(|record| record.record_id == "activity/review/task")
            .and_then(|record| record.source.as_ref()),
        Some(&locator)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_migration_is_atomic_idempotent_and_reclaims_old_source() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-workspace-source-migration-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    create(&root, "data/workspace.redb").unwrap();
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "activity/development/review/task".into(),
            kind: "activity".into(),
            label: "Review".into(),
            state: "active".into(),
            owner: "subject/person/owner/subject".into(),
            revision: "revision/one".into(),
        })
        .unwrap(),
    );
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "activity/development/approve/task".into(),
            kind: "activity".into(),
            label: "Approve".into(),
            state: "planned".into(),
            owner: "subject/person/owner/subject".into(),
            revision: "revision/one".into(),
        })
        .unwrap(),
    );
    write.commit().unwrap();
    drop(graph);
    let seeded = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let previous = WorkspaceSourceLocator {
        semantic_unit_ref: "activity/development/review/task".into(),
        source_ref: "current-work.sem".into(),
        document_digest: "old/digest".into(),
        // The authored bytes are unchanged; only their file/range locator
        // moves. This keeps the source node identity stable.
        unit_digest: "new/unit/digest".into(),
        start_line: 1,
        end_line: 1,
    };
    let bound = bind_source(
        &root,
        "data/workspace.redb",
        "test/project",
        &seeded,
        "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "activity/development/review/task",
        &previous,
    )
    .unwrap();
    let second_base = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let second_previous = WorkspaceSourceLocator {
        semantic_unit_ref: "activity/development/approve/task".into(),
        source_ref: "approval.sem".into(),
        document_digest: "approval/old/digest".into(),
        unit_digest: "approval/old/unit/digest".into(),
        start_line: 1,
        end_line: 1,
    };
    bind_source(
        &root,
        "data/workspace.redb",
        "test/project",
        &second_base,
        "3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "activity/development/approve/task",
        &second_previous,
    )
    .unwrap();
    let migration_base = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let replacement = WorkspaceSourceLocator {
        semantic_unit_ref: "activity/development/review/task".into(),
        source_ref: "work/development/review.sem".into(),
        document_digest: "new/digest".into(),
        unit_digest: "new/unit/digest".into(),
        start_line: 1,
        end_line: 1,
    };
    let entry = crate::workspace_change::SourceMigrationEntry {
        record_ref: "activity/development/review/task".into(),
        previous: previous.clone(),
        replacement: replacement.clone(),
    };
    let second_entry = crate::workspace_change::SourceMigrationEntry {
        record_ref: "activity/development/approve/task".into(),
        previous: second_previous,
        replacement: replacement.clone(),
    };
    let migrated = migrate_sources(
        &root,
        "data/workspace.redb",
        "test/project",
        &migration_base,
        "2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        &[entry.clone(), second_entry.clone()],
    )
    .unwrap();
    assert!(migrated.graph_revision > bound.graph_revision);
    assert_eq!(
        migrate_sources(
            &root,
            "data/workspace.redb",
            "test/project",
            &migration_base,
            "2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            &[entry, second_entry],
        )
        .unwrap(),
        migrated
    );
    let loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert_eq!(loaded.nodes.len(), 3);
    assert_eq!(
        loaded
            .presentation
            .records
            .iter()
            .find(|record| record.record_id == "activity/development/review/task")
            .and_then(|record| record.source.as_ref()),
        Some(&replacement)
    );
    assert_eq!(
        loaded
            .presentation
            .records
            .iter()
            .find(|record| record.record_id == "activity/development/approve/task")
            .and_then(|record| record.source.as_ref()),
        Some(&replacement)
    );
    assert_eq!(
        source_node_id(&previous).expect("stable source id"),
        source_node_id(&replacement).expect("replacement source id")
    );
    assert!(
        loaded.nodes.iter().any(|node| {
            node.id == source_node_id(&replacement).expect("replacement source id")
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_source_is_feedback_and_never_replaces_the_bound_digest() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-workspace-source-stale-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout = crate::project::initialize(&root, "test/source_stale", "en-US").unwrap();
    let manifest = crate::project_source::compile(&fs::read_to_string(&layout.project).unwrap())
        .unwrap()
        .manifest;
    let original_source = crate::project_source::render(
        &manifest,
        "# Observation\n  Input:\n    Project context.\n    Review evidence.\n",
    )
    .unwrap();
    fs::write(&layout.project, &original_source).unwrap();
    let evidence_line = original_source
        .lines()
        .position(|line| line == "    Review evidence.")
        .unwrap() as u32;
    let (graph, space) = open(
        &root,
        crate::project::WORKSPACE_GRAPH_PATH,
        "test/source_stale",
    )
    .unwrap();
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(
        projection_node(&ProjectionNode {
            id: "activity/review/task".into(),
            kind: "activity".into(),
            label: "Review".into(),
            state: "active".into(),
            owner: "subject/person/owner/subject".into(),
            revision: "revision/one".into(),
        })
        .unwrap(),
    );
    write.commit().unwrap();
    drop(graph);
    let expected = revision(
        &root,
        crate::project::WORKSPACE_GRAPH_PATH,
        "test/source_stale",
    )
    .unwrap();
    bind_source(
        &root,
        crate::project::WORKSPACE_GRAPH_PATH,
        "test/source_stale",
        &expected,
        "3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "activity/review/task",
        &WorkspaceSourceLocator {
            semantic_unit_ref: "activity/review/task".into(),
            source_ref: "project.sem".into(),
            document_digest: Digest::document_digest(original_source.as_bytes()).to_hex(),
            unit_digest: Digest::document_digest(b"    Review evidence.").to_hex(),
            start_line: evidence_line,
            end_line: evidence_line,
        },
    )
    .unwrap();

    let current = crate::WorkspaceAnalyzer::load(&layout.project).unwrap();
    assert!(
        current.project_source_analyses()[0]
            .analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "SEM4001")
    );
    let unrelated_source = crate::project_source::render(
        &manifest,
        "# Observation\n  Input:\n    Changed project context.\n    Review evidence.\n",
    )
    .unwrap();
    fs::write(&layout.project, unrelated_source).unwrap();
    let unrelated_change = crate::WorkspaceAnalyzer::load(&layout.project).unwrap();
    assert!(
        unrelated_change.project_source_analyses()[0]
            .analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "SEM4001")
    );
    assert!(unrelated_change.project_source_analyses()[0].source_bindings[0].current);
    let changed_source = crate::project_source::render(
        &manifest,
        "# Observation\n  Input:\n    Changed project context.\n    Review changed evidence.\n",
    )
    .unwrap();
    fs::write(&layout.project, changed_source).unwrap();
    let changed = crate::WorkspaceAnalyzer::load(&layout.project).unwrap();
    assert!(
        changed.project_source_analyses()[0]
            .analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SEM4001")
    );
    let loaded = load(
        &root,
        crate::project::WORKSPACE_GRAPH_PATH,
        "test/source_stale",
    )
    .unwrap();
    let original_digest = Digest::document_digest(original_source.as_bytes()).to_hex();
    assert_eq!(
        loaded
            .presentation
            .records
            .iter()
            .find(|record| record.record_id == "activity/review/task")
            .and_then(|record| record.source.as_ref())
            .map(|source| source.document_digest.as_str()),
        Some(original_digest.as_str())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task_registration_is_atomic_and_idempotent() {
    let root = std::env::temp_dir().join(format!("sem-lang-workspace-task-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    create(&root, "data/workspace.redb").unwrap();
    let request = crate::workspace_change::TaskRegistrationRequest {
        schema_version: crate::SCHEMA_VERSION.into(),
        task_path: "development/review".into(),
        activity_state: crate::workspace::ActivityState::Active,
        owner_ref: "subject/person/owner/subject".into(),
        method_ref: "method/manual/review/procedure".into(),
        title: "Review current evidence".into(),
        next_instruction: "Select the first evidence record".into(),
        source_ref: "work/development/review.sem".into(),
        source_start_line: 1,
        source_end_line: 2,
        authority_refs: Vec::new(),
        reference_refs: Vec::new(),
    };
    let source = WorkspaceSourceLocator {
        semantic_unit_ref: format!("activity/{}/task", request.task_path),
        source_ref: request.source_ref.clone(),
        document_digest: "source/digest".into(),
        unit_digest: "unit/digest".into(),
        start_line: request.source_start_line,
        end_line: request.source_end_line,
    };
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut seed = graph.write(&space).unwrap();
    for (id, kind) in [
        (&request.owner_ref, "subject"),
        (&request.method_ref, "method"),
    ] {
        seed.upsert_node(
            projection_node(&ProjectionNode {
                id: id.clone(),
                kind: kind.into(),
                label: id.clone(),
                state: "declared".into(),
                owner: request.owner_ref.clone(),
                revision: "seed/revision".into(),
            })
            .unwrap(),
        );
    }
    seed.commit().unwrap();
    drop(graph);
    let expected = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let revision = register_task(
        &root,
        "data/workspace.redb",
        "test/project",
        &expected,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        &request,
        &source,
    )
    .unwrap();
    assert!(revision.graph_revision > expected.sequence);
    assert_eq!(
        register_task(
            &root,
            "data/workspace.redb",
            "test/project",
            &expected,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            &request,
            &source,
        )
        .unwrap(),
        revision
    );
    let loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert!(loaded.nodes.iter().any(|node| {
        node.id == "activity/development/review/task"
            && node.label == "Review current evidence"
            && node.state == "active"
    }));
    assert_eq!(loaded.presentation.activities.len(), 1);
    assert_eq!(loaded.nodes.len(), 6);
    for record_ref in [
        "activity/development/review/task",
        "reference/activity/development/review/title/reference",
        "reference/activity/development/review/next/reference",
    ] {
        assert_eq!(
            loaded
                .presentation
                .records
                .iter()
                .find(|record| record.record_id == record_ref)
                .and_then(|record| record.source.as_ref()),
            Some(&source)
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn history_registration_is_append_only_source_bound_and_idempotent() {
    let root =
        std::env::temp_dir().join(format!("sem-lang-workspace-history-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    create(&root, "data/workspace.redb").unwrap();
    let request = crate::workspace_change::HistoryRegistrationRequest {
        schema_version: crate::SCHEMA_VERSION.into(),
        history_path: "design/detailed/202512".into(),
        title: "Detailed design work recorded".into(),
        former_activity_ref: "activity/design/detailed/task".into(),
        owner_ref: "subject/person/owner/subject".into(),
        outcome: "recorded".into(),
        occurred_at: "2025-12-31T23:59:59+09:00".into(),
        source_revision: "a".repeat(64),
        evidence_refs: vec!["evidence/report/202512/reference".into()],
        source_ref: "records/history.sem".into(),
        source_start_line: 1,
        source_end_line: 1,
    };
    let source = WorkspaceSourceLocator {
        semantic_unit_ref: format!("history/{}/record", request.history_path),
        source_ref: request.source_ref.clone(),
        document_digest: "b".repeat(64),
        unit_digest: "c".repeat(64),
        start_line: 1,
        end_line: 1,
    };
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut seed = graph.write(&space).unwrap();
    seed.upsert_node(
        projection_node(&ProjectionNode {
            id: request.owner_ref.clone(),
            kind: "subject".into(),
            label: "Owner".into(),
            state: "declared".into(),
            owner: request.owner_ref.clone(),
            revision: "seed/revision".into(),
        })
        .unwrap(),
    );
    seed.commit().unwrap();
    drop(graph);
    let expected = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let digest = "c".repeat(64);
    let first = register_history(
        &root,
        "data/workspace.redb",
        "test/project",
        &expected,
        &digest,
        &request,
        &source,
    )
    .unwrap();
    assert_eq!(
        register_history(
            &root,
            "data/workspace.redb",
            "test/project",
            &expected,
            &digest,
            &request,
            &source,
        )
        .unwrap(),
        first
    );
    let loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert_eq!(loaded.presentation.history.len(), 1);
    assert_eq!(
        loaded.presentation.history[0].history_ref,
        "history/design/detailed/202512/record"
    );
    assert_eq!(
        loaded.presentation.history[0].former_activity_ref,
        request.former_activity_ref
    );
    assert_eq!(
        loaded
            .presentation
            .records
            .iter()
            .find(|value| value.record_id == "history/design/detailed/202512/record")
            .and_then(|value| value.source.as_ref()),
        Some(&source)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_work_scenario_persists_dependencies_milestone_record_sets_and_output() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-workspace-tracking-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    create(&root, "data/workspace.redb").unwrap();
    let (graph, space) = open(&root, "data/workspace.redb", "test/project").unwrap();
    let mut seed = graph.write(&space).unwrap();
    for (id, kind, state) in [
        ("activity/current/wbs", "activity", "declared"),
        ("time/current/schedule", "time", "declared"),
        ("activity/report/classify/task", "activity", "active"),
        ("activity/report/review/task", "activity", "planned"),
        ("activity/report/render/task", "activity", "planned"),
    ] {
        seed.upsert_node(
            projection_node(&ProjectionNode {
                id: id.into(),
                kind: kind.into(),
                label: id.into(),
                state: state.into(),
                owner: "subject/person/owner/subject".into(),
                revision: "seed/revision".into(),
            })
            .unwrap(),
        );
    }
    seed.commit().unwrap();
    drop(graph);
    let expected = revision(&root, "data/workspace.redb", "test/project").unwrap();
    let request = crate::workspace_change::WorkTrackingRequest {
        schema_version: crate::SCHEMA_VERSION.into(),
        source_ref: "work/reporting.sem".into(),
        source_start_line: 1,
        source_end_line: 1,
        plans: vec![
            crate::workspace::WorkspacePlanEntry {
                plan_ref: "activity/current/wbs".into(),
                task_ref: "activity/report/classify/task".into(),
                parent_task_ref: None,
                order: 1,
            },
            crate::workspace::WorkspacePlanEntry {
                plan_ref: "activity/current/wbs".into(),
                task_ref: "activity/report/review/task".into(),
                parent_task_ref: Some("activity/report/classify/task".into()),
                order: 2,
            },
            crate::workspace::WorkspacePlanEntry {
                plan_ref: "activity/current/wbs".into(),
                task_ref: "activity/report/render/task".into(),
                parent_task_ref: Some("activity/report/review/task".into()),
                order: 3,
            },
        ],
        timeline: vec![crate::workspace::WorkspaceTimelineEntry {
            time_ref: "time/current/schedule".into(),
            activity_ref: "activity/report/classify/task".into(),
            starts_at: "2026-09-25T00:00:00+09:00".into(),
            ends_at: "2026-09-27T23:59:59+09:00".into(),
            timezone: "Asia/Tokyo".into(),
        }],
        dependencies: vec![
            crate::workspace::WorkspaceDependencyEntry {
                prerequisite_ref: "activity/report/classify/task".into(),
                dependent_ref: "activity/report/review/task".into(),
            },
            crate::workspace::WorkspaceDependencyEntry {
                prerequisite_ref: "activity/report/review/task".into(),
                dependent_ref: "activity/report/render/task".into(),
            },
        ],
        milestones: vec![crate::workspace::WorkspaceMilestonePresentation {
            milestone_ref: "milestone/report/quarter/completion".into(),
            activity_ref: "activity/report/render/task".into(),
            label: "Quarter report complete".into(),
            state: "active".into(),
            planned_at: "2026-09-30T23:59:59+09:00".into(),
            actual_at: None,
            timezone: "Asia/Tokyo".into(),
            target_value: "reviewed PDF".into(),
            actual_value: Some("evidence review active".into()),
            variance: Some("final wording remains".into()),
            evidence_refs: vec!["evidence/report/september/source".into()],
        }],
        record_sets: vec![
            crate::workspace_change::WorkspaceRecordSetRequest {
                record_set_ref: "records/report/july/set".into(),
                activity_ref: "activity/report/classify/task".into(),
                label: "July records".into(),
                state: "classified".into(),
                period_start: "2026-07-01".into(),
                period_end: "2026-07-31".into(),
            },
            crate::workspace_change::WorkspaceRecordSetRequest {
                record_set_ref: "records/report/september/set".into(),
                activity_ref: "activity/report/classify/task".into(),
                label: "September records".into(),
                state: "collecting".into(),
                period_start: "2026-09-01".into(),
                period_end: "2026-09-30".into(),
            },
        ],
        outputs: vec![crate::workspace::WorkspaceOutputPresentation {
            output_ref: "output/report/quarter/pdf".into(),
            activity_ref: "activity/report/render/task".into(),
            label: "Reviewed quarter report".into(),
            state: "planned".into(),
            role: "reviewed_delivery".into(),
            operation_id: "document/render/pdf".into(),
            evidence_refs: vec!["records/report/september/set".into()],
        }],
        procedure_runs: vec![crate::workspace::WorkspaceProcedureRunPresentation {
            run_ref: "run/report/quarter/procedure".into(),
            procedure_ref: "method/report/quarter".into(),
            activity_ref: "activity/report/classify/task".into(),
            state: "active".into(),
            current_step_ref: Some("method/report/quarter/step/classify".into()),
        }],
        step_runs: vec![crate::workspace::WorkspaceStepRunPresentation {
            step_run_ref: "run/report/quarter/step/classify".into(),
            procedure_run_ref: "run/report/quarter/procedure".into(),
            step_ref: "method/report/quarter/step/classify".into(),
            state: "active".into(),
            actor_ref: Some("subject/person/owner/subject".into()),
            input_bindings: vec![crate::workspace::WorkspaceDataBinding {
                data_role_ref: "input/report/source".into(),
                value_ref: "evidence/report/september/source".into(),
            }],
            output_bindings: vec![],
            blocking_refs: vec![],
        }],
    };
    let locator = WorkspaceSourceLocator {
        semantic_unit_ref: "tracking/project/work/record".into(),
        source_ref: request.source_ref.clone(),
        document_digest: "source/digest".into(),
        unit_digest: "unit/digest".into(),
        start_line: 1,
        end_line: 1,
    };
    let resolved_record_sets = vec![
        crate::workspace::WorkspaceRecordSetPresentation {
            record_set_ref: "records/report/july/set".into(),
            activity_ref: "activity/report/classify/task".into(),
            label: "July records".into(),
            state: "classified".into(),
            period_start: "2026-07-01".into(),
            period_end: "2026-07-31".into(),
            record_refs: vec!["evidence/report/july/source".into()],
        },
        crate::workspace::WorkspaceRecordSetPresentation {
            record_set_ref: "records/report/september/set".into(),
            activity_ref: "activity/report/classify/task".into(),
            label: "September records".into(),
            state: "collecting".into(),
            period_start: "2026-09-01".into(),
            period_end: "2026-09-30".into(),
            record_refs: vec!["evidence/report/september/source".into()],
        },
    ];
    let committed = replace_work_tracking(
        &root,
        "data/workspace.redb",
        WorkTrackingReplacement {
            project_id: "test/project",
            expected_revision: &expected,
            plan_digest: "4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            content_digest: "tracking/digest",
            request: &request,
            resolved_record_sets: &resolved_record_sets,
            source_locator: &locator,
        },
    )
    .unwrap();
    assert_eq!(
        replace_work_tracking(
            &root,
            "data/workspace.redb",
            WorkTrackingReplacement {
                project_id: "test/project",
                expected_revision: &expected,
                plan_digest: "4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                content_digest: "tracking/digest",
                request: &request,
                resolved_record_sets: &resolved_record_sets,
                source_locator: &locator,
            },
        )
        .unwrap(),
        committed
    );
    let loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert_eq!(loaded.presentation.plans, request.plans);
    assert_eq!(loaded.presentation.timeline, request.timeline);
    assert_eq!(loaded.presentation.dependencies, request.dependencies);
    assert_eq!(loaded.presentation.milestones, request.milestones);
    assert_eq!(loaded.presentation.record_sets, resolved_record_sets);
    assert_eq!(loaded.presentation.outputs, request.outputs);
    assert_eq!(loaded.presentation.procedure_runs, request.procedure_runs);
    assert_eq!(loaded.presentation.step_runs, request.step_runs);
    assert!(loaded.edges.iter().any(|edge| {
        edge.source == "activity/report/review/task"
            && edge.relation == "depends-on"
            && edge.target == "activity/report/classify/task"
    }));
    let mut changed = request.clone();
    changed.step_runs[0].state = "blocked".into();
    let next_expected = revision(&root, "data/workspace.redb", "test/project").unwrap();
    replace_work_tracking(
        &root,
        "data/workspace.redb",
        WorkTrackingReplacement {
            project_id: "test/project",
            expected_revision: &next_expected,
            plan_digest: "5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            content_digest: "tracking/changed-digest",
            request: &changed,
            resolved_record_sets: &resolved_record_sets,
            source_locator: &locator,
        },
    )
    .unwrap();
    let changed_loaded = load(&root, "data/workspace.redb", "test/project").unwrap();
    assert_eq!(changed_loaded.presentation.step_runs, changed.step_runs);
    assert!(changed_loaded.edges.iter().any(|edge| {
        edge.source == "activity/report/review/task"
            && edge.relation == "depends-on"
            && edge.target == "activity/report/classify/task"
    }));
    assert!(loaded.edges.iter().any(|edge| {
        edge.source == "run/report/quarter/step/classify"
            && edge.relation == "instantiates-step"
            && edge.target == "method/report/quarter/step/classify"
    }));
    fs::remove_dir_all(root).unwrap();
}
