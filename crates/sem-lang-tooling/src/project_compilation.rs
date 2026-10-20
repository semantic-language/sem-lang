//! Project compilation and input revision fences.

use super::*;

pub fn compile_project(
    project_file: impl AsRef<Path>,
) -> Result<(WorkspaceAnalyzer, ProjectCompilationReceipt), ToolingError> {
    let project_file = project_file.as_ref();
    for _ in 0..3 {
        let analyzer = WorkspaceAnalyzer::load(project_file)?;
        let graph_ref = analyzer
            .manifest
            .workspace_graph
            .as_deref()
            .ok_or_else(|| {
                ToolingError::Project(
                    "compiled project manifest is missing its derived workspace graph".into(),
                )
            })?;
        let base = project_directory(project_file)?;
        if !compilation_inputs_match(&base, &analyzer.compilation)? {
            continue;
        }
        let receipt =
            workspace_graph::publish_project_compilation(&base, graph_ref, &analyzer.compilation)?;
        if compilation_inputs_match(&base, &analyzer.compilation)? {
            return Ok((analyzer, receipt));
        }
    }
    Err(ToolingError::Project(
        "project inputs changed repeatedly during compilation".into(),
    ))
}

pub(super) struct ProjectCompilationContext<'a> {
    pub(super) base: &'a Path,
    pub(super) manifest: &'a ProjectManifest,
    pub(super) semantic_packages: &'a semantic_package::LoadedPackages,
    pub(super) standard_set: &'a sem_lang_standard::StandardLanguageSetDescriptor,
    pub(super) source_digest: &'a str,
    pub(super) semantic_revision: &'a str,
    pub(super) sources: &'a [ProjectSourceInput],
    pub(super) service_documents: &'a [ServiceDocument],
}

pub(super) fn build_project_compilation(
    context: ProjectCompilationContext<'_>,
) -> Result<ProjectCompilation, ToolingError> {
    let mut inputs = context
        .sources
        .iter()
        .map(|source| ProjectCompilationInput {
            kind: if source.document.path == project_source::PROJECT_SOURCE_PATH {
                "project_source".into()
            } else {
                "source_document".into()
            },
            path: source.document.path.clone(),
            digest_sha256: Digest::document_digest(source.authored.as_bytes()).to_hex(),
        })
        .collect::<Vec<_>>();
    inputs.push(ProjectCompilationInput {
        kind: "standard_language_set".into(),
        path: format!(
            "{}@{}",
            context.standard_set.package_ref, context.standard_set.version
        ),
        digest_sha256: context.standard_set.digest_sha256.clone(),
    });
    inputs.extend(context.semantic_packages.packages.iter().map(|package| {
        ProjectCompilationInput {
            kind: "semantic_package".into(),
            path: package.manifest_ref.clone(),
            digest_sha256: package.manifest_digest.clone(),
        }
    }));
    inputs.extend(
        context
            .semantic_packages
            .language_sources
            .iter()
            .map(|selection| ProjectCompilationInput {
                kind: "language_source".into(),
                path: selection.source.clone(),
                digest_sha256: selection.digest_sha256.clone(),
            }),
    );
    inputs.extend(context.manifest.behavior_packages.iter().map(|selection| {
        ProjectCompilationInput {
            kind: "behavior_package".into(),
            path: selection.manifest.clone(),
            digest_sha256: selection.digest_sha256.clone(),
        }
    }));
    inputs.extend(context.manifest.procedure_packages.iter().map(|selection| {
        ProjectCompilationInput {
            kind: "procedure_package".into(),
            path: selection.manifest.clone(),
            digest_sha256: selection.digest_sha256.clone(),
        }
    }));
    inputs.extend(
        context
            .semantic_packages
            .semantic_modules
            .iter()
            .map(|selection| ProjectCompilationInput {
                kind: "semantic_module".into(),
                path: selection.source.clone(),
                digest_sha256: selection.digest_sha256.clone(),
            }),
    );
    for document in context.service_documents {
        let path = document
            .path
            .strip_prefix(context.base)
            .map_err(|_| ToolingError::Project("service document escaped project root".into()))?;
        let path = path_identity::logical_path(path).map_err(ToolingError::Project)?;
        inputs.push(ProjectCompilationInput {
            kind: match document.kind {
                ServiceDocumentKind::Capability => "capability",
                ServiceDocumentKind::Authentication => "authentication",
                ServiceDocumentKind::State => {
                    return Err(ToolingError::Project(
                        "runtime service state cannot enter project compilation".into(),
                    ));
                }
            }
            .into(),
            path,
            digest_sha256: Digest::document_digest(&fs::read(&document.path)?).to_hex(),
        });
    }
    inputs.sort();
    if inputs
        .windows(2)
        .any(|pair| pair[0].kind == pair[1].kind && pair[0].path == pair[1].path)
    {
        return Err(ToolingError::Project(
            "project compilation contains a duplicate physical input".into(),
        ));
    }
    let mut compilation = ProjectCompilation {
        schema_version: SCHEMA_VERSION.into(),
        project_id: context.manifest.project_id.clone(),
        source_digest: context.source_digest.into(),
        semantic_revision: context.semantic_revision.into(),
        manifest: context.manifest.clone(),
        inputs,
        artifact_digest: String::new(),
    };
    compilation.artifact_digest = project_compilation_digest(&compilation)?;
    Ok(compilation)
}

pub(crate) fn project_compilation_digest(
    compilation: &ProjectCompilation,
) -> Result<String, ToolingError> {
    let mut canonical = compilation.clone();
    canonical.artifact_digest.clear();
    Ok(
        Digest::from_serializable(1, &("sem-lang/project/compilation", canonical))
            .map_err(|error| ToolingError::Project(error.to_string()))?
            .to_hex(),
    )
}

pub(super) fn compilation_inputs_match(
    base: &Path,
    compilation: &ProjectCompilation,
) -> Result<bool, ToolingError> {
    for input in &compilation.inputs {
        if input.kind == "standard_language_set" {
            let standard = sem_lang_standard::language_set()
                .map_err(|error| ToolingError::Vocabulary(error.to_string()))?;
            if input.path
                != format!(
                    "{}@{}",
                    standard.descriptor.package_ref, standard.descriptor.version
                )
                || input.digest_sha256 != standard.descriptor.digest_sha256
            {
                return Ok(false);
            }
            continue;
        }
        let path = resolve_project_file(
            base,
            &input.path,
            "project compilation input",
            MAX_PROJECT_DOCUMENT_BYTES,
        )?;
        if Digest::document_digest(&fs::read(path)?).to_hex() != input.digest_sha256 {
            return Ok(false);
        }
    }
    Ok(project_compilation_digest(compilation)? == compilation.artifact_digest)
}
