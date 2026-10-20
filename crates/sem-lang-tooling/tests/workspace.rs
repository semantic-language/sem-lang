use sem_lang_tooling::WorkspaceAnalyzer;
use sem_lang_tooling::{
    AuthenticationState, BehaviorInjectionState, BehaviorPackageSelection, ExecutionChoiceState,
    PreviewFieldState, PreviewState, ProjectManifest, ProjectSourceDocument, ProjectSourceRole,
    VersionControlSelection, semantic_package::SemanticPackageSelection,
};
use serde_json::json;
use std::fs;

fn installed_language_digest(root: &std::path::Path, relative: &str) -> String {
    sem_lang_core::Digest::document_digest(&fs::read(root.join(relative)).unwrap()).to_hex()
}

fn project_manifest(project_id: &str, locale: &str) -> ProjectManifest {
    ProjectManifest {
        schema_version: "0.10.0".into(),
        project_id: project_id.into(),
        locale: locale.into(),
        time_zone: None,
        semantic_packages: Vec::new(),
        source_documents: vec![ProjectSourceDocument {
            path: "project.sem".into(),
            role: ProjectSourceRole::ProjectLanguage,
        }],
        service_roots: vec![".sem-lang/services".into()],
        service_state_roots: vec![".sem-lang/runtime/services".into()],
        behavior_packages: Vec::new(),
        procedure_packages: Vec::new(),
        workspace_graph: None,
        version_control: None,
    }
}

fn write_project(root: &std::path::Path, manifest: &ProjectManifest, program: &str) {
    fs::write(
        root.join("project.sem"),
        sem_lang_tooling::project_source::render(manifest, program).unwrap(),
    )
    .unwrap();
    let graph = root.join(sem_lang_tooling::project::WORKSPACE_GRAPH_PATH);
    if !graph.exists() {
        sem_lang_tooling::workspace_graph::create(
            root,
            sem_lang_tooling::project::WORKSPACE_GRAPH_PATH,
        )
        .unwrap();
    }
}

#[test]
fn selected_semantic_packages_form_an_order_independent_closed_dependency_set() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-semantic-packages-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".sem-lang/services")).unwrap();
    fs::create_dir_all(root.join(".sem-lang/runtime/services")).unwrap();
    let extension = "packages/example/extension/language/extension.sem";
    fs::create_dir_all(root.join(extension).parent().unwrap()).unwrap();
    fs::write(
        root.join(extension),
        "# 言語\n  依存: sem-lang/standard/semantic\n",
    )
    .unwrap();
    let extension_manifest = "packages/example/extension/package.sem";
    fs::write(
        root.join(extension_manifest),
        format!(
            "# Semantic package\n  Version: 0.10.0\n  Owner: organization/example\n\n# Package dependency\n  Identifier: sem-lang/standard\n\n# Language source\n  Source: language/extension.sem\n  Digest: {}\n",
            installed_language_digest(&root, extension)
        ),
    )
    .unwrap();
    let mut manifest = project_manifest("project/semantic-packages", "ja-JP");
    manifest.semantic_packages.push(SemanticPackageSelection {
        manifest: extension_manifest.into(),
        digest_sha256: installed_language_digest(&root, extension_manifest),
    });
    write_project(&root, &manifest, "");
    let first = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    let semantic_revision = first.project_compilation().semantic_revision.clone();

    let unchanged = manifest.clone();
    write_project(&root, &manifest, "");
    let reordered = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    assert_eq!(
        reordered.project_compilation().semantic_revision,
        semantic_revision
    );
    assert_eq!(manifest, unchanged);

    fs::write(
        root.join(extension),
        "# 言語\n  依存: missing/package/language/core\n",
    )
    .unwrap();
    fs::write(
        root.join(extension_manifest),
        format!(
            "# Semantic package\n  Version: 0.10.0\n  Owner: organization/example\n\n# Package dependency\n  Identifier: missing/package\n\n# Language source\n  Source: language/extension.sem\n  Digest: {}\n",
            installed_language_digest(&root, extension)
        ),
    )
    .unwrap();
    manifest
        .semantic_packages
        .iter_mut()
        .find(|selection| selection.manifest == extension_manifest)
        .unwrap()
        .digest_sha256 = installed_language_digest(&root, extension_manifest);
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_pipeline_value_requires_and_projects_an_authority_owned_binding() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-value-binding-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("packages/fixture/value/language")).unwrap();
    fs::create_dir_all(root.join("packages/fixture/value/modules")).unwrap();
    fs::create_dir_all(root.join(".sem-lang/services")).unwrap();
    fs::create_dir_all(root.join(".sem-lang/runtime/services")).unwrap();

    let language = "packages/fixture/value/language/core.sem";
    fs::write(
        root.join(language),
        "# 言語\n  依存: sem-lang/standard/semantic\n\n# 用語\n  名称: CurrentWork\n  説明: Current work\n  意味: Primitive/Value\n\n# 用語\n  名称: CompletedWork\n  説明: Completed work\n  意味: Primitive/Value\n\n# 表現\n  言語: ja-JP\n  表記: 現在の作業\n  正規形: 現在の作業\n  種別: Denotational\n  対象: CurrentWork\n  優先度: 50\n  出典: fixture/value\n\n# 表現\n  言語: ja-JP\n  表記: 完了した作業\n  正規形: 完了した作業\n  種別: Denotational\n  対象: CompletedWork\n  優先度: 50\n  出典: fixture/value\n",
    )
    .unwrap();
    let module = "packages/fixture/value/modules/current-work.sem";
    fs::write(
        root.join(module),
        "# 意味モジュール\n  所有者: subject/fixture/project\n\n# 主体\n  識別子: subject/fixture/project\n  種別: kind/project\n  表示名: Fixture\n  言語: en-US\n  公開範囲: internal\n  正本: authority/fixture\n  正本の版: revision/fixture/one\n  根拠: reference/fixture\n\n# 値束縛\n  識別子: value/fixture/current-work\n  用語ソース: fixture/value/language/core\n  用語: CurrentWork\n  値の型: kind/work/activity\n  件数: many\n  正本: workspace/fixture\n  生成元: workspace/activity/current\n  実体化: reference_only\n  根拠: source/work/current\n",
    )
    .unwrap();
    let package = "packages/fixture/value/package.sem";
    let package_source = format!(
        "# 意味パッケージ\n  版: 0.10.0\n  所有者: subject/fixture/project\n\n# 依存パッケージ\n  識別子: sem-lang/standard\n\n# 言語ソース\n  ソース: language/core.sem\n  ダイジェスト: {}\n\n# 意味モジュール\n  ソース: modules/current-work.sem\n  ダイジェスト: {}\n",
        installed_language_digest(&root, language),
        installed_language_digest(&root, module)
    );
    fs::write(root.join(package), package_source).unwrap();

    let mut manifest = project_manifest("project/value-binding", "ja-JP");
    manifest.semantic_packages.push(SemanticPackageSelection {
        manifest: package.into(),
        digest_sha256: installed_language_digest(&root, package),
    });
    let program = "# 依頼\n\n入力:\n  - 現在の作業\n\n結果:\n  - 完了した作業\n";
    write_project(&root, &manifest, program);
    let analysis = WorkspaceAnalyzer::load(root.join("project.sem"))
        .unwrap()
        .analyze(program);
    assert_eq!(analysis.pipelines[0].state, "structured");
    let input = &analysis.pipelines[0].steps[0].values[0];
    assert!(input.binding_required);
    assert_eq!(input.bindings[0].value_ref, "value/fixture/current-work");
    assert_eq!(input.bindings[0].state, "empty");
    assert!(input.bindings[0].resolved_value_refs.is_empty());
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM0120")
    );

    fs::write(
        root.join(package),
        format!(
            "# 意味パッケージ\n  版: 0.10.0\n  所有者: subject/fixture/project\n\n# 依存パッケージ\n  識別子: sem-lang/standard\n\n# 言語ソース\n  ソース: language/core.sem\n  ダイジェスト: {}\n",
            installed_language_digest(&root, language)
        ),
    )
    .unwrap();
    manifest.semantic_packages[0].digest_sha256 = installed_language_digest(&root, package);
    write_project(&root, &manifest, program);
    let missing = WorkspaceAnalyzer::load(root.join("project.sem"))
        .unwrap()
        .analyze(program);
    assert_eq!(missing.pipelines[0].state, "invalid");
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM0120")
    );
    fs::remove_dir_all(root).unwrap();
}

fn edit_project(path: &std::path::Path, edit: impl FnOnce(&mut ProjectManifest), program: &str) {
    let source = fs::read_to_string(path).unwrap();
    let mut manifest = sem_lang_tooling::project_source::compile(&source)
        .unwrap()
        .manifest;
    edit(&mut manifest);
    fs::write(
        path,
        sem_lang_tooling::project_source::render(&manifest, program).unwrap(),
    )
    .unwrap();
}

fn copy_tree(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn selected_procedure_package_stays_inert_and_reports_missing_behavior() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-procedure-package-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        sem_lang_tooling::project::initialize(&root, "project/procedure-package", "ja-JP").unwrap();
    let relative = ".sem-lang/packages/example/work/procedure-package.json";
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "0.10.0",
            "package_ref": "example/work",
            "revision": "0.10.0",
            "owner_ref": "subject/example/owner",
            "roles": [{
                "role_ref": "role/example/operator",
                "label": "Operator",
                "executor_kind": "person",
                "responsibilities": ["Operate the selected procedure."],
                "constraints": ["Do not invent missing input."]
            }],
            "data_roles": [{
                "data_role_ref": "input/example/source",
                "direction": "input",
                "value_kind_ref": "kind/example/source",
                "label": "Source",
                "authority": "external_owner",
                "sensitivity": "internal",
                "materialization": "reference_only",
                "required": true,
                "description": "Exact source reference."
            }, {
                "data_role_ref": "output/example/observation",
                "direction": "output",
                "value_kind_ref": "kind/example/observation",
                "label": "Observation",
                "authority": "project",
                "sensitivity": "internal",
                "materialization": "project_record",
                "required": true,
                "description": "Recorded observation."
            }],
            "procedures": [{
                "procedure_ref": "method/example/work",
                "label": "Example work",
                "purpose": "Describe reusable work without implementing it.",
                "source_refs": ["reference/example/source"],
                "steps": [{
                    "step_ref": "method/example/work/step/observe",
                    "order": 1,
                    "label": "Observe",
                    "purpose": "Observe one exact source.",
                    "input_roles": ["input/example/source"],
                    "output_roles": ["output/example/observation"],
                    "completion_conditions": ["The observation reference exists."],
                    "actor_role_refs": ["role/example/operator"],
                    "operation_requirements": [{
                        "requirement_ref": "requirement/example/observe",
                        "operation_id": "source/example/observe",
                        "optional": false
                    }],
                    "next_step_ref": null
                }]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    edit_project(
        &layout.project,
        |manifest| {
            manifest.procedure_packages.push(
                sem_lang_tooling::procedure::ProcedurePackageSelection {
                    manifest: relative.into(),
                    digest_sha256: sem_lang_core::Digest::document_digest(
                        &fs::read(&path).unwrap(),
                    )
                    .to_hex(),
                },
            );
        },
        "",
    );

    let analyzer = WorkspaceAnalyzer::load(&layout.project).unwrap();
    let selected = analyzer.procedure_package_presentation();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].procedures[0].steps.len(), 1);
    assert_eq!(
        analyzer
            .project_diagnostics()
            .iter()
            .filter(|value| value.code == "SEM0004")
            .count(),
        1
    );
    assert!(
        analyzer
            .workspace_presentation()
            .unwrap()
            .procedure_runs
            .is_empty()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_template_exposes_only_authored_sources_and_hides_machine_state() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-project-template-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        sem_lang_tooling::project::initialize(&root, "person/reservation-planning", "ja-JP")
            .unwrap();
    assert_eq!(layout.root, root);
    assert!(layout.project.is_file());
    assert!(!root.join("sem.project.json").exists());
    assert!(layout.workspace_graph.is_file());
    assert!(!layout.root.join("source").exists());
    assert!(!layout.root.join("data").exists());
    assert!(!layout.root.join("definitions").exists());
    assert!(!layout.root.join("services").exists());
    assert!(layout.services_root.is_dir());
    assert!(layout.runtime_services_root.is_dir());
    assert!(layout.runtime_root.join("receipts").is_dir());
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        ".sem-lang/\n"
    );

    let manifest =
        sem_lang_tooling::project_source::compile(&fs::read_to_string(&layout.project).unwrap())
            .unwrap()
            .manifest;
    assert_eq!(manifest.project_id, "person/reservation-planning");
    assert_eq!(manifest.locale, "ja-JP");
    assert!(manifest.time_zone.is_none());
    assert!(manifest.semantic_packages.is_empty());
    assert!(!root.join("packages/sem-lang/core").exists());
    assert_eq!(manifest.source_documents[0].path, "project.sem");
    assert_eq!(
        manifest.source_documents[0].role,
        ProjectSourceRole::ProjectLanguage
    );
    assert_eq!(manifest.service_roots[0], ".sem-lang/services");
    assert_eq!(
        manifest.workspace_graph.as_deref(),
        Some(".sem-lang/data/workspace.redb")
    );
    assert_eq!(
        manifest.service_state_roots[0],
        ".sem-lang/runtime/services"
    );
    let analyzer = WorkspaceAnalyzer::load(&layout.project).unwrap();
    let sources = analyzer.project_source_analyses();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source_ref, "project.sem");
    assert_eq!(
        sources[0].role,
        sem_lang_tooling::ProjectSourceRole::ProjectLanguage
    );
    let all_project_text = [layout.project.clone(), root.join("README.md")]
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    for forbidden in ["access_token\"", "client_secret\"", "private_key\""] {
        assert!(!all_project_text.contains(forbidden));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.join(".sem-lang"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&layout.runtime_services_root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let analyzer = WorkspaceAnalyzer::load(&layout.project).unwrap();
    assert_eq!(analyzer.project_diagnostics().len(), 1);
    assert_eq!(analyzer.project_diagnostics()[0].code, "SEM0002");
    let sources = analyzer.project_source_analyses();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source_ref, "project.sem");
    assert!(sources[0].analysis.diagnostics.is_empty());
    assert!(analyzer.project_compilation().inputs.iter().any(|input| {
        input.kind == "standard_language_set"
            && input.path == "sem-lang/standard@0.10.0"
            && input.digest_sha256.len() == 64
    }));
    assert!(
        sem_lang_tooling::project::initialize(&root, "person/reservation-planning", "ja-JP",)
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_save_compilation_is_durable_idempotent_and_keeps_last_valid_revision() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-project-compilation-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        sem_lang_tooling::project::initialize(&root, "person/compiled-project", "ja-JP").unwrap();

    let (initial, initial_receipt) = sem_lang_tooling::workspace_graph::load_project_compilation(
        &root,
        sem_lang_tooling::project::WORKSPACE_GRAPH_PATH,
        "person/compiled-project",
    )
    .unwrap()
    .expect("initializer publishes compilation");
    assert_eq!(initial.source_digest, initial_receipt.source_digest);

    let (_, replay) = sem_lang_tooling::compile_project(&layout.project).unwrap();
    assert_eq!(replay, initial_receipt);
    assert_eq!(
        sem_lang_tooling::workspace_graph::revision(
            &root,
            sem_lang_tooling::project::WORKSPACE_GRAPH_PATH,
            "person/compiled-project",
        )
        .unwrap()
        .sequence,
        initial_receipt.graph_revision
    );

    let source = fs::read_to_string(&layout.project).unwrap();
    let compiled = sem_lang_tooling::project_source::compile(&source).unwrap();
    let mut manifest = compiled.manifest;
    manifest.time_zone = Some("Asia/Tokyo".into());
    fs::write(
        &layout.project,
        sem_lang_tooling::project_source::render(&manifest, &compiled.analysis_source).unwrap(),
    )
    .unwrap();
    let (_, changed) = sem_lang_tooling::compile_project(&layout.project).unwrap();
    assert_ne!(changed.artifact_digest, initial_receipt.artifact_digest);
    assert!(changed.graph_revision > initial_receipt.graph_revision);

    fs::write(&layout.project, "project:\n---\n").unwrap();
    assert!(sem_lang_tooling::compile_project(&layout.project).is_err());
    let (retained, retained_receipt) = sem_lang_tooling::workspace_graph::load_project_compilation(
        &root,
        sem_lang_tooling::project::WORKSPACE_GRAPH_PATH,
        "person/compiled-project",
    )
    .unwrap()
    .expect("last successful compilation remains readable");
    assert_eq!(retained.artifact_digest, changed.artifact_digest);
    assert_eq!(retained_receipt, changed);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn incomplete_project_context_is_feedback_not_analyzer_failure() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-context-feedback-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        sem_lang_tooling::project::initialize(&root, "project/context-feedback", "ja-JP").unwrap();
    let program = "# 依頼\n  入力:\n    利用者の依頼を確認する。\n";
    edit_project(&layout.project, |manifest| manifest.locale.clear(), program);

    let analyzer = WorkspaceAnalyzer::load(&layout.project).unwrap();
    assert_eq!(
        analyzer
            .project_diagnostics()
            .iter()
            .map(|value| value.code.as_str())
            .collect::<Vec<_>>(),
        ["SEM0001", "SEM0002"]
    );
    let analysis = analyzer.project_source_analyses();
    assert_eq!(analysis.len(), 1);
    assert!(analysis[0].analysis.diagnostics.is_empty());
    assert_eq!(analysis[0].analysis.projection.nodes.len(), 3);

    edit_project(
        &layout.project,
        |manifest| {
            manifest.locale = "ja-JP".into();
            manifest.time_zone = Some("invalid-zone".into());
        },
        program,
    );
    assert!(WorkspaceAnalyzer::load(&layout.project).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_vocabulary_and_sources_are_confined_to_physical_project_relative_files() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-project-path-boundary-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        sem_lang_tooling::project::initialize(&root, "person/path-boundary", "en-US").unwrap();
    let original = fs::read_to_string(&layout.project).unwrap();
    let original_manifest = sem_lang_tooling::project_source::compile(&original)
        .unwrap()
        .manifest;
    let mut manifest = original_manifest.clone();

    manifest.semantic_packages.push(SemanticPackageSelection {
        manifest: "/outside/package.sem".into(),
        digest_sha256: "0".repeat(64),
    });
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(&layout.project).is_err());

    manifest.semantic_packages[0].manifest = "../outside/package.sem".into();
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(&layout.project).is_err());

    manifest = original_manifest.clone();
    fs::write(root.join(".sem-lang/generated.sem"), "internal\n").unwrap();
    manifest.source_documents.push(ProjectSourceDocument {
        path: ".sem-lang/generated.sem".into(),
        role: ProjectSourceRole::Work,
    });
    write_project(&root, &manifest, "");
    assert!(
        WorkspaceAnalyzer::load(&layout.project)
            .err()
            .unwrap()
            .to_string()
            .contains("source role is derived from the work or records root directory")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let alias = root.join("packages/alias/package.sem");
        fs::create_dir_all(alias.parent().unwrap()).unwrap();
        symlink(&layout.project, &alias).unwrap();
        manifest = original_manifest.clone();
        manifest.semantic_packages.push(SemanticPackageSelection {
            manifest: "packages/alias/package.sem".into(),
            digest_sha256: installed_language_digest(&root, "project.sem"),
        });
        write_project(&root, &manifest, "");
        assert!(WorkspaceAnalyzer::load(&layout.project).is_err());
    }

    manifest = original_manifest.clone();
    manifest.source_documents.push(ProjectSourceDocument {
        path: layout
            .project
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        role: ProjectSourceRole::Work,
    });
    assert!(sem_lang_tooling::project_source::render(&manifest, "").is_ok());
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(&layout.project).is_err());

    manifest = original_manifest.clone();
    manifest.source_documents.push(ProjectSourceDocument {
        path: "../outside.sem".into(),
        role: ProjectSourceRole::Work,
    });
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(&layout.project).is_err());

    let duplicate = format!("{original}\n# ソース\n  パス: project.sem\n");
    fs::write(&layout.project, duplicate).unwrap();
    assert!(WorkspaceAnalyzer::load(&layout.project).is_err());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let alias = root.join("alias.sem");
        symlink(&layout.project, &alias).unwrap();
        manifest = original_manifest;
        manifest.source_documents.push(ProjectSourceDocument {
            path: "alias.sem".into(),
            role: ProjectSourceRole::Work,
        });
        write_project(&root, &manifest, "");
        assert!(WorkspaceAnalyzer::load(&layout.project).is_err());
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_behavior_packages_resolve_exact_operations_and_dependencies() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-behavior-packages-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".sem-lang/runtime/services")).unwrap();
    fs::write(root.join("source.sem"), "").unwrap();

    let definitions = [
        (
            "source/folder",
            "zixcel/source",
            "source/folder/observe",
            "source/folder/observe",
            Vec::<&str>::new(),
        ),
        (
            "source/workbook",
            "zixcel/source",
            "source/workbook/observe",
            "source/workbook/observe",
            Vec::<&str>::new(),
        ),
        (
            "design/evidence",
            "zixcel/design",
            "design/evidence/classify",
            "design/evidence/classify",
            vec!["source/folder/observe", "source/workbook/observe"],
        ),
    ];
    let mut selections = Vec::new();
    for (target, owner, operation, behavior, requires) in definitions {
        let capability_path = root.join(format!(".sem-lang/services/{target}/capability.json"));
        fs::create_dir_all(capability_path.parent().unwrap()).unwrap();
        let semantic_bindings = if behavior == "design/evidence/classify" {
            // `Create` / 作る is an existing project-language word. The
            // selected package does not add syntax; it injects executable
            // behavior for that exact admitted TermId.
            json!([{
                "term_id": sem_lang_core::TermId::from_declaration("sem-lang/standard/semantic", "Create"),
                "operation_id": operation
            }])
        } else {
            json!([])
        };
        fs::write(
            &capability_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "0.10.0",
                "owner": owner,
                "revision": "0.10.0",
                "resource": {
                    "resource_id": target,
                    "contract_revision": "0.10.0",
                    "value_schema": {"type": "null"},
                    "readable": false,
                    "availability": {"state": "available"},
                    "operations": [{
                        "operation_id": operation,
                        "target": target,
                        "contract_revision": "0.10.0",
                        "input_schema": {"type": "object", "fields": {}, "required": []},
                        "availability": {"state": "available"},
                        "expected_revision_required": true
                    }]
                },
                "semantic_bindings": semantic_bindings,
                "input_bindings": [],
                "authentication": null
            }))
            .unwrap(),
        )
        .unwrap();
        let package_ref = target.to_owned();
        let package_path = root.join(format!(".sem-lang/packages/{target}/behavior-package.json"));
        fs::create_dir_all(package_path.parent().unwrap()).unwrap();
        let bytes = serde_json::to_vec_pretty(&json!({
            "schema": "zixcel://contracts/behavior/package/v1",
            "package_ref": package_ref,
            "package_revision": "0.10.0",
            "owner": owner,
            "artifact": {
                "registry_ref": "zixcel/local/registry",
                "package_ref": package_ref,
                "release": "0.10.0",
                "digest_sha256": "a".repeat(64)
            },
            "implementations": [{
                "behavior_ref": behavior,
                "operation_id": operation,
                "target": target,
                "contract_revision": "0.10.0",
                "input_type_ref": "source/reference",
                "output_type_ref": "source/observation",
                "effect": "observe",
                "requires": requires
            }]
        }))
        .unwrap();
        fs::write(&package_path, &bytes).unwrap();
        selections.push(BehaviorPackageSelection {
            manifest: package_path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            digest_sha256: sem_lang_core::Digest::document_digest(&bytes).to_hex(),
            publisher_ref: owner.into(),
            admission_ref: format!("admission/project/behavior/{target}/0.10.0"),
        });
    }
    let mut manifest = project_manifest("test/behavior/packages", "ja-JP");
    manifest.behavior_packages = selections;
    write_project(&root, &manifest, "");

    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    let packages = analyzer.behavior_package_presentation();
    assert_eq!(packages.len(), 3);
    assert!(
        packages
            .iter()
            .all(|value| value.state == BehaviorInjectionState::Ready)
    );
    let composition = packages
        .iter()
        .find(|value| value.package_ref == "design/evidence")
        .unwrap();
    assert_eq!(composition.implementations[0].requirements.len(), 2);
    assert!(
        composition.implementations[0]
            .requirements
            .iter()
            .all(|value| value.state == BehaviorInjectionState::Ready
                && value.provider_package_ref.is_some())
    );
    let create = analyzer
        .completions()
        .into_iter()
        .find(|value| value.label == "作る")
        .unwrap();
    assert_eq!(create.behavior_bindings.len(), 1);
    assert_eq!(create.behavior_bindings[0].package_ref, "design/evidence");
    assert_eq!(
        create.behavior_bindings[0].behavior_ref,
        "design/evidence/classify"
    );
    assert_eq!(
        create.behavior_bindings[0].state,
        BehaviorInjectionState::Ready
    );
    assert!(create.detail.contains("design/evidence"));

    fs::remove_file(root.join(".sem-lang/services/source/folder/capability.json")).unwrap();
    let unavailable = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    assert!(
        unavailable
            .behavior_package_presentation()
            .iter()
            .any(|value| value.package_ref == "source/folder"
                && value.state == BehaviorInjectionState::Unavailable)
    );
    assert!(
        unavailable
            .project_diagnostics()
            .iter()
            .any(|value| value.code == "SEM0003")
    );

    let admitted_publisher = manifest.behavior_packages[0].publisher_ref.clone();
    manifest.behavior_packages[0].publisher_ref = "untrusted/publisher".into();
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());
    manifest.behavior_packages[0].publisher_ref = admitted_publisher;
    manifest.behavior_packages[0].digest_sha256 = "b".repeat(64);
    write_project(&root, &manifest, "");
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn incomplete_reservation_exposes_missing_fields_targets_and_authentication() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/vscode-workspace");
    let workspace = std::env::temp_dir().join(format!(
        "sem-lang-tooling-editor-example-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&workspace);
    copy_tree(&fixture, &workspace);
    sem_lang_tooling::project::initialize_storage(workspace.join("project.sem")).unwrap();
    let analyzer = WorkspaceAnalyzer::load(workspace.join("project.sem")).unwrap();
    let capabilities = analyzer.capability_presentation();
    assert!(capabilities.iter().any(|capability| {
        capability.operation_id == "calendar/event/create"
            && capability.target == "calendar/personal"
            && capability
                .authentication
                .as_ref()
                .is_some_and(|authentication| {
                    authentication.state == AuthenticationState::AuthorizationRequired
                })
    }));
    let source = fs::read_to_string(workspace.join("work/instruction.sem")).unwrap();
    let analysis = analyzer.analyze(&source);
    assert_eq!(analysis.pipelines.len(), 1);
    assert_eq!(analysis.pipelines[0].state, "structured");
    assert_eq!(analysis.execution_choices.len(), 1);
    let choice = analysis.execution_choices.first().unwrap();
    assert_eq!(choice.state, ExecutionChoiceState::NeedsRoute);
    assert_eq!(choice.routes.len(), 4);
    let requirement = choice
        .routes
        .iter()
        .find(|route| route.operation_id == "calendar/event/create")
        .unwrap();

    assert_eq!(requirement.preview.state, PreviewState::NeedsInput);
    assert_eq!(requirement.preview.targets.len(), 2);
    let personal = requirement
        .preview
        .targets
        .iter()
        .find(|target| target.target == "calendar/personal")
        .unwrap();
    assert_eq!(
        personal.authentication.as_ref().unwrap().state,
        AuthenticationState::AuthorizationRequired
    );
    assert_eq!(
        personal.authentication.as_ref().unwrap().state_owner,
        "zixcel/reference/authentication/runtime"
    );
    assert!(
        personal
            .fields
            .iter()
            .any(|field| { field.path == "/end" && field.state == PreviewFieldState::Missing })
    );
    assert!(personal.fields.iter().any(|field| {
        field.path == "/start" && field.state == PreviewFieldState::ObservedNeedsNormalization
    }));
    assert!(
        analysis.projection.nodes.iter().any(|node| {
            node.kind == "authentication" && node.state == "authorization-required"
        })
    );
    assert!(choice.routes.iter().any(|route| {
        route.operation_id == "calendar/invitation/create"
            && route.preview.targets.iter().all(|target| {
                target.fields.iter().any(|field| {
                    field.path == "/participants" && field.state == PreviewFieldState::Missing
                })
            })
    }));
    assert!(choice.routes.iter().any(|route| {
        route.operation_id == "organization/meeting/reserve"
            && route.preview.targets[0].fields.iter().any(|field| {
                field.path == "/department" && field.state == PreviewFieldState::Missing
            })
    }));
    assert!(choice.routes.iter().any(|route| {
        route.operation_id == "service/appointment/request"
            && route.preview.state == PreviewState::Unavailable
            && route.preview.targets[0].availability_reason.as_deref()
                == Some("provider-selection-required")
    }));
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SEM3000")
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SEM2002")
    );
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SEM2001")
    );

    let unrelated_creation = analyzer.analyze("資料を作ってください。");
    assert!(unrelated_creation.execution_choices.is_empty());
    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn project_authentication_rejects_secret_fields_and_unknown_targets() {
    let root = std::env::temp_dir().join(format!("sem-lang-tooling-auth-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let capability_path = root.join(".sem-lang/services/calendar/one/capability.json");
    let authentication_path =
        root.join(".sem-lang/services/authentication/example/authentication.json");
    let state_path = root.join(".sem-lang/runtime/services/authentication/example/state.json");
    for path in [&capability_path, &authentication_path, &state_path] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    fs::write(root.join("source.sem"), "").unwrap();
    let mut capability = json!({
        "schema_version": "0.10.0",
        "owner": "zixcel/example",
        "revision": "catalog-1",
        "resource": {
            "resource_id": "calendar/one",
            "contract_revision": "contract-1",
            "value_schema": {"type": "null"},
            "readable": false,
            "availability": {"state": "available"},
            "operations": []
        },
        "semantic_bindings": [],
        "input_bindings": [],
        "authentication": "authentication/example"
    });
    fs::write(
        &capability_path,
        serde_json::to_vec_pretty(&capability).unwrap(),
    )
    .unwrap();
    let manifest = project_manifest("test/authentication", "ja-JP");
    write_project(&root, &manifest, "");

    let mut authentication = json!({
        "schema_version": "0.10.0",
        "owner": "zixcel/auth",
        "revision": "auth-1",
        "label": "Example",
        "configuration_ref": "zixcel://authentication/configuration/example",
        "token": "must-not-be-accepted"
    });
    fs::write(
        &authentication_path,
        serde_json::to_vec_pretty(&authentication).unwrap(),
    )
    .unwrap();
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "0.10.0",
            "owner": "zixcel/auth-runtime",
            "revision": "auth-runtime-1",
            "state": "ready"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());

    authentication.as_object_mut().unwrap().remove("token");
    fs::write(
        &authentication_path,
        serde_json::to_vec_pretty(&authentication).unwrap(),
    )
    .unwrap();
    capability["authentication"] = json!("authentication/unknown");
    fs::write(
        &capability_path,
        serde_json::to_vec_pretty(&capability).unwrap(),
    )
    .unwrap();
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());

    capability["authentication"] = json!("authentication/example");
    fs::write(
        &capability_path,
        serde_json::to_vec_pretty(&capability).unwrap(),
    )
    .unwrap();
    fs::remove_file(&state_path).unwrap();
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());

    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "0.10.0",
            "owner": "zixcel/auth-runtime",
            "revision": "auth-runtime-3",
            "state": "ready"
        }))
        .unwrap(),
    )
    .unwrap();
    WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();

    let obsolete_catalog = root.join(".sem-lang/services/calendar/catalog.json");
    fs::write(&obsolete_catalog, b"{}\n").unwrap();
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());
    fs::remove_file(obsolete_catalog).unwrap();

    capability["resource"]["resource_id"] = json!("calendar/different");
    fs::write(
        &capability_path,
        serde_json::to_vec_pretty(&capability).unwrap(),
    )
    .unwrap();
    assert!(WorkspaceAnalyzer::load(root.join("project.sem")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resolves_language_and_exact_available_capability() {
    let root = std::env::temp_dir().join(format!("sem-lang-tooling-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let capability_path = root.join(".sem-lang/services/example/resource/capability.json");
    fs::create_dir_all(capability_path.parent().unwrap()).unwrap();
    fs::create_dir_all(root.join(".sem-lang/runtime/services")).unwrap();
    fs::write(root.join("source.sem"), "").unwrap();
    fs::write(
        &capability_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "0.10.0",
            "owner": "zixcel/example",
            "revision": "catalog-1",
            "resource": {
                "resource_id": "example/resource",
                "contract_revision": "catalog-1",
                "value_schema": {"type": "null"},
                "readable": false,
                "availability": {"state": "available"},
                "operations": [{
                    "operation_id": "example/create",
                    "target": "example/resource",
                    "contract_revision": "catalog-1",
                    "input_schema": {"type": "object", "fields": {}, "required": []},
                    "availability": {"state": "available"},
                    "expected_revision_required": false
                }]
            },
            "semantic_bindings": [{
                "term_id": sem_lang_core::TermId::from_declaration("sem-lang/standard/semantic", "Create"),
                "operation_id": "example/create"
            }],
            "input_bindings": [],
            "authentication": null
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = project_manifest("test/available-capability", "ja-JP");
    write_project(&root, &manifest, "");
    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    let analysis = analyzer.analyze("作る");
    assert!(analysis.execution_choices.iter().any(|choice| {
        choice.routes.iter().any(|route| {
            route.operation_id == "example/create"
                && route.preview.state == sem_lang_tooling::PreviewState::Ready
        })
    }));
    assert!(
        analysis
            .projection
            .nodes
            .iter()
            .any(|node| node.id == "operation:example/create" && node.state == "ready")
    );
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM1001")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_text_and_missing_capability_are_never_filled() {
    let root =
        std::env::temp_dir().join(format!("sem-lang-tooling-negative-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let capability_path = root.join(".sem-lang/services/example/resource/capability.json");
    fs::create_dir_all(capability_path.parent().unwrap()).unwrap();
    fs::create_dir_all(root.join(".sem-lang/runtime/services")).unwrap();
    fs::write(root.join("source.sem"), "").unwrap();
    fs::write(
        &capability_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "0.10.0",
            "owner": "zixcel/example",
            "revision": "catalog-1",
            "resource": {
                "resource_id": "example/resource",
                "contract_revision": "catalog-1",
                "value_schema": {"type": "null"},
                "readable": false,
                "availability": {"state": "available"},
                "operations": [{
                    "operation_id": "example/create",
                    "target": "example/resource",
                    "contract_revision": "catalog-1",
                    "input_schema": {"type": "object", "fields": {}, "required": []},
                    "availability": {"state": "unavailable", "reason": "not-configured"},
                    "expected_revision_required": false
                }]
            },
            "semantic_bindings": [{
                "term_id": sem_lang_core::TermId::from_declaration("sem-lang/standard/semantic", "Create"),
                "operation_id": "example/create"
            }],
            "input_bindings": [],
            "authentication": null
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = project_manifest("test/unavailable-capability", "ja-JP");
    write_project(&root, &manifest, "");
    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    let analysis = analyzer.analyze("未定義の指示\n作る");
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM1001"
                && item.severity == sem_lang_tooling::Severity::Warning)
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM2001")
    );
    assert_eq!(analysis.execution_choices.len(), 1);
    assert_eq!(
        analysis.execution_choices[0].routes[0].preview.state,
        sem_lang_tooling::PreviewState::Unavailable
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn detailed_instruction_preserves_resolved_and_unresolved_surface_evidence() {
    let root = std::env::temp_dir().join(format!("sem-lang-tooling-detail-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".sem-lang/services")).unwrap();
    fs::create_dir_all(root.join(".sem-lang/runtime/services")).unwrap();
    fs::write(root.join("source.sem"), "").unwrap();
    let manifest = project_manifest("test/detailed-instruction", "ja-JP");
    write_project(&root, &manifest, "");

    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    let short = analyzer.analyze("予約を作ってください。");
    let detailed = analyzer.analyze("予約を作ってください。明日の8時に会議室Aです。");

    assert_ne!(short.source_digest, detailed.source_digest);
    assert_ne!(
        short.projection.projection_digest,
        detailed.projection.projection_digest
    );
    assert!(detailed.projection.nodes.iter().any(|node| {
        node.kind == "surface" && node.label == "明日" && node.state == "matched"
    }));
    assert!(detailed.projection.nodes.iter().any(|node| {
        node.kind == "semantic" && node.label == "Tomorrow" && node.state == "resolved"
    }));
    assert!(detailed.projection.nodes.iter().any(|node| {
        node.kind == "unresolved"
            && node.label.contains("8時")
            && node.label.contains("会議室A")
            && node.state == "requires-definition"
    }));
    assert!(
        !short
            .projection
            .nodes
            .iter()
            .any(|node| node.label.contains("8時") || node.label.contains("会議室A"))
    );
    assert!(
        detailed
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM1003")
    );

    let request = analyzer
        .semantic_fix_request(
            "予約を作ってください。明日の8時に会議室Aです。",
            0,
            "SEM1003",
        )
        .unwrap();
    assert_eq!(request.source_digest, detailed.source_digest);
    assert_eq!(request.semantic_revision, analyzer.semantic_revision());
    assert!(
        request
            .unresolved
            .iter()
            .any(|item| item.surface.contains("8時") && item.surface.contains("会議室A"))
    );
    assert!(
        request
            .resolved_terms
            .iter()
            .any(|item| item.label == "Tomorrow")
    );
    assert!(request.allowed_terms_complete);
    assert!(
        request
            .allowed_terms
            .iter()
            .any(|item| item.label == "Place")
    );

    let project_instruction = "予約を作ってください。明日の8時に会議室Aです。";
    let project_program = format!("# 依頼\n  入力:\n    {project_instruction}\n");
    let project_source = sem_lang_tooling::project_source::render(&manifest, &project_program)
        .expect("render project.sem");
    let project_line = project_source
        .lines()
        .position(|line| line.trim() == project_instruction)
        .expect("project program line") as u32;
    let project_request = analyzer
        .semantic_fix_request_project_source(&project_source, project_line, "SEM1003")
        .expect("project semantic repair request");
    assert_eq!(project_request.instruction, project_instruction);
    assert_eq!(
        project_request.source_digest,
        sem_lang_core::Digest::document_digest(project_source.as_bytes()).to_hex()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn contribution_source_is_structured_by_an_explicit_localized_communicative_act() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-structured-contribution-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    sem_lang_tooling::project::initialize(&root, "project/structured", "ja-JP").unwrap();
    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();
    let kinds = analyzer.contribution_structure_kinds();
    assert_eq!(
        kinds
            .iter()
            .map(|kind| kind.surface.as_str())
            .collect::<Vec<_>>(),
        ["依頼", "質問", "記録"]
    );
    let source = analyzer
        .structure_contribution(&kinds[1].term_id, "権限ごとの操作可否を確認したい。")
        .unwrap();
    assert_eq!(
        source,
        "# 質問\n\n入力:\n  - 権限ごとの操作可否を確認したい。\n"
    );
    let analysis = analyzer.analyze_contribution(&source);
    assert!(analysis.projection.nodes.iter().any(|node| {
        node.kind == "section" && node.label == "質問" && node.state == "structured"
    }));
    assert!(analysis.projection.edges.iter().any(|edge| {
        edge.source == "section:1"
            && edge.relation == "contains-step"
            && edge.target == "pipeline:1:1"
    }));
    assert_eq!(analysis.pipelines.len(), 1);
    assert_eq!(analysis.pipelines[0].state, "invalid");
    assert_eq!(analysis.pipelines[0].steps[0].operator, "source");
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM0111")
    );
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|item| item.code == "SEM0101")
    );

    let unstructured = analyzer.analyze_contribution("権限ごとの操作可否を確認したい。");
    assert!(
        unstructured
            .diagnostics
            .iter()
            .any(|item| { item.code == "SEM0101" && item.owner == "sem-lang/source-structure" })
    );
}

#[test]
fn narrative_properties_preserve_unresolved_evidence_without_compiler_warnings() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-narrative-property-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    sem_lang_tooling::project::initialize(&root, "project/narrative", "ja-JP").unwrap();
    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();

    let analysis = analyzer.analyze(
        "# 依頼\n\n目的:\n  利用者が自由に記述した説明。\n\n完了条件:\n  利用者が結果を確認できる。",
    );

    assert!(analysis.projection.nodes.iter().any(|node| {
        node.kind == "unresolved" && node.label.contains("自由に記述した説明")
    }));
    assert!(
        analysis
            .projection
            .nodes
            .iter()
            .any(|node| { node.kind == "unresolved" && node.label.contains("結果を確認") })
    );
    assert!(
        !analysis
            .diagnostics
            .iter()
            .any(|item| item.severity == sem_lang_tooling::Severity::Warning)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn target_property_remains_semantically_strict() {
    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-strict-target-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    sem_lang_tooling::project::initialize(&root, "project/strict-target", "ja-JP").unwrap();
    let analyzer = WorkspaceAnalyzer::load(root.join("project.sem")).unwrap();

    let analysis = analyzer.analyze("# 依頼\n\n対象:\n  未定義の対象");

    assert!(analysis.diagnostics.iter().any(|item| {
        item.code == "SEM1001" && item.severity == sem_lang_tooling::Severity::Warning
    }));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shareable_markdown_lists_and_localized_properties_compile_to_the_same_semantics() {
    let mut semantic_targets = Vec::new();
    for (locale, source, expected_heading, expected_value) in [
        ("ja-JP", "# 依頼\n\n目的:\n  - 予約\n", "目的", "予約"),
        (
            "en-US",
            "# Request\n\nPurpose:\n  - reservation\n",
            "Purpose",
            "reservation",
        ),
    ] {
        let root = std::env::temp_dir().join(format!(
            "sem-lang-tooling-markdown-{}-{}",
            locale,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        sem_lang_tooling::project::initialize(&root, "project/markdown", locale).unwrap();
        let analysis = WorkspaceAnalyzer::load(root.join("project.sem"))
            .unwrap()
            .analyze_contribution(source);
        let property = analysis
            .projection
            .nodes
            .iter()
            .find(|node| node.kind == "property" && node.label == expected_heading)
            .expect("localized structural property");
        semantic_targets.push(
            analysis
                .projection
                .edges
                .iter()
                .find(|edge| edge.source == property.id && edge.relation == "denotes")
                .expect("property meaning")
                .target
                .clone(),
        );
        assert!(
            analysis
                .projection
                .nodes
                .iter()
                .any(|node| { node.kind == "instruction" && node.label == expected_value })
        );
        assert!(
            !analysis
                .projection
                .nodes
                .iter()
                .any(|node| node.label.starts_with("- "))
        );
        fs::remove_dir_all(root).unwrap();
    }
    assert_eq!(semantic_targets[0], semantic_targets[1]);
}

#[test]
fn optional_git_observation_tracks_user_changes_without_owning_work_history() {
    use sem_lang_tooling::workspace_change::{
        HistoryRegistrationRequest, apply_history_registration, plan_history_registration,
    };
    use std::process::Command;

    let root = std::env::temp_dir().join(format!(
        "sem-lang-tooling-version-control-scenario-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let layout =
        sem_lang_tooling::project::initialize(&root, "project/version-control-scenario", "ja-JP")
            .unwrap();
    let optional = sem_lang_tooling::project::observe_source_versioning(&layout.project).unwrap();
    assert_eq!(optional.state, "not-configured");

    let module_ref = "packages/scenario/context/modules/person.sem";
    let package_ref = "packages/scenario/context/package.sem";
    fs::create_dir_all(root.join(module_ref).parent().unwrap()).unwrap();
    fs::write(
        root.join(module_ref),
        "# 意味モジュール\n  所有者: subject/person/scenario-user\n\n# 主体\n  識別子: subject/person/scenario-user\n  種別: kind/person\n  表示名: Scenario User\n  言語: ja-JP\n  公開範囲: internal\n  正本: identity/person/scenario-user\n  正本の版: revision/scenario/user/one\n  根拠: reference/scenario/user\n",
    )
    .unwrap();
    fs::write(
        root.join(package_ref),
        format!("# 意味パッケージ\n  版: 0.10.0\n  所有者: subject/person/scenario-user\n\n# 依存パッケージ\n  識別子: sem-lang/standard\n\n# 意味モジュール\n  ソース: modules/person.sem\n  ダイジェスト: {}\n", installed_language_digest(&root, module_ref)),
    )
    .unwrap();
    edit_project(
        &layout.project,
        |manifest| {
            manifest.semantic_packages.push(SemanticPackageSelection {
                manifest: package_ref.into(),
                digest_sha256: installed_language_digest(&root, package_ref),
            });
            manifest.version_control = Some(VersionControlSelection {
                provider: "git".into(),
                root: ".".into(),
            });
        },
        "",
    );
    let unavailable =
        sem_lang_tooling::project::observe_source_versioning(&layout.project).unwrap();
    assert_eq!(unavailable.state, "unavailable");

    let git = |arguments: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(arguments)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.name", "Scenario User"]);
    git(&["config", "user.email", "scenario@example.invalid"]);
    git(&["add", "project.sem", "README.md", ".gitignore", "packages"]);
    git(&["commit", "--quiet", "-m", "initialize project"]);
    let initial = sem_lang_tooling::project::observe_source_versioning(&layout.project).unwrap();
    assert_eq!(initial.match_state, "exact");
    assert_eq!(
        initial.observation.as_ref().unwrap().working_state,
        zixcel_version_control::WorkingState::Clean
    );

    fs::create_dir_all(root.join("records")).unwrap();
    fs::write(
        root.join("records/git-evidence.sem"),
        "# 記録\n\n入力:\n  版管理証跡を確認した。\n",
    )
    .unwrap();
    edit_project(
        &layout.project,
        |manifest| {
            manifest.source_documents.push(ProjectSourceDocument {
                path: "records/git-evidence.sem".into(),
                role: ProjectSourceRole::Records,
            });
        },
        "",
    );
    let working = sem_lang_tooling::project::observe_source_versioning(&layout.project).unwrap();
    assert_eq!(working.match_state, "working-copy");
    assert!(working.observation.as_ref().unwrap().changed_entry_count >= 1);
    let (_, before_commit) = sem_lang_tooling::compile_project(&layout.project).unwrap();

    git(&["add", "project.sem", "records/git-evidence.sem"]);
    git(&["commit", "--quiet", "-m", "add versioned work"]);
    let committed = sem_lang_tooling::project::observe_source_versioning(&layout.project).unwrap();
    assert_eq!(committed.match_state, "exact");
    let observation_ref = committed
        .observation
        .as_ref()
        .unwrap()
        .observation_ref
        .clone();
    let (_, after_commit) = sem_lang_tooling::compile_project(&layout.project).unwrap();
    assert_eq!(before_commit.artifact_digest, after_commit.artifact_digest);

    let request = HistoryRegistrationRequest {
        schema_version: "0.10.0".into(),
        history_path: "version/control/scenario".into(),
        title: "版管理証跡を確認した".into(),
        former_activity_ref: "activity/version-control/scenario/task".into(),
        owner_ref: "subject/person/scenario-user".into(),
        outcome: "completed".into(),
        occurred_at: "2026-09-29T12:00:00+09:00".parse().unwrap(),
        source_revision: after_commit.source_digest,
        evidence_refs: vec![observation_ref.clone()],
        source_ref: "records/git-evidence.sem".into(),
        source_start_line: 0,
        source_end_line: 3,
    };
    let plan = plan_history_registration(&layout.project, request).unwrap();
    let first = apply_history_registration(&layout.project, &plan).unwrap();
    let retry = apply_history_registration(&layout.project, &plan).unwrap();
    assert_eq!(first, retry);
    let presentation = WorkspaceAnalyzer::load(&layout.project)
        .unwrap()
        .workspace_presentation()
        .unwrap()
        .clone();
    assert_eq!(presentation.history.len(), 1);
    assert_eq!(presentation.history[0].evidence_refs, vec![observation_ref]);

    fs::remove_dir_all(root).unwrap();
}
