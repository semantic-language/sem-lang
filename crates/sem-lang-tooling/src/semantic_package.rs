//! Exact, portable semantic package manifests.
//!
//! A package directory is a human-visible layout. `package.sem` is its sole
//! machine authority and explicitly binds every language source, semantic
//! module and package dependency. Directory scans never select project input.

use crate::{
    MAX_PROJECT_DOCUMENT_BYTES, SCHEMA_VERSION, ToolingError,
    path_identity::{logical_path, logical_reference_between, source_identity},
    resolve_project_file,
};
use sem_lang_core::Digest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};
use zixcel_interaction::valid_reference;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticPackageSelection {
    pub manifest: String,
    pub digest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct SemanticPackageSource {
    pub package_ref: String,
    pub source_identity: String,
    pub package_version: String,
    pub source: String,
    pub digest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticPackage {
    pub schema_version: String,
    pub package_ref: String,
    pub version: String,
    pub owner_ref: String,
    pub manifest_ref: String,
    pub manifest_digest: String,
    pub dependencies: Vec<String>,
    pub language_sources: Vec<SemanticPackageSource>,
    pub semantic_modules: Vec<SemanticPackageSource>,
}

pub(crate) struct LoadedPackages {
    pub packages: Vec<SemanticPackage>,
    pub language_sources: Vec<SemanticPackageSource>,
    pub semantic_modules: Vec<SemanticPackageSource>,
}

pub(crate) fn load(
    base: &Path,
    selections: &[SemanticPackageSelection],
) -> Result<LoadedPackages, ToolingError> {
    if selections.len() > 64 {
        return Err(ToolingError::Vocabulary(
            "project may select at most 64 domain semantic packages".into(),
        ));
    }
    let mut manifest_paths = BTreeSet::new();
    let mut pending = BTreeMap::new();
    for selection in selections {
        if !manifest_paths.insert(selection.manifest.clone()) {
            return Err(ToolingError::Vocabulary(
                "semantic package manifest paths must be unique".into(),
            ));
        }
        let path = resolve_project_file(
            base,
            &selection.manifest,
            "semantic package manifest",
            MAX_PROJECT_DOCUMENT_BYTES,
        )?;
        let bytes = fs::read(path)?;
        if Digest::document_digest(&bytes).to_hex() != selection.digest_sha256 {
            return Err(ToolingError::Vocabulary(format!(
                "semantic package digest does not match project.sem: {}",
                selection.manifest
            )));
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            ToolingError::Vocabulary(format!(
                "semantic package {} is not UTF-8",
                selection.manifest
            ))
        })?;
        let package = compile(&selection.manifest, source).map_err(ToolingError::Vocabulary)?;
        if package.schema_version != SCHEMA_VERSION || package.version != SCHEMA_VERSION {
            return Err(ToolingError::Vocabulary(
                "semantic package version must exactly match 0.10.0".into(),
            ));
        }
        if pending
            .insert(package.package_ref.clone(), package)
            .is_some()
        {
            return Err(ToolingError::Vocabulary(
                "semantic package identities must be unique".into(),
            ));
        }
    }
    let selected = pending.keys().cloned().collect::<BTreeSet<_>>();
    for package in pending.values() {
        if let Some(dependency) = package.dependencies.iter().find(|dependency| {
            dependency.as_str() != sem_lang_standard::PACKAGE_REF && !selected.contains(*dependency)
        }) {
            return Err(ToolingError::Vocabulary(format!(
                "semantic package {} requires unselected package {dependency}",
                package.package_ref
            )));
        }
    }

    let mut packages = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let admitted = packages
            .iter()
            .map(|package: &SemanticPackage| package.package_ref.as_str())
            .collect::<BTreeSet<_>>();
        let ready = pending
            .iter()
            .find(|(_, package)| {
                package.dependencies.iter().all(|dependency| {
                    dependency == sem_lang_standard::PACKAGE_REF
                        || admitted.contains(dependency.as_str())
                })
            })
            .map(|(id, _)| id.clone())
            .ok_or_else(|| {
                ToolingError::Vocabulary("semantic package dependencies contain a cycle".into())
            })?;
        packages.push(
            pending
                .remove(&ready)
                .expect("ready package remains selected"),
        );
    }

    let mut language_sources = Vec::new();
    let mut semantic_modules = Vec::new();
    for package in &packages {
        let parent = Path::new(&package.manifest_ref).parent().ok_or_else(|| {
            ToolingError::Vocabulary("semantic package manifest has no directory".into())
        })?;
        for selection in &package.language_sources {
            language_sources.push(resolve_source(parent, selection)?);
        }
        for selection in &package.semantic_modules {
            semantic_modules.push(resolve_source(parent, selection)?);
        }
    }
    Ok(LoadedPackages {
        packages,
        language_sources,
        semantic_modules,
    })
}

fn resolve_source(
    parent: &Path,
    selection: &SemanticPackageSource,
) -> Result<SemanticPackageSource, ToolingError> {
    let source = logical_path(&parent.join(&selection.source)).map_err(ToolingError::Vocabulary)?;
    Ok(SemanticPackageSource {
        package_ref: selection.package_ref.clone(),
        source_identity: selection.source_identity.clone(),
        package_version: selection.package_version.clone(),
        source,
        digest_sha256: selection.digest_sha256.clone(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Package,
    Dependency,
    Language,
    Module,
}

pub fn compile(manifest_ref: &str, source: &str) -> Result<SemanticPackage, String> {
    if !valid_manifest_ref(manifest_ref) || source.as_bytes().contains(&0) {
        return Err("semantic package manifest reference or bytes are invalid".into());
    }
    let mut package = None::<BTreeMap<String, String>>;
    let mut dependencies = Vec::new();
    let mut languages = Vec::new();
    let mut modules = Vec::new();
    let mut current = None::<Section>;
    let mut fields = BTreeMap::new();
    let flush = |section: Option<Section>,
                 fields: &mut BTreeMap<String, String>,
                 package: &mut Option<BTreeMap<String, String>>,
                 dependencies: &mut Vec<BTreeMap<String, String>>,
                 languages: &mut Vec<BTreeMap<String, String>>,
                 modules: &mut Vec<BTreeMap<String, String>>|
     -> Result<(), String> {
        let Some(section) = section else {
            return if fields.is_empty() {
                Ok(())
            } else {
                Err("semantic package fields must follow a section".into())
            };
        };
        if fields.is_empty() {
            return Err("semantic package section cannot be empty".into());
        }
        let value = std::mem::take(fields);
        match section {
            Section::Package if package.is_none() => *package = Some(value),
            Section::Package => return Err("semantic package header is duplicated".into()),
            Section::Dependency => dependencies.push(value),
            Section::Language => languages.push(value),
            Section::Module => modules.push(value),
        }
        Ok(())
    };

    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#').map(str::trim) {
            flush(
                current,
                &mut fields,
                &mut package,
                &mut dependencies,
                &mut languages,
                &mut modules,
            )?;
            current = Some(match heading {
                "意味パッケージ" | "Semantic package" => Section::Package,
                "依存パッケージ" | "Package dependency" => Section::Dependency,
                "言語ソース" | "Language source" => Section::Language,
                "意味モジュール" | "Semantic module" => Section::Module,
                _ => {
                    return Err(format!(
                        "unknown semantic package section on line {}: {heading}",
                        index + 1
                    ));
                }
            });
            continue;
        }
        if !line.starts_with("  ") || line.starts_with("    ") {
            return Err(format!(
                "semantic package line {} must be a section or two-space-indented field",
                index + 1
            ));
        }
        let (key, value) = trimmed.split_once(':').ok_or_else(|| {
            format!(
                "semantic package field on line {} must use `name: value`",
                index + 1
            )
        })?;
        let section = current.ok_or_else(|| {
            format!(
                "semantic package field on line {} has no section",
                index + 1
            )
        })?;
        let key = field_key(section, key.trim())
            .ok_or_else(|| format!("unknown semantic package field on line {}", index + 1))?;
        let value = value.trim();
        if value.is_empty() || value.len() > 4_096 || value.contains(['\r', '\n']) {
            return Err(format!(
                "invalid semantic package value on line {}",
                index + 1
            ));
        }
        if fields.insert(key.into(), value.into()).is_some() {
            return Err(format!(
                "duplicate semantic package field on line {}",
                index + 1
            ));
        }
    }
    flush(
        current,
        &mut fields,
        &mut package,
        &mut dependencies,
        &mut languages,
        &mut modules,
    )?;

    let package = package.ok_or_else(|| "semantic package header is missing".to_owned())?;
    exact(&package, &["version", "owner"])?;
    let package_ref =
        logical_reference_between(Path::new(manifest_ref), &["packages"], "package.sem")?;
    let version = required(&package, "version")?.to_owned();
    let owner_ref = required(&package, "owner")?.to_owned();
    if !valid_reference(&package_ref) || !valid_reference(&owner_ref) {
        return Err("semantic package identity or owner is invalid".into());
    }

    let mut dependency_refs = BTreeSet::new();
    for value in dependencies {
        exact(&value, &["id"])?;
        let dependency = required(&value, "id")?.to_owned();
        if !valid_reference(&dependency)
            || dependency == package_ref
            || !dependency_refs.insert(dependency)
        {
            return Err("semantic package dependency is invalid or duplicated".into());
        }
    }
    let language_sources = sources(languages, "language", &package_ref, &version)?;
    let semantic_modules = sources(modules, "module", &package_ref, &version)?;
    if language_sources.is_empty() && semantic_modules.is_empty() {
        return Err("semantic package must contain a language source or semantic module".into());
    }
    Ok(SemanticPackage {
        schema_version: version.clone(),
        package_ref,
        version,
        owner_ref,
        manifest_ref: manifest_ref.into(),
        manifest_digest: Digest::document_digest(source.as_bytes()).to_hex(),
        dependencies: dependency_refs.into_iter().collect(),
        language_sources,
        semantic_modules,
    })
}

fn sources(
    values: Vec<BTreeMap<String, String>>,
    kind: &str,
    package_ref: &str,
    package_version: &str,
) -> Result<Vec<SemanticPackageSource>, String> {
    if values.len() > 128 {
        return Err(format!("semantic package contains too many {kind} sources"));
    }
    let mut paths = BTreeSet::new();
    let mut sources = Vec::with_capacity(values.len());
    for value in values {
        exact(&value, &["source", "digest"])?;
        let source = required(&value, "source")?.to_owned();
        let digest_sha256 = required(&value, "digest")?.to_owned();
        if !valid_relative_source(&source)
            || digest_sha256.len() != 64
            || !digest_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !paths.insert(source.clone())
        {
            return Err(format!(
                "semantic package {kind} source is invalid or duplicated"
            ));
        }
        sources.push(SemanticPackageSource {
            package_ref: package_ref.into(),
            source_identity: source_identity(package_ref, Path::new(&source))?,
            package_version: package_version.into(),
            source,
            digest_sha256,
        });
    }
    sources.sort();
    Ok(sources)
}

fn field_key(section: Section, value: &str) -> Option<&'static str> {
    Some(match (section, value) {
        (Section::Package, "版" | "Version") => "version",
        (Section::Package, "所有者" | "Owner") => "owner",
        (Section::Dependency, "識別子" | "Identifier") => "id",
        (Section::Language | Section::Module, "ソース" | "Source") => "source",
        (Section::Language | Section::Module, "ダイジェスト" | "Digest") => "digest",
        _ => return None,
    })
}

fn exact(fields: &BTreeMap<String, String>, expected: &[&str]) -> Result<(), String> {
    if fields.len() == expected.len() && expected.iter().all(|key| fields.contains_key(*key)) {
        Ok(())
    } else {
        Err("semantic package section has missing or unsupported fields".into())
    }
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("semantic package field `{key}` is missing"))
}

fn valid_manifest_ref(value: &str) -> bool {
    valid_relative_source(value) && value.ends_with("package.sem")
}

fn valid_relative_source(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && value.len() <= 512
        && value.ends_with(".sem")
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manifest_is_exact_and_directory_scans_are_not_authority() {
        let source = format!(
            "# 意味パッケージ\n  版: 0.10.0\n  所有者: organization/example\n\n# 依存パッケージ\n  識別子: sem-lang/standard\n\n# 言語ソース\n  ソース: language/work.sem\n  ダイジェスト: {}\n\n# 意味モジュール\n  ソース: modules/roles.sem\n  ダイジェスト: {}\n",
            "a".repeat(64),
            "b".repeat(64)
        );
        let package = compile("packages/example/work/package.sem", &source).unwrap();
        assert_eq!(package.package_ref, "example/work");
        assert_eq!(package.dependencies, ["sem-lang/standard"]);
        assert_eq!(package.language_sources.len(), 1);
        assert_eq!(
            package.language_sources[0].source_identity,
            "example/work/language/work"
        );
        assert_eq!(package.semantic_modules.len(), 1);
        assert!(
            compile(
                "packages/example/work/package.sem",
                &format!(
                    "{source}\n# 言語ソース\n  ソース: ../escape.sem\n  ダイジェスト: {}\n",
                    "c".repeat(64)
                )
            )
            .is_err()
        );
    }
}
