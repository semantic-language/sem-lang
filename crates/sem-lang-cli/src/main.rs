use clap::{Parser, Subcommand, ValueEnum};
mod binding;
mod diagnostic;
use sem_lang::prelude::*;
use sem_lang_codec::Codec;
use sem_lang_context::{ContextEngine, ContextStore};
use sem_lang_language::{CompiledLanguage, SemanticCompilation};
use sem_lang_linguistics::SemanticExpansionTrace;
use sem_lang_repository::{ObjectStore, Repository};
use sem_lang_store_fs::FsRepository;
use sem_lang_tooling::{
    SCHEMA_VERSION, WorkspaceAnalyzer,
    workspace_change::{
        HistoryRegistrationRequest, SourceBindingRequest, SourceMigrationRequest,
        TaskRegistrationRequest, WorkTrackingRequest, apply_history_registration,
        apply_source_binding, apply_source_migration, apply_task_registration, apply_work_tracking,
        plan_history_registration, plan_source_binding, plan_source_migration,
        plan_task_registration, plan_work_tracking,
    },
};
use serde_json::json;
use std::fmt::Display;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Parser)]
#[command(name = "sem-lang", version, about = "sem-lang CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    #[arg(long)]
    quiet: bool,
    #[arg(long)]
    verbose: bool,
    /// Optional additional language-definition source. The bundled sem-lang
    /// standard is always present and never copied into a project.
    #[arg(long, global = true)]
    language: Option<String>,
}

#[derive(ValueEnum, Clone)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        directory: String,
    },
    #[command(subcommand)]
    Project(ProjectCommand),
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    #[command(subcommand)]
    Vocab(VocabCommand),
    #[command(subcommand)]
    Linguistic(LinguisticCommand),
    #[command(subcommand)]
    Knowledge(KnowledgeCommand),
    #[command(subcommand)]
    Language(LanguageCommand),
    #[command(subcommand)]
    Definition(DefinitionCommand),
    #[command(subcommand)]
    Binding(binding::BindingCommand),
    #[command(subcommand)]
    Context(ContextCommand),
    #[command(subcommand)]
    Learning(LearningCommand),
    #[command(subcommand)]
    Surface(SurfaceCommand),
    Compile {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        restoration: String,
        #[arg(long)]
        observation_ref: String,
        #[arg(long, conflicts_with = "stdin", required_unless_present = "stdin")]
        text: Option<String>,
        #[arg(long, conflicts_with = "text")]
        stdin: bool,
        #[arg(long)]
        context: String,
    },
    #[command(subcommand)]
    Packet(PacketCommand),
    #[command(subcommand)]
    Extension(ExtensionCommand),
    #[command(subcommand)]
    Repository(RepoCommand),
    Digest {
        file: String,
    },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Create a new project with isolated service and authentication boundaries.
    Init {
        directory: String,
        #[arg(long)]
        project_id: String,
        #[arg(long, default_value = "en-US")]
        locale: String,
    },
    /// Explicitly create the graph selected by an existing project.sem and
    /// publish its first exact compilation. Normal compilation never creates
    /// missing storage.
    InitStorage {
        #[arg(long)]
        project: String,
    },
    /// Compile project.sem and publish its exact derived artifact to the
    /// project graph. An unchanged source returns the existing receipt.
    Compile {
        #[arg(long)]
        project: String,
    },
    /// Observe the explicitly selected local version-control source. Absence
    /// remains a successful, non-versioned project state.
    ObserveSource {
        #[arg(long)]
        project: String,
    },
    /// Explicitly retire only rebuildable compilation artifacts before a
    /// project source-contract migration. Workspace data is preserved.
    RetireCompilation {
        #[arg(long)]
        project: String,
    },
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Return the complete validated read-only project context for editor and
    /// coding-agent adapters. This never executes capabilities or mutates state.
    Context {
        #[arg(long)]
        project: String,
    },
    /// Inspect exact workspace documents and one instruction surface without
    /// applying changes or executing capabilities.
    Inspect {
        #[arg(long)]
        project: String,
        #[arg(long)]
        source: String,
    },
    /// Explicitly bind an existing graph record to one authored `.sem` line.
    BindSource {
        #[arg(long)]
        project: String,
        #[arg(long)]
        record_ref: String,
        #[arg(long)]
        source_ref: String,
        /// One-based source line.
        #[arg(long)]
        line: u32,
        /// Optional one-based inclusive end line. Defaults to `line`.
        #[arg(long)]
        end_line: Option<u32>,
    },
    /// Atomically move existing graph records to an explicitly reviewed source layout.
    MigrateSources {
        #[arg(long)]
        project: String,
        /// JSON SourceMigrationRequest. The file is an operation input, not project state.
        #[arg(long)]
        request: String,
    },
    /// Register one source-bound activity through an immutable plan and CAS commit.
    RegisterTask {
        #[arg(long)]
        project: String,
        /// JSON TaskRegistrationRequest. The file is operation input, not project state.
        #[arg(long)]
        request: String,
    },
    /// Append one immutable, source-bound completed-work outcome.
    RegisterHistory {
        #[arg(long)]
        project: String,
        /// JSON HistoryRegistrationRequest. The file is operation input, not project state.
        #[arg(long)]
        request: String,
    },
    /// Atomically replace the complete typed project-work tracking projection.
    ReplaceTracking {
        #[arg(long)]
        project: String,
        /// JSON WorkTrackingRequest. The file is operation input, not project state.
        #[arg(long)]
        request: String,
    },
}

#[derive(Subcommand)]
enum VocabCommand {
    Validate {
        file: String,
    },
    Import {
        file: String,
        #[arg(long)]
        repo: String,
    },
}

#[derive(Subcommand)]
enum LinguisticCommand {
    #[command(subcommand)]
    Kernel(KernelCommand),
}

#[derive(Subcommand)]
enum KernelCommand {
    Show,
    Validate,
}

#[derive(Subcommand)]
enum KnowledgeCommand {
    List,
    Show { id: String },
}

#[derive(Subcommand)]
enum LanguageCommand {
    Build,
    Inspect,
    Explain {
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "ja-JP")]
        locale: String,
        #[arg(long)]
        expand: bool,
    },
    Normalize {
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "ja-JP")]
        locale: String,
    },
}

#[derive(Subcommand)]
enum DefinitionCommand {
    Adopt {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        namespace: String,
        #[arg(long, required = true, num_args = 1..)]
        file: Vec<String>,
        #[arg(long)]
        decision_ref: String,
        #[arg(long)]
        expected_revision: Option<String>,
        #[arg(long)]
        expected_commit_revision: String,
    },
    Read {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        namespace: String,
    },
    Show {
        term: String,
    },
    Expand {
        term: String,
    },
    Trace {
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "ja-JP")]
        locale: String,
    },
}

#[derive(Subcommand)]
enum ContextCommand {
    Init {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Show {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Defaults {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Apply {
        #[arg(long)]
        request_json: String,
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    ShortTerm {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Candidates {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Learn {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Promote {
        #[arg(long)]
        candidate_file: String,
        #[arg(long)]
        request: String,
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    History {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
}

#[derive(Subcommand)]
enum LearningCommand {
    Run {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
    Report {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
}

#[derive(Subcommand)]
enum SurfaceCommand {
    Retention {
        #[arg(long, default_value = ".sem-lang-context.json")]
        repo: String,
    },
}

#[derive(Subcommand)]
enum PacketCommand {
    Validate {
        file: String,
    },
    Apply {
        file: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        head: String,
        #[arg(long)]
        restoration: String,
        #[arg(long)]
        request: String,
    },
}

#[derive(Subcommand)]
enum ExtensionCommand {
    Validate { file: String },
}

#[derive(Subcommand)]
enum RepoCommand {
    Head {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        name: String,
    },
    Verify {
        #[arg(long)]
        repo: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let json = matches!(cli.format, OutputFormat::Json);
    if let Err(err) = run(cli) {
        if json {
            eprintln!("{}", diagnostic::encode(err.as_ref()));
        } else {
            eprintln!("{err}");
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Init { directory } => {
            let repo = FsRepository::create(directory)?;
            emit(&cli.format, &json!({"opened": repo.version()}), cli.quiet);
        }
        Commands::Project(ProjectCommand::Init {
            directory,
            project_id,
            locale,
        }) => {
            let initialized =
                sem_lang_tooling::project::initialize(directory, &project_id, &locale)?;
            emit(&cli.format, &serde_json::to_value(initialized)?, cli.quiet);
        }
        Commands::Project(ProjectCommand::InitStorage { project }) => {
            let receipt = sem_lang_tooling::project::initialize_storage(project)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Project(ProjectCommand::Compile { project }) => {
            let (_, receipt) = sem_lang_tooling::compile_project(project)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Project(ProjectCommand::ObserveSource { project }) => {
            let observation = sem_lang_tooling::project::observe_source_versioning(project)?;
            emit(&cli.format, &serde_json::to_value(observation)?, cli.quiet);
        }
        Commands::Project(ProjectCommand::RetireCompilation { project }) => {
            let receipt = sem_lang_tooling::project::retire_compilation(project)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Workspace(WorkspaceCommand::Context { project }) => {
            let analyzer = WorkspaceAnalyzer::load(&project)?;
            let source_versioning = sem_lang_tooling::project::observe_source_versioning(&project)?;
            emit(
                &cli.format,
                &analyzer.project_context(source_versioning),
                cli.quiet,
            );
        }
        Commands::Workspace(WorkspaceCommand::Inspect { project, source }) => {
            let analyzer = WorkspaceAnalyzer::load(project)?;
            let analysis = analyzer.analyze(&fs::read_to_string(source)?);
            emit(&cli.format, &serde_json::to_value(analysis)?, cli.quiet);
        }
        Commands::Workspace(WorkspaceCommand::BindSource {
            project,
            record_ref,
            source_ref,
            line,
            end_line,
        }) => {
            let source_line = line
                .checked_sub(1)
                .ok_or("source line must be at least 1")?;
            let source_end_line = end_line
                .unwrap_or(line)
                .checked_sub(1)
                .ok_or("source end line must be at least 1")?;
            let project = Path::new(&project);
            let plan = plan_source_binding(
                project,
                SourceBindingRequest {
                    schema_version: SCHEMA_VERSION.into(),
                    record_ref,
                    source_ref,
                    start_line: source_line,
                    end_line: source_end_line,
                },
            )?;
            let receipt = apply_source_binding(project, &plan)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Workspace(WorkspaceCommand::MigrateSources { project, request }) => {
            let request: SourceMigrationRequest = serde_json::from_slice(&fs::read(request)?)?;
            let project = Path::new(&project);
            let plan = plan_source_migration(project, request)?;
            let receipt = apply_source_migration(project, &plan)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Workspace(WorkspaceCommand::RegisterTask { project, request }) => {
            let request: TaskRegistrationRequest = serde_json::from_slice(&fs::read(request)?)?;
            let project = Path::new(&project);
            let plan = plan_task_registration(project, request)?;
            let receipt = apply_task_registration(project, &plan)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Workspace(WorkspaceCommand::RegisterHistory { project, request }) => {
            let request: HistoryRegistrationRequest = serde_json::from_slice(&fs::read(request)?)?;
            let project = Path::new(&project);
            let plan = plan_history_registration(project, request)?;
            let receipt = apply_history_registration(project, &plan)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Workspace(WorkspaceCommand::ReplaceTracking { project, request }) => {
            let request: WorkTrackingRequest = serde_json::from_slice(&fs::read(request)?)?;
            let project = Path::new(&project);
            let plan = plan_work_tracking(project, request)?;
            let receipt = apply_work_tracking(project, &plan)?;
            emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
        }
        Commands::Vocab(cmd) => match cmd {
            VocabCommand::Validate { file } => {
                let raw = read_repo_file(file)?;
                let _ = VocabularyModule::from_sem(&raw)?;
                emit_success(&cli.format, "valid vocabulary", cli.quiet);
            }
            VocabCommand::Import { file, repo } => {
                let raw = read_repo_file(file)?;
                let _ = VocabularyModule::from_sem(&raw)?;
                let mut repository = FsRepository::create(repo)?;
                let id = repository.put_object(raw.into_bytes())?;
                emit(&cli.format, &json!({"imported": id}), cli.quiet);
            }
        },
        Commands::Linguistic(cmd) => match cmd {
            LinguisticCommand::Kernel(kernel) => match kernel {
                KernelCommand::Show => {
                    let kernel = KernelVocabulary::embedded();
                    let concepts: Vec<_> = kernel
                        .concepts
                        .iter()
                        .map(|concept| {
                            json!({
                                "path": concept.path,
                                "term_id": concept.term_id.to_hex()
                            })
                        })
                        .collect();
                    emit(
                        &cli.format,
                        &json!({"version": kernel.version, "concepts": concepts}),
                        cli.quiet,
                    );
                }
                KernelCommand::Validate => {
                    let kernel = KernelVocabulary::embedded();
                    let has_only_kernel_paths = kernel
                        .concepts
                        .iter()
                        .all(|concept| concept.path.starts_with("kernel."));
                    emit(
                        &cli.format,
                        &json!({
                            "valid": !kernel.concepts.is_empty() && has_only_kernel_paths,
                            "version": kernel.version,
                            "concept_count": kernel.concepts.len()
                        }),
                        cli.quiet,
                    );
                }
            },
        },
        Commands::Knowledge(cmd) => match cmd {
            KnowledgeCommand::List => {
                let knowledge = LinguisticKnowledge::new();
                emit(
                    &cli.format,
                    &json!({
                        "kernel": knowledge.kernel.entries.len(),
                        "curated": knowledge.curated.entries.len(),
                        "learned": knowledge.learned.entries.len()
                    }),
                    cli.quiet,
                );
            }
            KnowledgeCommand::Show { id } => {
                let knowledge = LinguisticKnowledge::new();
                let entry = knowledge
                    .all_entries()
                    .find(|entry| entry.id.to_string() == id)
                    .map(|entry| {
                        json!({
                            "id": entry.id.to_string(),
                            "origin": format!("{:?}", entry.origin),
                            "revision": entry.revision,
                            "confidence": entry.confidence
                        })
                    });
                emit(&cli.format, &json!({"entry": entry}), cli.quiet);
            }
        },
        Commands::Language(cmd) => match cmd {
            LanguageCommand::Build => {
                let language = external_language(&cli.language)?;
                emit(
                    &cli.format,
                    &json!({
                        "revision": language.revision,
                        "digest": language.digest.to_hex(),
                        "lexicon_entries": language.resources.lexicon.entries.len()
                    }),
                    cli.quiet,
                );
            }
            LanguageCommand::Inspect => {
                let language = external_language(&cli.language)?;
                emit(
                    &cli.format,
                    &json!({
                        "revision": language.revision,
                        "digest": language.digest.to_hex(),
                        "lexicon": language.resources.lexicon.entries.len(),
                        "constructions": language.resources.construction_set.constructions.len(),
                        "semantic_bindings": language.resources.semantic_bindings.bindings.len()
                    }),
                    cli.quiet,
                );
            }
            LanguageCommand::Explain {
                text,
                locale,
                expand,
            } => {
                let language = external_language(&cli.language)?;
                let explanation = language.explain(&text, &locale);
                let matches: Vec<_> = explanation
                    .matches
                    .iter()
                    .map(|item| {
                        json!({
                            "surface": item.surface,
                            "origin": format!("{:?}", item.origin),
                            "provenance": item.provenance,
                            "concept": item.concept.to_hex()
                        })
                    })
                    .collect();
                let mut result = json!({"text": explanation.text, "locale": explanation.locale, "matches": matches});
                if expand {
                    let compiled = language.compile_semantics(&text, &locale)?;
                    result["semantic"] = semantic_json(&compiled);
                }
                emit(&cli.format, &result, cli.quiet);
            }
            LanguageCommand::Normalize { text, locale } => {
                let language = external_language(&cli.language)?;
                let compiled = language.compile_semantics(&text, &locale)?;
                let result = semantic_json(&compiled);
                emit(&cli.format, &result, cli.quiet);
            }
        },
        Commands::Binding(cmd) => emit(&cli.format, &binding::run(cmd)?, cli.quiet),
        Commands::Definition(cmd) => {
            match cmd {
                DefinitionCommand::Adopt {
                    repo,
                    namespace,
                    file,
                    decision_ref,
                    expected_revision,
                    expected_commit_revision,
                } => {
                    let mut modules = Vec::with_capacity(file.len());
                    for source in file {
                        let mut bytes = Vec::new();
                        fs::File::open(source)?
                            .take(1_048_577)
                            .read_to_end(&mut bytes)?;
                        if bytes.len() > 1_048_576 {
                            return Err("definition input exceeds limit".into());
                        }
                        let text = std::str::from_utf8(&bytes)?;
                        modules.push(sem_lang_vocab::VocabularyModule::from_sem(text)?);
                    }
                    let mut repository = FsRepository::open_existing(repo)?;
                    let revision = sem_lang_memory::definitions::adopt(
                        &mut repository,
                        &namespace,
                        expected_revision.as_ref(),
                        &modules,
                        &decision_ref,
                        &serde_json::from_str(&expected_commit_revision)?,
                    )?;
                    emit(&cli.format, &json!({"revision": revision}), cli.quiet);
                }
                DefinitionCommand::Read { repo, namespace } => {
                    let repository = FsRepository::open_existing(repo)?;
                    let memory = sem_lang_memory::definitions::read(&repository, &namespace)?;
                    emit(
                        &cli.format,
                        &json!(memory.map(
                            |(revision, memory)| json!({"revision": revision, "memory": memory})
                        )),
                        cli.quiet,
                    );
                }
                DefinitionCommand::Show { term } => {
                    let language = external_language(&cli.language)?;
                    let definition = language
                        .definition(&term)
                        .map(|definition| format!("{definition:?}"));
                    emit(
                        &cli.format,
                        &json!({"term": term, "definition": definition}),
                        cli.quiet,
                    );
                }
                DefinitionCommand::Expand { term } => {
                    let language = external_language(&cli.language)?;
                    let target = language
                        .term(&term)
                        .ok_or_else(|| format!("unknown term: {term}"))?;
                    let trace = language.semantic_expander().expand(target)?;
                    emit(
                        &cli.format,
                        &json!({"term": term, "trace": trace_json(&trace)}),
                        cli.quiet,
                    );
                }
                DefinitionCommand::Trace { text, locale } => {
                    let language = external_language(&cli.language)?;
                    let compiled = language.compile_semantics(&text, &locale)?;
                    emit(&cli.format, &semantic_json(&compiled), cli.quiet);
                }
            }
        }
        Commands::Context(cmd) => {
            match cmd {
                ContextCommand::Init { repo } => {
                    ContextStore::create(&repo)?;
                    emit(
                        &cli.format,
                        &json!({"status":"context initialized"}),
                        cli.quiet,
                    );
                }
                ContextCommand::Show { repo } => {
                    let store = ContextStore::load(&repo)?;
                    emit(
                        &cli.format,
                        &json!({"defaults": store.persistent_defaults, "short_term": store.short_term.len(), "observations": store.observations.len(), "revisions": store.revisions.len()}),
                        cli.quiet,
                    );
                }
                ContextCommand::Defaults { repo } => {
                    let store = ContextStore::load(&repo)?;
                    emit(
                        &cli.format,
                        &json!({"defaults": store.persistent_defaults}),
                        cli.quiet,
                    );
                }
                ContextCommand::ShortTerm { repo } => {
                    let store = ContextStore::load(&repo)?;
                    let short_term_bytes = serde_json::to_vec(&store.short_term)?.len();
                    emit(
                        &cli.format,
                        &json!({"short_term": store.short_term, "serialized_bytes": short_term_bytes}),
                        cli.quiet,
                    );
                }
                ContextCommand::Apply { request_json, repo } => {
                    if request_json.len() > sem_lang_context::MAX_CONTEXT_BYTES {
                        return Err("context request exceeds capacity".into());
                    }
                    let request: sem_lang_context::ContextRequest =
                        serde_json::from_str(&request_json)?;
                    let receipt = ContextStore::publish(&repo, &request)?;
                    emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
                }
                ContextCommand::Candidates { repo } => {
                    let store = ContextStore::load(&repo)?;
                    let engine = ContextEngine::new(store);
                    let candidates = engine.induce_defaults().into_iter().map(|candidate| {
                    candidate.reference().map(|reference| json!({"candidate_ref": reference, "candidate": candidate}))
                }).collect::<Result<Vec<_>, _>>()?;
                    emit(
                        &cli.format,
                        &json!({"candidates": candidates, "memory_revision": engine.memory_revision()?, "commit_revision": engine.commit_revision()?}),
                        cli.quiet,
                    );
                }
                ContextCommand::Learn { repo } => {
                    let store = ContextStore::load(&repo)?;
                    emit(
                        &cli.format,
                        &json!({"status": "context observations retained", "observations": store.observations.len()}),
                        cli.quiet,
                    );
                }
                ContextCommand::Promote {
                    candidate_file,
                    request,
                    repo,
                } => {
                    let selected = serde_json::from_str(&read_repo_file(candidate_file)?)?;
                    let request = serde_json::from_str(&read_repo_file(request)?)?;
                    let receipt = ContextStore::publish_promotion(&repo, selected, &request)?;
                    emit(&cli.format, &serde_json::to_value(receipt)?, cli.quiet);
                }
                ContextCommand::History { repo } => {
                    let store = ContextStore::load(&repo)?;
                    emit(&cli.format, &json!({"history": store.revisions}), cli.quiet);
                }
            }
        }
        Commands::Learning(cmd) => match cmd {
            LearningCommand::Run { repo } => {
                let engine = ContextEngine::new(ContextStore::load(&repo)?);
                let candidates = engine.induce_defaults();
                emit(
                    &cli.format,
                    &json!({"candidates": candidates, "promoted": 0}),
                    cli.quiet,
                );
            }
            LearningCommand::Report { repo } => {
                let store = ContextStore::load(&repo)?;
                emit(
                    &cli.format,
                    &json!({"context_observations": store.observations.len(), "persistent_defaults": store.persistent_defaults.len(), "revisions": store.revisions.len(), "interpretation_rules": store.interpretation_rules.len()}),
                    cli.quiet,
                );
            }
        },
        Commands::Surface(cmd) => match cmd {
            SurfaceCommand::Retention { repo } => {
                let store = ContextStore::load(&repo)?;
                emit(
                    &cli.format,
                    &json!({"policy": {"complete": "DiscardAfterCompile", "unresolved": "KeepAsEvidence", "ambiguous": "KeepAsEvidence", "audit": "KeepForAudit"}, "short_term_records": store.short_term.len()}),
                    cli.quiet,
                );
            }
        },
        Commands::Compile {
            repo,
            restoration,
            observation_ref,
            text,
            stdin,
            context,
        } => {
            let restoration = serde_json::from_str(&read_repo_file(restoration)?)?;
            let mut engine = Engine::restore(FsRepository::open_existing(repo)?, restoration)?;
            let source = read_input(text, None, stdin)?;
            let context: Context = serde_json::from_str(&read_repo_file(context)?)?;
            // This is an explicit evaluation scope, not ownership/authentication
            // inferred from a process, profile, label or anonymous default.
            engine.context_mut().set_subject(context.subject.clone());
            let outcome =
                engine.compile_surface(&SurfaceInput::new(&observation_ref, &source), &context);
            emit(&cli.format, &serde_json::to_value(&outcome)?, cli.quiet);
            outcome?;
        }
        Commands::Packet(cmd) => match cmd {
            PacketCommand::Validate { file } => {
                let raw = read_repo_file(file)?;
                Codec::decode_packet(&raw)?;
                emit_success(&cli.format, "valid packet", cli.quiet);
            }
            PacketCommand::Apply {
                file,
                repo,
                head,
                restoration,
                request,
            } => {
                let packet = Codec::decode_packet(&read_repo_file(file)?)?;
                let input = serde_json::from_str(&read_repo_file(restoration)?)?;
                let request = serde_json::from_str(&read_repo_file(request)?)?;
                let result = sem_lang::restoration::apply(
                    &mut FsRepository::open_existing(repo)?,
                    &input,
                    &packet,
                    &request,
                    &head,
                )?;
                emit(&cli.format, &serde_json::to_value(result)?, cli.quiet);
            }
        },
        Commands::Extension(cmd) => match cmd {
            ExtensionCommand::Validate { file } => {
                let raw = read_repo_file(file)?;
                let _dto: serde_json::Value = serde_json::from_str(&raw)?;
                emit_success(&cli.format, "valid extension proposal", cli.quiet);
            }
        },
        Commands::Repository(cmd) => match cmd {
            RepoCommand::Head { repo, name } => {
                use sem_lang_repository::HeadStore;
                let repository = FsRepository::open_existing(repo)?;
                emit(
                    &cli.format,
                    &json!({"reference": repository.load_head(&name)?, "commit_revision": repository.head_revision(&name)?}),
                    cli.quiet,
                );
            }
            RepoCommand::Verify { repo } => {
                let repository = FsRepository::open_existing(repo)?;
                let report = repository.verify()?;
                emit(&cli.format, &serde_json::to_value(&report)?, cli.quiet);
            }
        },
        Commands::Digest { file } => {
            let raw = read_repo_file(file)?;
            emit(
                &cli.format,
                &json!({"sha": sem_lang_codec::PacketDigest::proposition(&Codec::decode_packet(&raw)?)?.to_hex()}),
                cli.quiet,
            );
        }
    }
    Ok(())
}

fn emit<T: serde::Serialize>(format: &OutputFormat, value: &T, quiet: bool) {
    if quiet {
        return;
    }
    match format {
        OutputFormat::Human => match serde_json::to_string_pretty(value) {
            Ok(text) => println!("{text}"),
            Err(err) => println!("{err}"),
        },
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string(value).unwrap_or_else(|err| format!("{err}"))
            )
        }
    }
}

fn emit_success(format: &OutputFormat, msg: impl Display, quiet: bool) {
    emit(format, &json!({"status": msg.to_string()}), quiet)
}

fn load_vocabulary(path: &str) -> Result<VocabularyModule, Box<dyn std::error::Error>> {
    let raw = read_repo_file(path)?;
    Ok(VocabularyModule::from_sem(&raw)?)
}

fn external_language(
    path: &Option<String>,
) -> Result<CompiledLanguage, Box<dyn std::error::Error>> {
    let mut registry = sem_lang_standard::registry()?;
    if let Some(path) = path {
        registry.register(load_vocabulary(path)?)?;
    }
    Ok(CompiledLanguage::embedded(&registry)?)
}

fn trace_json(trace: &SemanticExpansionTrace) -> serde_json::Value {
    json!({
        "source": trace.source.to_hex(),
        "quality": format!("{:?}", trace.quality),
        "stop": format!("{:?}", trace.stop),
        "result": trace.result.canonical(),
        "steps": trace.steps.iter().map(|step| json!({
            "from": step.from_label,
            "to": step.to_label,
            "from_id": step.from.to_hex(),
            "to_id": step.to.to_hex()
        })).collect::<Vec<_>>(),
        "residuals": trace.residuals.iter().map(|residual| format!("{residual:?}")).collect::<Vec<_>>()
    })
}

fn semantic_json(compiled: &SemanticCompilation) -> serde_json::Value {
    json!({
        "surface": compiled.surface,
        "lexeme": {"id": compiled.lexeme.id.to_string(), "surface": compiled.lexeme.surface, "locale": compiled.lexeme.locale},
        "sense": {"id": compiled.sense.id.to_string(), "target": compiled.sense.target_label},
        "concept": compiled.concept.to_hex(),
        "semantic_definition": compiled.definition.as_ref().map(|definition| format!("{definition:?}")),
        "expansion": trace_json(&compiled.trace),
        "normalized_ir": compiled.normalized.expr.canonical(),
        "proposition_digest": compiled.normalized.proposition_digest.to_hex(),
        "interaction_digest": compiled.normalized.interaction_digest.to_hex(),
        "semantic_residual": compiled.normalized.residuals.iter().map(|residual| format!("{residual:?}")).collect::<Vec<_>>(),
        "pragmatic_contribution": compiled.pragmatic,
        "interaction_act": compiled.interaction_act,
        "conditions": compiled.conditions,
        "complete_surface": compiled.complete_surface
    })
}

fn read_repo_file<P: AsRef<Path>>(path: P) -> io::Result<String> {
    fs::read_to_string(path)
}

fn read_input(
    text: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    match (text, file, stdin) {
        (Some(text), None, false) => Ok(text),
        (None, Some(file), false) => Ok(read_repo_file(file)?),
        (None, None, true) => {
            let mut buffer = String::new();
            io::stdin().read_to_string(&mut buffer)?;
            Ok(buffer)
        }
        _ => Err("one of --text, --file, or --stdin is required".into()),
    }
}
