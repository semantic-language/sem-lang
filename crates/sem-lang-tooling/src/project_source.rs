//! Canonical `project.sem` compiler.
//!
//! Project selection and the project program use the same section-oriented
//! `.sem` surface as definition and semantic-module sources. The typed project
//! sections are compiled into a content-addressed artifact; the first ordinary
//! semantic section begins the line-preserving project program.

use crate::{
    BehaviorPackageSelection, ProjectManifest, ProjectSourceDocument, ProjectSourceRole,
    ToolingError, VersionControlSelection, procedure::ProcedurePackageSelection,
    semantic_package::SemanticPackageSelection,
};
use std::path::{Component, Path};

pub const PROJECT_SOURCE_PATH: &str = "project.sem";
pub const WORKSPACE_GRAPH_PATH: &str = ".sem-lang/data/workspace.redb";
pub const SERVICES_PATH: &str = ".sem-lang/services";
pub const RUNTIME_SERVICES_PATH: &str = ".sem-lang/runtime/services";

#[derive(Debug, Clone)]
pub struct CompiledProjectSource {
    pub manifest: ProjectManifest,
    /// A line-preserving view containing only the authored project program.
    /// Typed project sections are blanked so diagnostics and graph locators
    /// retain their physical `project.sem` line numbers.
    pub analysis_source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Project,
    SemanticPackage,
    SourceDocument,
    BehaviorPackage,
    ProcedurePackage,
    VersionControl,
}

struct Record {
    kind: SectionKind,
    fields: Vec<(String, String)>,
}

pub fn compile(source: &str) -> Result<CompiledProjectSource, ToolingError> {
    if source.as_bytes().contains(&0) {
        return Err(invalid("project.sem contains a NUL byte"));
    }
    let mut records = Vec::<Record>::new();
    let mut current = None::<Record>;
    let mut program_line = source.lines().count();
    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#').map(str::trim) {
            if let Some(record) = current.take() {
                records.push(record);
            }
            let Some(kind) = section_kind(heading) else {
                program_line = index;
                break;
            };
            current = Some(Record {
                kind,
                fields: Vec::new(),
            });
            continue;
        }
        if !line.starts_with("  ") || line.starts_with("    ") {
            return Err(invalid(format!(
                "line {} must be a `# section` or a two-space-indented field",
                index + 1
            )));
        }
        let Some(record) = current.as_mut() else {
            return Err(invalid(format!(
                "line {} appears before a project section",
                index + 1
            )));
        };
        let Some((surface, value)) = trimmed.split_once(':') else {
            return Err(invalid(format!(
                "line {} must use `name: value`",
                index + 1
            )));
        };
        let name = field_key(record.kind, surface.trim()).ok_or_else(|| {
            invalid(format!(
                "line {} contains an unknown project field `{}`",
                index + 1,
                surface.trim()
            ))
        })?;
        let value = value.trim();
        let empty_locale = record.kind == SectionKind::Project && name == "locale";
        if (!empty_locale && value.is_empty()) || value.contains(['\r', '\n']) {
            return Err(invalid(format!(
                "line {} has an empty project field",
                index + 1
            )));
        }
        if record.fields.iter().any(|(existing, _)| existing == name) {
            return Err(invalid(format!(
                "line {} duplicates field `{name}`",
                index + 1
            )));
        }
        record.fields.push((name.into(), value.into()));
    }
    if let Some(record) = current.take() {
        records.push(record);
    }
    if program_line < source.lines().count() {
        for (offset, raw) in source.lines().skip(program_line + 1).enumerate() {
            let trimmed = raw.trim_end_matches('\r').trim();
            if let Some(heading) = trimmed.strip_prefix('#').map(str::trim)
                && section_kind(heading).is_some()
            {
                return Err(invalid(format!(
                    "line {} contains a typed project section after the semantic program began",
                    program_line + offset + 2
                )));
            }
        }
    }

    let project = take_single(&mut records, SectionKind::Project)?;
    exact_fields(
        &project,
        &["schema_version", "project_id", "locale"],
        &["time_zone"],
    )?;
    let package_records = take_many(&mut records, SectionKind::SemanticPackage, 0, 64)?;
    let mut semantic_packages = Vec::with_capacity(package_records.len());
    for package in package_records {
        exact_fields(&package, &["manifest", "digest_sha256"], &[])?;
        semantic_packages.push(SemanticPackageSelection {
            manifest: field(&package, "manifest")?.into(),
            digest_sha256: field(&package, "digest_sha256")?.into(),
        });
    }

    let mut source_documents = vec![ProjectSourceDocument {
        path: PROJECT_SOURCE_PATH.into(),
        role: ProjectSourceRole::ProjectLanguage,
    }];
    let mut behavior_packages = Vec::new();
    let mut procedure_packages = Vec::new();
    let version_control_records = take_many(&mut records, SectionKind::VersionControl, 0, 1)?;
    let version_control = version_control_records
        .into_iter()
        .next()
        .map(|record| {
            exact_fields(&record, &["provider", "root"], &[])?;
            let provider = field(&record, "provider")?.to_owned();
            if provider != "git" {
                return Err(invalid("version control provider must be `git`"));
            }
            let root = field(&record, "root")?.to_owned();
            validate_version_control_root(&root)?;
            Ok(VersionControlSelection { provider, root })
        })
        .transpose()?;
    for record in records {
        match record.kind {
            SectionKind::SourceDocument => {
                exact_fields(&record, &["path"], &[])?;
                let path = field(&record, "path")?.to_owned();
                if path == PROJECT_SOURCE_PATH {
                    return Err(invalid(
                        "project.sem is selected implicitly and cannot be listed again",
                    ));
                }
                let role = source_role(&path)?;
                source_documents.push(ProjectSourceDocument { path, role });
            }
            SectionKind::BehaviorPackage => {
                exact_fields(
                    &record,
                    &[
                        "manifest",
                        "digest_sha256",
                        "publisher_ref",
                        "admission_ref",
                    ],
                    &[],
                )?;
                behavior_packages.push(BehaviorPackageSelection {
                    manifest: field(&record, "manifest")?.into(),
                    digest_sha256: field(&record, "digest_sha256")?.into(),
                    publisher_ref: field(&record, "publisher_ref")?.into(),
                    admission_ref: field(&record, "admission_ref")?.into(),
                });
            }
            SectionKind::ProcedurePackage => {
                exact_fields(&record, &["manifest", "digest_sha256"], &[])?;
                procedure_packages.push(ProcedurePackageSelection {
                    manifest: field(&record, "manifest")?.into(),
                    digest_sha256: field(&record, "digest_sha256")?.into(),
                });
            }
            SectionKind::Project | SectionKind::SemanticPackage | SectionKind::VersionControl => {
                return Err(invalid(format!(
                    "project.sem contains an extra `{}` section",
                    section_name(record.kind)
                )));
            }
        }
    }
    let manifest = ProjectManifest {
        schema_version: field(&project, "schema_version")?.into(),
        project_id: field(&project, "project_id")?.into(),
        locale: field(&project, "locale")?.into(),
        time_zone: optional_field(&project, "time_zone").map(str::to_owned),
        semantic_packages,
        source_documents,
        service_roots: vec![SERVICES_PATH.into()],
        service_state_roots: vec![RUNTIME_SERVICES_PATH.into()],
        behavior_packages,
        procedure_packages,
        workspace_graph: Some(WORKSPACE_GRAPH_PATH.into()),
        version_control,
    };

    let analysis_source = source
        .lines()
        .enumerate()
        .map(|(index, line)| {
            if index < program_line {
                ""
            } else {
                line.trim_end_matches('\r')
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if source.ends_with('\n') { "\n" } else { "" };
    Ok(CompiledProjectSource {
        manifest,
        analysis_source,
    })
}

pub fn render(manifest: &ProjectManifest, program: &str) -> Result<String, ToolingError> {
    let mut output = String::new();
    let ja = manifest.locale.eq_ignore_ascii_case("ja")
        || manifest.locale.to_ascii_lowercase().starts_with("ja-");
    record(
        &mut output,
        label(ja, "プロジェクト", "Project"),
        &[
            (label(ja, "版", "Version"), manifest.schema_version.as_str()),
            (
                label(ja, "識別子", "Identifier"),
                manifest.project_id.as_str(),
            ),
            (label(ja, "言語", "Language"), manifest.locale.as_str()),
        ],
    )?;
    if let Some(value) = manifest.time_zone.as_deref() {
        field_line(&mut output, label(ja, "時間帯", "Timezone"), value)?;
    }
    if manifest.semantic_packages.len() > 64 {
        return Err(invalid(
            "project may select at most 64 domain semantic packages",
        ));
    }
    for package in &manifest.semantic_packages {
        record(
            &mut output,
            label(ja, "意味パッケージ", "Semantic package"),
            &[
                (
                    label(ja, "マニフェスト", "Manifest"),
                    package.manifest.as_str(),
                ),
                (
                    label(ja, "ダイジェスト", "Digest"),
                    package.digest_sha256.as_str(),
                ),
            ],
        )?;
    }
    for source in manifest
        .source_documents
        .iter()
        .filter(|source| source.path != PROJECT_SOURCE_PATH)
    {
        match source.role {
            ProjectSourceRole::Work | ProjectSourceRole::Records => {}
            ProjectSourceRole::ProjectLanguage => {
                return Err(invalid(
                    "only project.sem may own the project_language role",
                ));
            }
        }
        record(
            &mut output,
            label(ja, "ソース", "Source document"),
            &[(label(ja, "パス", "Path"), &source.path)],
        )?;
    }
    for value in &manifest.behavior_packages {
        record(
            &mut output,
            label(ja, "振る舞いパッケージ", "Behavior package"),
            &[
                (label(ja, "マニフェスト", "Manifest"), &value.manifest),
                (label(ja, "ダイジェスト", "Digest"), &value.digest_sha256),
                (label(ja, "発行者", "Publisher"), &value.publisher_ref),
                (label(ja, "採用", "Admission"), &value.admission_ref),
            ],
        )?;
    }
    for value in &manifest.procedure_packages {
        record(
            &mut output,
            label(ja, "手順パッケージ", "Procedure package"),
            &[
                (label(ja, "マニフェスト", "Manifest"), &value.manifest),
                (label(ja, "ダイジェスト", "Digest"), &value.digest_sha256),
            ],
        )?;
    }
    if let Some(value) = &manifest.version_control {
        record(
            &mut output,
            label(ja, "版管理", "Version control"),
            &[
                (label(ja, "提供方式", "Provider"), &value.provider),
                (label(ja, "基点", "Root"), &value.root),
            ],
        )?;
    }
    if !program.trim().is_empty() {
        output.push('\n');
        output.push_str(program.trim_start_matches(['\r', '\n']));
        if !output.ends_with('\n') {
            output.push('\n');
        }
    }
    Ok(output)
}

fn label<'a>(ja: bool, japanese: &'a str, english: &'a str) -> &'a str {
    if ja { japanese } else { english }
}

fn source_role(value: &str) -> Result<ProjectSourceRole, ToolingError> {
    let path = Path::new(value);
    if path.is_absolute() || path.extension().and_then(|value| value.to_str()) != Some("sem") {
        return Err(invalid(
            "source path must be a relative .sem file below work or records",
        ));
    }
    let components = path.components().collect::<Vec<_>>();
    if components.len() < 2
        || components
            .iter()
            .any(|value| !matches!(value, Component::Normal(_)))
    {
        return Err(invalid(
            "source path must be normalized below work or records",
        ));
    }
    match components[0].as_os_str().to_str() {
        Some("work") => Ok(ProjectSourceRole::Work),
        Some("records") => Ok(ProjectSourceRole::Records),
        _ => Err(invalid(
            "source role is derived from the work or records root directory",
        )),
    }
}

fn validate_version_control_root(value: &str) -> Result<(), ToolingError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::CurDir | Component::Normal(_)))
    {
        return Err(invalid(
            "version control root must be a normalized project-relative directory",
        ));
    }
    Ok(())
}

fn section_kind(value: &str) -> Option<SectionKind> {
    Some(match value {
        "プロジェクト" | "Project" => SectionKind::Project,
        "意味パッケージ" | "Semantic package" => SectionKind::SemanticPackage,
        "ソース" | "Source document" => SectionKind::SourceDocument,
        "振る舞いパッケージ" | "Behavior package" => SectionKind::BehaviorPackage,
        "手順パッケージ" | "Procedure package" => SectionKind::ProcedurePackage,
        "版管理" | "Version control" => SectionKind::VersionControl,
        _ => return None,
    })
}

fn section_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Project => "Project",
        SectionKind::SemanticPackage => "Semantic package",
        SectionKind::SourceDocument => "Source document",
        SectionKind::BehaviorPackage => "Behavior package",
        SectionKind::ProcedurePackage => "Procedure package",
        SectionKind::VersionControl => "Version control",
    }
}

fn field_key(kind: SectionKind, value: &str) -> Option<&'static str> {
    Some(match (kind, value) {
        (SectionKind::Project, "版" | "Version") => "schema_version",
        (SectionKind::Project, "識別子" | "Identifier") => "project_id",
        (SectionKind::Project, "言語" | "Language") => "locale",
        (SectionKind::Project, "時間帯" | "Timezone") => "time_zone",
        (SectionKind::SourceDocument, "パス" | "Path") => "path",
        (
            SectionKind::BehaviorPackage | SectionKind::ProcedurePackage,
            "マニフェスト" | "Manifest",
        ) => "manifest",
        (
            SectionKind::SemanticPackage
            | SectionKind::BehaviorPackage
            | SectionKind::ProcedurePackage,
            "ダイジェスト" | "Digest",
        ) => "digest_sha256",
        (SectionKind::SemanticPackage, "マニフェスト" | "Manifest") => "manifest",
        (SectionKind::BehaviorPackage, "発行者" | "Publisher") => "publisher_ref",
        (SectionKind::BehaviorPackage, "採用" | "Admission") => "admission_ref",
        (SectionKind::VersionControl, "提供方式" | "Provider") => "provider",
        (SectionKind::VersionControl, "基点" | "Root") => "root",
        _ => return None,
    })
}

fn record(output: &mut String, kind: &str, fields: &[(&str, &str)]) -> Result<(), ToolingError> {
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str("# ");
    output.push_str(kind);
    output.push('\n');
    for (name, value) in fields {
        field_line(output, name, value)?;
    }
    Ok(())
}

fn field_line(output: &mut String, name: &str, value: &str) -> Result<(), ToolingError> {
    let empty_locale = matches!(name, "言語" | "Language");
    if value.trim() != value || (!empty_locale && value.is_empty()) || value.contains(['\r', '\n'])
    {
        return Err(invalid(format!(
            "project field `{name}` is not a bounded single-line value"
        )));
    }
    output.push_str("  ");
    output.push_str(name);
    output.push_str(": ");
    output.push_str(value);
    output.push('\n');
    Ok(())
}

fn take_single(records: &mut Vec<Record>, kind: SectionKind) -> Result<Record, ToolingError> {
    let matches = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.kind == kind)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(invalid(format!(
            "project.sem requires exactly one `{}` section",
            section_name(kind)
        )));
    }
    Ok(records.remove(matches[0]))
}

fn take_many(
    records: &mut Vec<Record>,
    kind: SectionKind,
    minimum: usize,
    maximum: usize,
) -> Result<Vec<Record>, ToolingError> {
    let count = records.iter().filter(|record| record.kind == kind).count();
    if count < minimum || count > maximum {
        return Err(invalid(format!(
            "project.sem requires between {minimum} and {maximum} `{}` sections",
            section_name(kind)
        )));
    }
    let mut selected = Vec::with_capacity(count);
    let mut retained = Vec::with_capacity(records.len() - count);
    for record in records.drain(..) {
        if record.kind == kind {
            selected.push(record);
        } else {
            retained.push(record);
        }
    }
    *records = retained;
    Ok(selected)
}

fn exact_fields(record: &Record, required: &[&str], optional: &[&str]) -> Result<(), ToolingError> {
    for name in required {
        if optional_field(record, name).is_none() {
            return Err(invalid(format!(
                "`{}` is missing `{name}`",
                section_name(record.kind)
            )));
        }
    }
    if let Some((name, _)) = record
        .fields
        .iter()
        .find(|(name, _)| !required.contains(&name.as_str()) && !optional.contains(&name.as_str()))
    {
        return Err(invalid(format!(
            "`{}` contains unknown field `{name}`",
            section_name(record.kind)
        )));
    }
    Ok(())
}

fn field<'a>(record: &'a Record, name: &str) -> Result<&'a str, ToolingError> {
    optional_field(record, name).ok_or_else(|| {
        invalid(format!(
            "`{}` is missing `{name}`",
            section_name(record.kind)
        ))
    })
}

fn optional_field<'a>(record: &'a Record, name: &str) -> Option<&'a str> {
    record
        .fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.as_str())
}

fn invalid(detail: impl Into<String>) -> ToolingError {
    ToolingError::Project(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SCHEMA_VERSION;

    fn manifest(locale: &str) -> ProjectManifest {
        ProjectManifest {
            schema_version: SCHEMA_VERSION.into(),
            project_id: "project/example".into(),
            locale: locale.into(),
            time_zone: Some("Asia/Tokyo".into()),
            semantic_packages: vec![
                SemanticPackageSelection {
                    manifest: "packages/core/package.sem".into(),
                    digest_sha256: "a".repeat(64),
                },
                SemanticPackageSelection {
                    manifest: "packages/work/package.sem".into(),
                    digest_sha256: "b".repeat(64),
                },
            ],
            source_documents: vec![
                ProjectSourceDocument {
                    path: PROJECT_SOURCE_PATH.into(),
                    role: ProjectSourceRole::ProjectLanguage,
                },
                ProjectSourceDocument {
                    path: "work/review.sem".into(),
                    role: ProjectSourceRole::Work,
                },
            ],
            service_roots: vec![".sem-lang/services".into()],
            service_state_roots: vec![RUNTIME_SERVICES_PATH.into()],
            behavior_packages: vec![],
            procedure_packages: vec![],
            workspace_graph: Some(".sem-lang/data/workspace.redb".into()),
            version_control: Some(VersionControlSelection {
                provider: "git".into(),
                root: ".".into(),
            }),
        }
    }

    #[test]
    fn localized_sections_round_trip_and_keep_program_line_positions() {
        let japanese_manifest = manifest("ja-JP");
        let source = render(&japanese_manifest, "# 記録\n  入力:\n    事実\n").unwrap();
        assert!(source.starts_with("# プロジェクト\n  版: 0.10.0\n"));
        assert!(!source.contains("\n---\n"));
        let compiled = compile(&source).unwrap();
        assert_eq!(compiled.manifest, japanese_manifest);
        assert_eq!(
            compiled.analysis_source.lines().count(),
            source.lines().count()
        );
        assert_eq!(compiled.analysis_source.lines().last(), Some("    事実"));

        let english = render(&manifest("en-US"), "").unwrap();
        assert!(english.starts_with("# Project\n  Version: 0.10.0\n"));
        assert_eq!(compile(&english).unwrap().manifest.locale, "en-US");
    }

    #[test]
    fn legacy_records_parallel_state_and_unknown_fields_are_rejected() {
        let source = render(&manifest("ja-JP"), "").unwrap();
        assert!(
            compile(&source.replace("  パス: work/review.sem", "  パス: other/review.sem"))
                .is_err()
        );
        assert!(
            compile(&source.replace(
                "  パス: work/review.sem",
                "  パス: work/review.sem\n  役割: 作業"
            ))
            .is_err()
        );
        assert!(compile("project:\n  schema_version: 0.10.0\n---\n").is_err());
        assert!(
            compile(&source.replace(
                "# ソース\n  パス: work/review.sem\n",
                "# ソース\n  パス: project.sem\n"
            ))
            .is_err()
        );
    }
}
