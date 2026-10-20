//! A bounded, dependency-light Language Server for sem-lang.
//!
//! It owns editor protocol state only. Semantic analysis and typed workspace
//! mutations remain in `sem-lang-tooling`; capability execution stays outside.
//! Mutation requests always cross the immutable plan/apply boundary and never
//! run as a side effect of analysis or presentation.

use sem_lang_tooling::{
    Analysis, SCHEMA_VERSION, SemanticFixRequest, Severity, ToolingError, ToolingIssue,
    WorkspaceAnalyzer, compile_project, semantic_module,
    workspace_change::{
        HistoryRegistrationPlan, HistoryRegistrationRequest, TaskRegistrationPlan,
        TaskRegistrationRequest, apply_history_registration, apply_task_registration,
        plan_history_registration, plan_task_registration,
    },
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("sem-lang-language-server 0.10.0");
        return;
    }
    if let Err(error) = run() {
        eprintln!("sem-lang-language-server: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let project = project_argument()?;
    let (mut analyzer, _) = compile_project(&project)?;
    let mut project_index = ProjectIndex::new(&project, &analyzer)?;
    let input = io::stdin();
    let mut reader = BufReader::new(input.lock());
    let output = io::stdout();
    let mut writer = output.lock();
    let mut documents = BTreeMap::<String, Document>::new();
    let mut shutdown = false;
    while let Some(message) = read_message(&mut reader)? {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => respond(
                &mut writer,
                id,
                json!({
                    "capabilities": {
                        "textDocumentSync": {"openClose": true, "change": 1, "save": true},
                        "completionProvider": {"triggerCharacters": [" ", "。"]},
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "documentSymbolProvider": true,
                        "codeActionProvider": {"codeActionKinds": ["quickfix"]}
                    },
                    "serverInfo": {"name": "sem-lang", "version": "0.10.0"}
                }),
            )?,
            "initialized" => {}
            "shutdown" => {
                shutdown = true;
                respond(&mut writer, id, Value::Null)?;
            }
            "exit" => return Ok(()),
            "textDocument/didOpen" => {
                if let Some(document) = params.get("textDocument") {
                    let uri = string(document, "uri")?;
                    documents.insert(
                        uri.clone(),
                        Document {
                            text: string(document, "text")?,
                            version: document.get("version").and_then(Value::as_i64),
                        },
                    );
                    publish(
                        &mut writer,
                        &analyzer,
                        &project_index,
                        &uri,
                        documents.get(&uri).unwrap(),
                    )?;
                }
            }
            "textDocument/didChange" => {
                let document = params.get("textDocument").ok_or("missing textDocument")?;
                let uri = string(document, "uri")?;
                let text = params
                    .get("contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str)
                    .ok_or("missing full document change")?
                    .to_owned();
                let entry = Document {
                    text,
                    version: document.get("version").and_then(Value::as_i64),
                };
                documents.insert(uri.clone(), entry);
                publish(
                    &mut writer,
                    &analyzer,
                    &project_index,
                    &uri,
                    documents.get(&uri).unwrap(),
                )?;
            }
            "textDocument/didSave" => {
                let uri = text_document_uri(&params)?;
                if project_index.compilation_input(&uri) {
                    match compile_project(&project) {
                        Ok((compiled, project_compilation)) => {
                            let compiled_index = ProjectIndex::new(&project, &compiled)?;
                            analyzer = compiled;
                            project_index = compiled_index;
                            notify(
                                &mut writer,
                                "semLang/projectCompilation",
                                json!({
                                    "schema_version": SCHEMA_VERSION,
                                    "state": "compiled",
                                    "compilation": &project_compilation
                                }),
                            )?;
                            if let Some(document) = documents.get(&uri) {
                                publish(&mut writer, &analyzer, &project_index, &uri, document)?;
                            }
                        }
                        Err(error) => {
                            notify(
                                &mut writer,
                                "semLang/projectCompilation",
                                json!({
                                    "schema_version": SCHEMA_VERSION,
                                    "state": "invalid",
                                    "error": error.to_string()
                                }),
                            )?;
                            if let Some(document) = documents.get(&uri) {
                                notify(
                                    &mut writer,
                                    "textDocument/publishDiagnostics",
                                    json!({
                                        "uri": uri,
                                        "version": document.version,
                                        "diagnostics": source_error_diagnostic(
                                            &document.text,
                                            &error.to_string(),
                                            "SEM5003",
                                            "sem-lang/project-compilation",
                                            "correct-and-save-project-source",
                                        )
                                    }),
                                )?;
                            }
                        }
                    }
                }
            }
            "textDocument/didClose" => {
                let uri = string(
                    params.get("textDocument").ok_or("missing textDocument")?,
                    "uri",
                )?;
                documents.remove(&uri);
                notify(
                    &mut writer,
                    "textDocument/publishDiagnostics",
                    json!({"uri": uri, "diagnostics": []}),
                )?;
            }
            "textDocument/completion" => {
                let uri = text_document_uri(&params).ok();
                let values = if uri
                    .as_deref()
                    .and_then(|value| project_index.declaration_source(value))
                    .is_some()
                {
                    Vec::new()
                } else {
                    analyzer
                        .completions()
                        .into_iter()
                        .map(completion_item)
                        .collect::<Vec<_>>()
                };
                respond(
                    &mut writer,
                    id,
                    json!({"isIncomplete": false, "items": values}),
                )?;
            }
            "textDocument/hover" => {
                let uri = text_document_uri(&params)?;
                let line = params
                    .pointer("/position/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                let value = documents.get(&uri).map(|document| {
                    if let Some(manifest_ref) = project_index.semantic_package_manifest(&uri) {
                        return match sem_lang_tooling::semantic_package::compile(
                            manifest_ref,
                            &document.text,
                        ) {
                            Ok(package) => json!({"contents": {"kind": "markdown", "value": format!(
                                "Semantic package `{}` at version `{}`",
                                package.package_ref, package.version
                            )}}),
                            Err(error) => json!({"contents": {"kind": "markdown", "value": error}}),
                        };
                    }
                    if let Some(selection) = project_index.semantic_module_source(&uri) {
                        return match semantic_module::compile(
                            &selection.source_identity,
                            &selection.source,
                            &document.text,
                            SCHEMA_VERSION,
                        ) {
                            Ok(module) => json!({"contents": {"kind": "markdown", "value": format!(
                                "Semantic module `{}` at revision `{}`",
                                module.module_ref, module.revision
                            )}}),
                            Err(error) => json!({"contents": {"kind": "markdown", "value": error}}),
                        };
                    }
                    if let Some(selection) = project_index.language_source(&uri) {
                        return match sem_lang_vocab::VocabularyModule::from_package_sem(
                            &selection.source_identity,
                            &selection.package_version,
                            &document.text,
                        ) {
                            Ok(language) => json!({"contents": {"kind": "markdown", "value": format!(
                                "Language `{}` at version `{}`",
                                language.vocabulary_id, language.version
                            )}}),
                            Err(error) => json!({"contents": {"kind": "markdown", "value": error.to_string()}}),
                        };
                    }
                    let analysis = if project_index.project_source(&uri) {
                        analyzer.analyze_project_source(&document.text).ok()
                    } else {
                        Some(analyzer.analyze(&document.text))
                    };
                    let messages = analysis
                        .as_ref()
                        .into_iter()
                        .flat_map(|analysis| &analysis.diagnostics)
                        .filter(|item| item.range.start_line == line)
                        .map(|item| format!("- `{}` {}", item.code, item.message))
                        .collect::<Vec<_>>();
                    json!({"contents": {"kind": "markdown", "value": if messages.is_empty() {
                        analysis.as_ref().map(|value| format!("Semantic revision `{}`", value.semantic_revision)).unwrap_or_else(|| "Invalid project.sem source".into())
                    } else {
                        messages.join("\n")
                    }}})
                });
                respond(&mut writer, id, value.unwrap_or(Value::Null))?;
            }
            "textDocument/definition" => {
                let uri = text_document_uri(&params)?;
                let line = params
                    .pointer("/position/line")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let character = params
                    .pointer("/position/character")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let result = documents
                    .get(&uri)
                    .map(|document| {
                        project_index.definition_locations(
                            &analyzer,
                            &uri,
                            &document.text,
                            line,
                            character,
                        )
                    })
                    .unwrap_or_default();
                respond(&mut writer, id, Value::Array(result))?;
            }
            "textDocument/documentSymbol" => {
                let uri = text_document_uri(&params)?;
                let symbols = documents
                    .get(&uri)
                    .map(|document| document_symbols(&document.text))
                    .unwrap_or_default();
                respond(&mut writer, id, Value::Array(symbols))?;
            }
            "textDocument/codeAction" => {
                let uri = text_document_uri(&params)?;
                let result = documents
                    .get(&uri)
                    .map(|document| {
                        let requested = params
                            .pointer("/context/diagnostics")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|diagnostic| {
                                let code = diagnostic.get("code")?.as_str()?;
                                let line =
                                    diagnostic.pointer("/range/start/line")?.as_u64()? as u32;
                                Some((code.to_owned(), line))
                            })
                            .collect::<Vec<_>>();
                        requested
                            .into_iter()
                            .filter(|(code, _)| is_semantic_fix_diagnostic(code))
                            .filter_map(|(code, line)| {
                                let request = if project_index.project_source(&uri) {
                                    analyzer.semantic_fix_request_project_source(
                                        &document.text,
                                        line,
                                        &code,
                                    )
                                } else {
                                    analyzer.semantic_fix_request(&document.text, line, &code)
                                };
                                request.map(|request| semantic_code_actions(&uri, &request))
                            })
                            .flatten()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                respond(&mut writer, id, Value::Array(result))?;
            }
            "semLang/fixContext" => {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or("missing uri")?;
                let code = params
                    .get("diagnosticCode")
                    .and_then(Value::as_str)
                    .ok_or("missing diagnosticCode")?;
                let line = params
                    .get("sourceLine")
                    .and_then(Value::as_u64)
                    .ok_or("missing sourceLine")? as u32;
                let expected_digest = params
                    .get("sourceDigest")
                    .and_then(Value::as_str)
                    .ok_or("missing sourceDigest")?;
                let request = documents.get(uri).and_then(|document| {
                    if project_index.project_source(uri) {
                        analyzer.semantic_fix_request_project_source(&document.text, line, code)
                    } else {
                        analyzer.semantic_fix_request(&document.text, line, code)
                    }
                });
                match request {
                    Some(request) if request.source_digest == expected_digest => {
                        respond(&mut writer, id, serde_json::to_value(request)?)?;
                    }
                    Some(_) => respond_error(
                        &mut writer,
                        id,
                        -32002,
                        "document changed; request a new semantic quick fix",
                    )?,
                    None => respond_error(
                        &mut writer,
                        id,
                        -32003,
                        "semantic diagnostic is no longer present",
                    )?,
                }
            }
            "semLang/graph" => {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .or_else(|| params.pointer("/textDocument/uri").and_then(Value::as_str));
                let result = uri
                    .filter(|uri| project_index.declaration_source(uri).is_none())
                    .and_then(|uri| documents.get(uri).map(|document| (uri, document)))
                    .map(|(uri, document)| {
                        if project_index.project_source(uri) {
                            analyzer
                                .analyze_project_source(&document.text)
                                .and_then(|value| {
                                    serde_json::to_value(value).map_err(ToolingError::from)
                                })
                        } else {
                            serde_json::to_value(analyzer.analyze(&document.text))
                                .map_err(ToolingError::from)
                        }
                    })
                    .transpose()?
                    .unwrap_or(Value::Null);
                respond(&mut writer, id, result)?;
            }
            "semLang/contributionDraft" => {
                let surface = params
                    .get("surface")
                    .and_then(Value::as_str)
                    .ok_or("missing contribution surface")?;
                let kind_term = params
                    .get("kindTerm")
                    .and_then(Value::as_str)
                    .ok_or("missing contribution kind term")?;
                if surface.trim().is_empty()
                    || surface.trim() != surface
                    || surface.len() > 64 * 1024
                {
                    respond_error(&mut writer, id, -32020, "contribution surface is invalid")?;
                } else if analyzer.locale().is_empty() {
                    respond_error(
                        &mut writer,
                        id,
                        -32021,
                        "project language must be selected before drafting a contribution",
                    )?;
                } else if let Some(source) = analyzer.structure_contribution(kind_term, surface) {
                    let analysis = analyzer.analyze_contribution(&source);
                    respond(
                        &mut writer,
                        id,
                        json!({
                            "schema_version": "0.10.0",
                            "locale": analyzer.locale(),
                            "time_zone": analyzer.project_manifest().time_zone,
                            "kind_term": kind_term,
                            "surface": surface,
                            "source": source,
                            "source_digest": analysis.source_digest,
                            "analysis": analysis
                        }),
                    )?;
                } else {
                    respond_error(
                        &mut writer,
                        id,
                        -32022,
                        "contribution kind is not declared for the selected project language",
                    )?;
                }
            }
            "semLang/contributionKinds" => {
                respond(
                    &mut writer,
                    id,
                    serde_json::to_value(analyzer.contribution_structure_kinds())?,
                )?;
            }
            "semLang/workspaceSnapshot" => {
                // This is a read-only projection from the same validated
                // analyzer used by diagnostics.  It never scans a second
                // client-defined path and never persists or executes work.
                let source_versioning =
                    sem_lang_tooling::project::observe_source_versioning(&project)?;
                respond(&mut writer, id, analyzer.project_context(source_versioning))?;
            }
            "semLang/workspacePlanTask" => {
                match serde_json::from_value::<TaskRegistrationRequest>(params) {
                    Ok(request) => match plan_task_registration(&project, request) {
                        Ok(plan) => respond(&mut writer, id, serde_json::to_value(plan)?)?,
                        Err(error) => respond_tooling_error(&mut writer, id, -32010, &error)?,
                    },
                    Err(error) => respond_contract_error(&mut writer, id, -32602, &error)?,
                }
            }
            "semLang/workspaceApplyTask" => match params.get("plan").cloned() {
                Some(value) => match serde_json::from_value::<TaskRegistrationPlan>(value) {
                    Ok(plan) => match apply_task_registration(&project, &plan) {
                        Ok(receipt) => respond(&mut writer, id, serde_json::to_value(receipt)?)?,
                        Err(error) => respond_tooling_error(&mut writer, id, -32011, &error)?,
                    },
                    Err(error) => respond_contract_error(&mut writer, id, -32602, &error)?,
                },
                None => respond_issue(
                    &mut writer,
                    id,
                    -32602,
                    &request_issue("missing task registration plan"),
                )?,
            },
            "semLang/workspacePlanHistory" => {
                match serde_json::from_value::<HistoryRegistrationRequest>(params) {
                    Ok(request) => match plan_history_registration(&project, request) {
                        Ok(plan) => respond(&mut writer, id, serde_json::to_value(plan)?)?,
                        Err(error) => respond_tooling_error(&mut writer, id, -32012, &error)?,
                    },
                    Err(error) => respond_contract_error(&mut writer, id, -32602, &error)?,
                }
            }
            "semLang/workspaceApplyHistory" => match params.get("plan").cloned() {
                Some(value) => match serde_json::from_value::<HistoryRegistrationPlan>(value) {
                    Ok(plan) => match apply_history_registration(&project, &plan) {
                        Ok(receipt) => respond(&mut writer, id, serde_json::to_value(receipt)?)?,
                        Err(error) => respond_tooling_error(&mut writer, id, -32013, &error)?,
                    },
                    Err(error) => respond_contract_error(&mut writer, id, -32602, &error)?,
                },
                None => respond_issue(
                    &mut writer,
                    id,
                    -32602,
                    &request_issue("missing history registration plan"),
                )?,
            },
            _ if id.is_some() => respond_error(&mut writer, id, -32601, "method not found")?,
            _ => {}
        }
        if shutdown && method != "exit" {
            // LSP requires waiting for `exit`; keep the process alive but do no
            // additional semantic work after shutdown.
        }
    }
    Ok(())
}

fn completion_item(item: sem_lang_tooling::Completion) -> Value {
    let is_behavior = !item.behavior_bindings.is_empty();
    let documentation = item
        .behavior_bindings
        .iter()
        .map(|binding| {
            format!(
                "- `{}` provides `{}` through `{}` ({:?}, {:?})",
                binding.package_ref,
                binding.behavior_ref,
                binding.operation_id,
                binding.effect,
                binding.state
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "label": item.label,
        // Method mirrors a Java-style executable member; Keyword remains a
        // language-only semantic word.
        "kind": if is_behavior { 2 } else { 14 },
        "detail": item.detail,
        "insertText": item.insert_text,
        "sortText": format!(
            "{}:{}",
            if is_behavior { "0" } else { "1" },
            item.label
        ),
        "documentation": if documentation.is_empty() {
            Value::Null
        } else {
            json!({"kind": "markdown", "value": documentation})
        }
    })
}

/// Editor-only index over the exact files already admitted by the project.
/// It never discovers another directory and never treats a localized label as
/// an identity. Labels are used only to locate their selected declarations.
struct ProjectIndex {
    base: PathBuf,
    project_source: PathBuf,
    compilation_inputs: BTreeSet<PathBuf>,
    semantic_packages: BTreeMap<PathBuf, String>,
    language_sources: BTreeMap<PathBuf, sem_lang_tooling::semantic_package::SemanticPackageSource>,
    semantic_modules: BTreeMap<PathBuf, sem_lang_tooling::semantic_package::SemanticPackageSource>,
    package_declarations: BTreeMap<String, PackageDeclaration>,
}

struct PackageDeclaration {
    uri: String,
    source: String,
    selection_line: usize,
    selection_start: usize,
    selection_end: usize,
}

impl ProjectIndex {
    fn new(project: &Path, analyzer: &WorkspaceAnalyzer) -> Result<Self, io::Error> {
        let base = project
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "project has no parent"))?
            .canonicalize()?;
        let semantic_modules = analyzer
            .semantic_module_source_selections()
            .iter()
            .map(|selection| {
                let path = normalize_path(&base.join(&selection.source));
                (path, selection.clone())
            })
            .collect();
        let language_sources = analyzer
            .language_source_selections()
            .iter()
            .map(|selection| {
                let path = normalize_path(&base.join(&selection.source));
                (path, selection.clone())
            })
            .collect();
        let semantic_packages = analyzer
            .project_manifest()
            .semantic_packages
            .iter()
            .map(|selection| {
                (
                    normalize_path(&base.join(&selection.manifest)),
                    selection.manifest.clone(),
                )
            })
            .collect();
        let compilation_inputs = analyzer
            .project_compilation()
            .inputs
            .iter()
            .map(|input| normalize_path(&base.join(&input.path)))
            .collect();
        let mut package_declarations = analyzer
            .semantic_package_presentation()
            .iter()
            .map(|package| {
                (
                    package.package_ref.clone(),
                    compiled_package_declaration(package),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let standard = analyzer.standard_language_set_presentation();
        package_declarations.insert(
            standard.package_ref.clone(),
            compiled_standard_declaration(
                &standard.package_ref,
                &standard.version,
                &standard.digest_sha256,
                &standard.locales,
                &standard.module_ids,
            ),
        );
        Ok(Self {
            base,
            project_source: normalize_path(project),
            compilation_inputs,
            semantic_packages,
            language_sources,
            semantic_modules,
            package_declarations,
        })
    }

    fn semantic_package_manifest(&self, uri: &str) -> Option<&str> {
        let path = normalize_path(&file_uri_path(uri)?);
        self.semantic_packages.get(&path).map(String::as_str)
    }

    fn language_source(
        &self,
        uri: &str,
    ) -> Option<&sem_lang_tooling::semantic_package::SemanticPackageSource> {
        let path = normalize_path(&file_uri_path(uri)?);
        self.language_sources.get(&path)
    }

    fn project_source(&self, uri: &str) -> bool {
        file_uri_path(uri)
            .map(|path| normalize_path(&path) == self.project_source)
            .unwrap_or(false)
    }

    fn compilation_input(&self, uri: &str) -> bool {
        file_uri_path(uri)
            .map(|path| self.compilation_inputs.contains(&normalize_path(&path)))
            .unwrap_or(false)
    }

    fn declaration_source(&self, uri: &str) -> Option<&str> {
        self.semantic_package_manifest(uri)
            .or_else(|| {
                self.language_source(uri)
                    .map(|source| source.source.as_str())
            })
            .or_else(|| {
                self.semantic_module_source(uri)
                    .map(|source| source.source.as_str())
            })
    }

    fn semantic_module_source(
        &self,
        uri: &str,
    ) -> Option<&sem_lang_tooling::semantic_package::SemanticPackageSource> {
        let path = normalize_path(&file_uri_path(uri)?);
        self.semantic_modules.get(&path)
    }

    fn definition_locations(
        &self,
        analyzer: &WorkspaceAnalyzer,
        uri: &str,
        source: &str,
        line: usize,
        character: usize,
    ) -> Vec<Value> {
        if self.semantic_package_manifest(uri).is_some()
            && let Some(package_ref) = package_dependency_at_position(source, line, character)
            && let Some(declaration) = self.package_declarations.get(package_ref)
        {
            return vec![declaration.location()];
        }
        let Some(line_text) = source.lines().nth(line) else {
            return Vec::new();
        };
        let completions = analyzer.completions();
        let mut surfaces = completions
            .iter()
            .map(|item| item.label.as_str())
            .chain(
                analyzer
                    .semantic_module_presentation()
                    .iter()
                    .flat_map(|module| {
                        module
                            .subjects
                            .iter()
                            .flat_map(|value| {
                                [
                                    value.presentation_label.as_str(),
                                    value.subject_ref.as_str(),
                                ]
                            })
                            .chain(module.resources.iter().flat_map(|value| {
                                [
                                    value.presentation_label.as_str(),
                                    value.resource_ref.as_str(),
                                ]
                            }))
                    }),
            )
            .filter(|surface| !surface.is_empty())
            .collect::<Vec<_>>();
        surfaces.sort_by_key(|surface| std::cmp::Reverse(surface.chars().count()));
        surfaces.dedup();
        let Some(surface) = surfaces
            .into_iter()
            .find(|surface| surface_contains_position(line_text, surface, character))
        else {
            return Vec::new();
        };

        // Pipeline inputs navigate to the authority-owned value binding, not
        // to the lexical declaration which merely explains the word.  The
        // analyzer has already linked the binding to its exact producer and
        // current graph references.
        let mut value_targets = analyzer
            .analyze(source)
            .pipelines
            .into_iter()
            .flat_map(|pipeline| pipeline.steps)
            .filter(|step| step.operator == "source")
            .flat_map(|step| step.values)
            .filter(|value| value.source_line as usize == line)
            .flat_map(|value| value.bindings)
            .map(|binding| {
                let selection = binding.value_ref.clone();
                (
                    binding.definition_source_ref,
                    format!("識別子: {}", binding.value_ref),
                    selection,
                )
            })
            .collect::<Vec<_>>();
        value_targets.sort();
        value_targets.dedup();
        if !value_targets.is_empty() {
            return value_targets
                .into_iter()
                .flat_map(|(source_ref, query, selection)| {
                    self.locations_in_source(&source_ref, &query, &selection)
                })
                .collect();
        }

        let mut targets = Vec::<(String, String, String)>::new();
        if completions.iter().any(|item| item.label == surface) {
            for selection in analyzer.language_source_selections() {
                let needle = format!("表記: {surface}");
                if std::fs::read_to_string(self.base.join(&selection.source))
                    .is_ok_and(|source| source.lines().any(|line| line.trim() == needle))
                {
                    targets.push((selection.source.clone(), needle.clone(), surface.into()));
                }
            }
            for binding in completions
                .iter()
                .filter(|item| item.label == surface)
                .flat_map(|item| &item.behavior_bindings)
            {
                for selection in &analyzer.project_manifest().behavior_packages {
                    let path = self.base.join(&selection.manifest);
                    let matches_package = std::fs::read(&path)
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                        .and_then(|value| {
                            value
                                .get("package_ref")
                                .and_then(Value::as_str)
                                .map(|value| value == binding.package_ref)
                        })
                        .unwrap_or(false);
                    if matches_package {
                        targets.push((
                            selection.manifest.clone(),
                            format!(
                                "\"behavior_ref\": {}",
                                serde_json::to_string(&binding.behavior_ref)
                                    .expect("a behavior reference is serializable")
                            ),
                            binding.behavior_ref.clone(),
                        ));
                    }
                }
            }
        }
        for module in analyzer.semantic_module_presentation() {
            for (label, reference) in module
                .subjects
                .iter()
                .map(|value| (&value.presentation_label, &value.subject_ref))
                .chain(
                    module
                        .resources
                        .iter()
                        .map(|value| (&value.presentation_label, &value.resource_ref)),
                )
            {
                if label == surface {
                    targets.push((
                        module.source_ref.clone(),
                        format!("表示名: {surface}"),
                        surface.into(),
                    ));
                } else if reference == surface {
                    targets.push((
                        module.source_ref.clone(),
                        format!("識別子: {surface}"),
                        surface.into(),
                    ));
                }
            }
        }
        targets.sort();
        targets.dedup();
        targets
            .into_iter()
            .flat_map(|(source_ref, query, selection)| {
                self.locations_in_source(&source_ref, &query, &selection)
            })
            .collect()
    }

    fn locations_in_source(&self, source_ref: &str, query: &str, selection: &str) -> Vec<Value> {
        let path = normalize_path(&self.base.join(source_ref));
        let Ok(source) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        let uri = path_to_file_uri(&path);
        source
            .lines()
            .enumerate()
            .filter_map(|(line, text)| {
                let query_start = text.find(query)?;
                let selection_offset = text[query_start..].find(selection)?;
                let start = query_start + selection_offset;
                let start_character = text[..start].encode_utf16().count();
                let end_character = start_character + selection.encode_utf16().count();
                Some(json!({
                    "uri": uri,
                    "range": {
                        "start": {"line": line, "character": start_character},
                        "end": {"line": line, "character": end_character}
                    }
                }))
            })
            .take(32)
            .collect()
    }
}

impl PackageDeclaration {
    fn location(&self) -> Value {
        json!({
            "uri": self.uri,
            "range": {
                "start": {"line": self.selection_line, "character": self.selection_start},
                "end": {"line": self.selection_line, "character": self.selection_end}
            },
            "virtualDocument": {
                "languageId": "sem-lang",
                "content": self.source
            }
        })
    }
}

fn compiled_package_declaration(
    package: &sem_lang_tooling::semantic_package::SemanticPackage,
) -> PackageDeclaration {
    let mut lines = vec![
        "// Compiled semantic package declaration (read-only)".to_owned(),
        "// Generated from the exact package selected by project.sem.".to_owned(),
        String::new(),
        format!("package {}", package.package_ref),
        format!("  version: {}", package.version),
        format!("  owner: {}", package.owner_ref),
        format!("  digest: {}", package.manifest_digest),
    ];
    for dependency in &package.dependencies {
        lines.push(format!("  requires: {dependency}"));
    }
    for source in &package.language_sources {
        lines.push(format!(
            "  language: {} @ {}",
            source.source_identity, source.digest_sha256
        ));
    }
    for source in &package.semantic_modules {
        lines.push(format!(
            "  module: {} @ {}",
            source.source_identity, source.digest_sha256
        ));
    }
    lines.push(String::new());
    PackageDeclaration {
        uri: format!(
            "sem-lang-package:/{}/{}.sem",
            package.package_ref, package.manifest_digest
        ),
        source: lines.join("\n"),
        selection_line: 3,
        selection_start: "package ".len(),
        selection_end: package.package_ref.encode_utf16().count() + "package ".len(),
    }
}

fn compiled_standard_declaration(
    package_ref: &str,
    version: &str,
    digest_sha256: &str,
    locales: &[String],
    module_ids: &[String],
) -> PackageDeclaration {
    let mut lines = vec![
        "// Compiled semantic package declaration (read-only)".to_owned(),
        "// Bundled with this sem-lang release; projects cannot edit it.".to_owned(),
        String::new(),
        format!("package {package_ref}"),
        format!("  version: {version}"),
        format!("  digest: {digest_sha256}"),
    ];
    for locale in locales {
        lines.push(format!("  locale: {locale}"));
    }
    for module in module_ids {
        lines.push(format!("  module: {module}"));
    }
    lines.push(String::new());
    PackageDeclaration {
        uri: format!("sem-lang-package:/{}/{}.sem", package_ref, digest_sha256),
        source: lines.join("\n"),
        selection_line: 3,
        selection_start: "package ".len(),
        selection_end: package_ref.encode_utf16().count() + "package ".len(),
    }
}

fn package_dependency_at_position(
    source: &str,
    target_line: usize,
    character: usize,
) -> Option<&str> {
    let mut dependency_section = false;
    for (line, text) in source.lines().enumerate() {
        if line > target_line {
            break;
        }
        let trimmed = text.trim();
        if let Some(heading) = trimmed.strip_prefix('#').map(str::trim) {
            dependency_section = matches!(heading, "依存パッケージ" | "Package dependency");
            continue;
        }
        if line != target_line || !dependency_section {
            continue;
        }
        let (key, value) = trimmed.split_once(':')?;
        if !matches!(key.trim(), "識別子" | "Identifier") {
            return None;
        }
        let package_ref = value.trim();
        return surface_contains_position(text, package_ref, character).then_some(package_ref);
    }
    None
}

fn surface_contains_position(line: &str, surface: &str, character: usize) -> bool {
    line.match_indices(surface).any(|(start, _)| {
        let start_character = line[..start].encode_utf16().count();
        let end_character = start_character + surface.encode_utf16().count();
        start_character <= character && character <= end_character
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut value = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                value.pop();
            }
            other => value.push(other.as_os_str()),
        }
    }
    value
}

fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri
        .strip_prefix("file://localhost")
        .or_else(|| uri.strip_prefix("file://"))?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex(bytes[index + 1])?;
            let low = hex(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn path_to_file_uri(path: &Path) -> String {
    let mut value = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'.' | b'_' | b'~' => {
                value.push(*byte as char)
            }
            _ => value.push_str(&format!("%{byte:02X}")),
        }
    }
    value
}

#[derive(Debug)]
struct Document {
    text: String,
    version: Option<i64>,
}

fn project_argument() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    while let Some(value) = args.next() {
        if value == "--project" {
            return args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "missing --project value".into());
        }
    }
    Err("usage: sem-lang-language-server --project <project.sem>".into())
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid content length")
            })?);
        }
    }
    let length = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing content length"))?;
    if length > 8 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message is too large",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_message(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn respond(writer: &mut impl Write, id: Option<Value>, result: Value) -> io::Result<()> {
    if let Some(id) = id {
        write_message(
            writer,
            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
        )
    } else {
        Ok(())
    }
}

fn respond_error(
    writer: &mut impl Write,
    id: Option<Value>,
    code: i32,
    message: &str,
) -> io::Result<()> {
    if let Some(id) = id {
        write_message(
            writer,
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        )
    } else {
        Ok(())
    }
}

fn request_issue(detail: impl Into<String>) -> ToolingIssue {
    ToolingIssue {
        code: "language-server-request-invalid".into(),
        class: "contract".into(),
        owner: "sem-lang/language-server".into(),
        recovery: "caller-action".into(),
        detail: detail.into(),
    }
}

fn respond_contract_error(
    writer: &mut impl Write,
    id: Option<Value>,
    code: i32,
    error: &serde_json::Error,
) -> io::Result<()> {
    respond_issue(writer, id, code, &request_issue(error.to_string()))
}

fn respond_tooling_error(
    writer: &mut impl Write,
    id: Option<Value>,
    code: i32,
    error: &ToolingError,
) -> io::Result<()> {
    respond_issue(writer, id, code, &error.issue())
}

fn respond_issue(
    writer: &mut impl Write,
    id: Option<Value>,
    code: i32,
    issue: &ToolingIssue,
) -> io::Result<()> {
    if let Some(id) = id {
        write_message(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": issue.detail, "data": issue}
            }),
        )
    } else {
        Ok(())
    }
}

fn notify(writer: &mut impl Write, method: &str, params: Value) -> io::Result<()> {
    write_message(
        writer,
        &json!({"jsonrpc": "2.0", "method": method, "params": params}),
    )
}

fn publish(
    writer: &mut impl Write,
    analyzer: &WorkspaceAnalyzer,
    project_index: &ProjectIndex,
    uri: &str,
    document: &Document,
) -> io::Result<()> {
    let diagnostics = if project_index.project_source(uri) {
        match analyzer.analyze_project_source(&document.text) {
            Ok(analysis) => lsp_diagnostics(&analysis),
            Err(error) => source_error_diagnostic(
                &document.text,
                &error.to_string(),
                "SEM5003",
                "sem-lang/project-source",
                "correct-project-source",
            ),
        }
    } else if let Some(manifest_ref) = project_index.semantic_package_manifest(uri) {
        semantic_package_diagnostics(manifest_ref, &document.text)
    } else if let Some(selection) = project_index.language_source(uri) {
        language_source_diagnostics(selection, &document.text)
    } else {
        match project_index.semantic_module_source(uri) {
            Some(selection) => semantic_module_diagnostics(selection, &document.text),
            None => lsp_diagnostics(&analyzer.analyze(&document.text)),
        }
    };
    notify(
        writer,
        "textDocument/publishDiagnostics",
        json!({"uri": uri, "version": document.version, "diagnostics": diagnostics}),
    )
}

fn semantic_package_diagnostics(manifest_ref: &str, source: &str) -> Vec<Value> {
    let result =
        sem_lang_tooling::semantic_package::compile(manifest_ref, source).and_then(|package| {
            if package.schema_version == SCHEMA_VERSION && package.version == SCHEMA_VERSION {
                Ok(())
            } else {
                Err(format!(
                    "semantic package version must exactly match {SCHEMA_VERSION}"
                ))
            }
        });
    let Err(error) = result else {
        return Vec::new();
    };
    source_error_diagnostic(
        source,
        &error,
        "SEM5004",
        "sem-lang/semantic-package",
        "correct-semantic-package",
    )
}

fn language_source_diagnostics(
    selection: &sem_lang_tooling::semantic_package::SemanticPackageSource,
    source: &str,
) -> Vec<Value> {
    let Err(error) = sem_lang_vocab::VocabularyModule::from_package_sem(
        &selection.source_identity,
        &selection.package_version,
        source,
    ) else {
        return Vec::new();
    };
    source_error_diagnostic(
        source,
        &error.to_string(),
        "SEM5002",
        "sem-lang/language-source",
        "correct-language-source",
    )
}

fn semantic_module_diagnostics(
    selection: &sem_lang_tooling::semantic_package::SemanticPackageSource,
    source: &str,
) -> Vec<Value> {
    let Err(error) = semantic_module::compile(
        &selection.source_identity,
        &selection.source,
        source,
        SCHEMA_VERSION,
    ) else {
        return Vec::new();
    };
    source_error_diagnostic(
        source,
        &error,
        "SEM5001",
        "sem-lang/semantic-module",
        "correct-semantic-module",
    )
}

fn source_error_diagnostic(
    source: &str,
    error: &str,
    code: &str,
    owner: &str,
    recovery: &str,
) -> Vec<Value> {
    let line = error
        .split("line ")
        .nth(1)
        .and_then(|value| {
            value
                .split(|character: char| !character.is_ascii_digit())
                .next()
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .saturating_sub(1);
    let end_character = source
        .lines()
        .nth(line)
        .map(|value| value.encode_utf16().count())
        .unwrap_or(0);
    vec![json!({
        "range": {
            "start": {"line": line, "character": 0},
            "end": {"line": line, "character": end_character}
        },
        "severity": 1,
        "code": code,
        "source": owner,
        "message": error,
        "data": {
            "owner": owner,
            "recovery": recovery
        }
    })]
}

fn lsp_diagnostics(analysis: &Analysis) -> Vec<Value> {
    analysis
        .diagnostics
        .iter()
        .map(|item| {
            let severity = match item.severity {
                Severity::Error => 1,
                Severity::Warning => 2,
                Severity::Information => 3,
            };
            json!({
                "range": {
                    "start": {"line": item.range.start_line, "character": item.range.start_character},
                    "end": {"line": item.range.end_line, "character": item.range.end_character}
                },
                "severity": severity,
                "code": item.code,
                "source": "sem-lang",
                "message": item.message,
                "data": {"owner": item.owner, "recovery": item.recovery}
            })
        })
        .collect()
}

fn document_symbols(source: &str) -> Vec<Value> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let end = line.chars().count();
            let trimmed = line.trim();
            let section = trimmed.strip_prefix('#').map(str::trim);
            json!({
                "name": section.filter(|value| !value.is_empty()).unwrap_or(trimmed),
                "kind": if section.is_some() { 3 } else { 19 },
                "range": {"start": {"line": index, "character": 0}, "end": {"line": index, "character": end}},
                "selectionRange": {"start": {"line": index, "character": 0}, "end": {"line": index, "character": end}}
            })
        })
        .collect()
}

fn text_document_uri(params: &Value) -> Result<String, Box<dyn std::error::Error>> {
    string(
        params.get("textDocument").ok_or("missing textDocument")?,
        "uri",
    )
}

fn string(value: &Value, key: &str) -> Result<String, Box<dyn std::error::Error>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing {key}").into())
}

fn is_semantic_fix_diagnostic(code: &str) -> bool {
    matches!(code, "SEM1001" | "SEM1002" | "SEM1003")
}

fn semantic_code_actions(uri: &str, request: &SemanticFixRequest) -> Vec<Value> {
    let (review_title, inference_title) = if request.locale.to_ascii_lowercase().starts_with("ja") {
        ("未解決の意味根拠を確認", "推論で意味の解決候補を作成")
    } else {
        (
            "Review unresolved semantic evidence",
            "Use inference to propose a semantic resolution",
        )
    };
    let locator = json!({
        "schemaVersion": "0.10.0",
        "uri": uri,
        "diagnosticCode": request.diagnostic_code,
        "sourceLine": request.source_line,
        "sourceDigest": request.source_digest,
        "semanticRevision": request.semantic_revision
    });
    vec![
        json!({
            "title": review_title,
            "kind": "quickfix",
            "command": {
                "title": review_title,
                "command": "semLang.reviewSemanticDiagnostic",
                "arguments": [locator.clone()]
            }
        }),
        json!({
            "title": inference_title,
            "kind": "quickfix",
            "command": {
                "title": inference_title,
                "command": "semLang.proposeSemanticFix",
                "arguments": [locator]
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_round_trip() {
        let value = json!({"jsonrpc":"2.0","method":"initialized"});
        let mut bytes = Vec::new();
        write_message(&mut bytes, &value).unwrap();
        let mut reader = BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(value));
    }

    #[test]
    fn rejects_unbounded_message_before_allocation() {
        let input = b"Content-Length: 9000000\r\n\r\n";
        let mut reader = BufReader::new(input.as_slice());
        assert_eq!(
            read_message(&mut reader).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn semantic_code_actions_carry_an_exact_stale_safe_locator() {
        let request = SemanticFixRequest {
            schema_version: "0.10.0".into(),
            diagnostic_code: "SEM1003".into(),
            owner: "sem-lang/language".into(),
            recovery: "select-or-extend-definition".into(),
            locale: "ja-JP".into(),
            semantic_revision: "definitions-1".into(),
            source_digest: "source-1".into(),
            source_line: 4,
            instruction: "instruction".into(),
            unresolved: Vec::new(),
            resolved_terms: Vec::new(),
            allowed_terms: Vec::new(),
            allowed_terms_complete: true,
        };
        let actions = semantic_code_actions("file:///instruction.sem", &request);
        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[1].get("title").and_then(Value::as_str),
            Some("推論で意味の解決候補を作成")
        );
        assert_eq!(
            actions[1]
                .pointer("/command/command")
                .and_then(Value::as_str),
            Some("semLang.proposeSemanticFix")
        );
        assert_eq!(
            actions[1]
                .pointer("/command/arguments/0/sourceDigest")
                .and_then(Value::as_str),
            Some("source-1")
        );
    }

    #[test]
    fn semantic_module_source_is_not_analyzed_as_an_instruction() {
        let selection = sem_lang_tooling::semantic_package::SemanticPackageSource {
            package_ref: "example/work".into(),
            source_identity: "example/work/modules/example".into(),
            package_version: "0.10.0".into(),
            source: "packages/example/work/modules/example.sem".into(),
            digest_sha256: "a".repeat(64),
        };
        let source = "# 意味モジュール\n  所有者: subject/example\n\n# 主体\n  識別子: subject/example\n  種別: kind/person\n  表示名: 例示太郎\n  言語: ja-JP\n  公開範囲: private\n  正本: identity/example\n  正本の版: revision/example/1\n  根拠: evidence/example\n";
        assert!(semantic_module_diagnostics(&selection, source).is_empty());
        let invalid = source.replace("  表示名: 例示太郎", "  不明な欄: 例示太郎");
        let diagnostics = semantic_module_diagnostics(&selection, &invalid);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].get("code").and_then(Value::as_str),
            Some("SEM5001")
        );
    }

    #[test]
    fn semantic_package_manifest_is_not_analyzed_as_an_instruction() {
        let source = "# 意味パッケージ\n  版: 0.10.0\n  所有者: organization/example\n\n# 言語ソース\n  ソース: language/work.sem\n  ダイジェスト: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        assert!(
            semantic_package_diagnostics("packages/example/work/package.sem", source).is_empty()
        );
        let diagnostics = semantic_package_diagnostics(
            "packages/example/work/package.sem",
            &source.replace("所有者:", "不明な欄:"),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].get("code").and_then(Value::as_str),
            Some("SEM5004")
        );
    }

    #[test]
    fn language_source_is_compiled_as_a_definition_not_an_instruction() {
        let selection = sem_lang_tooling::semantic_package::SemanticPackageSource {
            package_ref: "example/work".into(),
            source_identity: "example/work/language/work".into(),
            package_version: "0.10.0".into(),
            source: "packages/example/work/language/work.sem".into(),
            digest_sha256: "a".repeat(64),
        };
        let source = "# 言語\n  依存: なし\n\n# 用語\n  名称: relation\n  説明: relation\n  意味: Primitive/Relation\n";
        assert!(language_source_diagnostics(&selection, source).is_empty());
        let diagnostics =
            language_source_diagnostics(&selection, &source.replace("意味:", "不明:"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].get("code").and_then(Value::as_str),
            Some("SEM5002")
        );
    }

    #[test]
    fn definition_surface_position_uses_lsp_utf16_offsets() {
        assert!(surface_contains_position(
            "    月別証跡分類",
            "月別証跡分類",
            5
        ));
        assert!(!surface_contains_position(
            "    月別証跡分類",
            "月別証跡分類",
            2
        ));
        assert!(surface_contains_position(
            "😀月別証跡分類",
            "月別証跡分類",
            3
        ));
    }

    #[test]
    fn package_dependency_is_a_definition_only_inside_its_exact_section() {
        let source = "# 意味パッケージ\n  版: 0.10.0\n  所有者: organization/example\n\n# 依存パッケージ\n  識別子: sem-lang/standard\n\n# 意味モジュール\n  ソース: modules/example.sem\n  ダイジェスト: aaaa\n";
        assert_eq!(
            package_dependency_at_position(source, 5, 12),
            Some("sem-lang/standard")
        );
        assert_eq!(package_dependency_at_position(source, 2, 12), None);
        assert_eq!(package_dependency_at_position(source, 8, 12), None);
    }

    #[test]
    fn compiled_package_declaration_is_exact_read_only_public_surface() {
        let package = sem_lang_tooling::semantic_package::SemanticPackage {
            schema_version: "0.10.0".into(),
            package_ref: "example/calendar".into(),
            version: "0.10.0".into(),
            owner_ref: "organization/example".into(),
            manifest_ref: "packages/example/calendar/package.sem".into(),
            manifest_digest: "a".repeat(64),
            dependencies: vec!["sem-lang/standard".into()],
            language_sources: vec![sem_lang_tooling::semantic_package::SemanticPackageSource {
                package_ref: "example/calendar".into(),
                source_identity: "example/calendar/language/reservation".into(),
                package_version: "0.10.0".into(),
                source: "language/reservation.sem".into(),
                digest_sha256: "b".repeat(64),
            }],
            semantic_modules: Vec::new(),
        };
        let declaration = compiled_package_declaration(&package);
        assert_eq!(
            declaration.uri,
            format!("sem-lang-package:/example/calendar/{}.sem", "a".repeat(64))
        );
        assert!(declaration.source.contains("package example/calendar"));
        assert!(declaration.source.contains("requires: sem-lang/standard"));
        assert!(
            declaration
                .source
                .contains("language: example/calendar/language/reservation")
        );
        assert!(!declaration.source.contains("language/reservation.sem"));
    }

    #[test]
    fn file_uri_round_trip_preserves_multilingual_paths() {
        let path = std::env::temp_dir().join("意味 定義.sem");
        assert_eq!(file_uri_path(&path_to_file_uri(&path)), Some(path));
    }
}
