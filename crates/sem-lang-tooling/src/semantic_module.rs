//! Compilation of reusable `.sem` data modules.
//!
//! Authored `.sem` is the only project-visible source.  The compiler turns
//! stable semantic references into TermIds and exposes immutable compiled
//! modules.  Presentation clients never parse the source or a parallel JSON
//! data package.

use sem_lang_core::{Digest, TermId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use zixcel_interaction::valid_reference;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticModuleSelection {
    pub source: String,
    pub digest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompiledSemanticModule {
    pub schema_version: String,
    pub module_ref: String,
    pub revision: String,
    pub owner_ref: String,
    pub source_ref: String,
    pub source_digest: String,
    pub module_digest: String,
    pub subjects: Vec<SemanticSubjectDefinition>,
    pub resources: Vec<SemanticResourceDefinition>,
    /// Exact bindings from a semantic term to a typed value producer.
    ///
    /// A lexical term only explains meaning.  It cannot be used as pipeline
    /// input until a selected module binds it to an authority-owned producer.
    pub value_bindings: Vec<SemanticValueBinding>,
    pub relations: Vec<SemanticRelationDefinition>,
    pub role_assignments: Vec<SemanticRoleAssignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticValueBinding {
    pub value_ref: String,
    pub term_source_ref: String,
    pub term_name: String,
    pub term_id: String,
    pub value_type_ref: String,
    pub cardinality: String,
    pub authority_ref: String,
    pub producer_ref: String,
    pub selector: Option<String>,
    pub materialization: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticSubjectDefinition {
    pub subject_ref: String,
    pub subject_kind: String,
    pub subject_kind_term_id: String,
    pub authority_ref: String,
    pub authority_revision: String,
    pub presentation_label: String,
    pub language_tag: String,
    pub sensitivity: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticResourceDefinition {
    pub resource_ref: String,
    pub resource_kind: String,
    pub resource_kind_term_id: String,
    pub owner_ref: String,
    pub presentation_label: String,
    pub language_tag: String,
    pub access_mode: String,
    pub data_boundary: String,
    pub value_type_ref: Option<String>,
    pub value_type_term_id: Option<String>,
    pub value: Option<String>,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticRelationDefinition {
    pub relation_ref: String,
    pub from_ref: String,
    pub predicate_ref: String,
    pub predicate_term_id: String,
    pub to_ref: String,
    pub authority_ref: String,
    pub authority_revision: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SemanticRoleAssignment {
    pub assignment_ref: String,
    pub assignee_ref: String,
    pub role_ref: String,
    pub scope_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Module,
    Subject,
    Resource,
    ValueBinding,
    Relation,
    Role,
}

#[derive(Default)]
struct ModuleBuilder {
    module: BTreeMap<String, String>,
    subjects: Vec<BTreeMap<String, String>>,
    resources: Vec<BTreeMap<String, String>>,
    value_bindings: Vec<BTreeMap<String, String>>,
    relations: Vec<BTreeMap<String, String>>,
    roles: Vec<BTreeMap<String, String>>,
}

/// Stable extension identity derived from an authority-controlled semantic
/// reference, never from a localized display label.
#[must_use]
pub fn term_id_for_reference(reference: &str) -> TermId {
    let digest =
        Digest::document_digest(format!("sem-lang/semantic-reference/v1\0{reference}").as_bytes())
            .to_hex();
    TermId::from_hex(&digest[..32]).expect("a SHA-256 prefix is a valid TermId")
}

pub fn compile(
    module_ref: &str,
    source_ref: &str,
    source: &str,
    schema_version: &str,
) -> Result<CompiledSemanticModule, String> {
    let mut builder = ModuleBuilder::default();
    let mut current = None::<SectionKind>;
    let mut fields = BTreeMap::<String, String>::new();

    let flush = |kind: Option<SectionKind>,
                 fields: &mut BTreeMap<String, String>,
                 target: &mut ModuleBuilder|
     -> Result<(), String> {
        let Some(kind) = kind else {
            if fields.is_empty() {
                return Ok(());
            }
            return Err("semantic module fields must follow a section heading".into());
        };
        if fields.is_empty() {
            return Err("semantic module section cannot be empty".into());
        }
        let value = std::mem::take(fields);
        match kind {
            SectionKind::Module if target.module.is_empty() => target.module = value,
            SectionKind::Module => {
                return Err("semantic module declares its module header more than once".into());
            }
            SectionKind::Subject => target.subjects.push(value),
            SectionKind::Resource => target.resources.push(value),
            SectionKind::ValueBinding => target.value_bindings.push(value),
            SectionKind::Relation => target.relations.push(value),
            SectionKind::Role => target.roles.push(value),
        }
        Ok(())
    };

    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(heading) = line.strip_prefix('#').map(str::trim) {
            flush(current, &mut fields, &mut builder)?;
            current = Some(section_kind(heading).ok_or_else(|| {
                format!(
                    "unknown semantic module section on line {}: {heading}",
                    index + 1
                )
            })?);
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(|| {
            format!(
                "semantic module field on line {} must use `name: value`",
                index + 1
            )
        })?;
        let key = field_key(key.trim()).ok_or_else(|| {
            format!(
                "unknown semantic module field on line {}: {}",
                index + 1,
                key.trim()
            )
        })?;
        let value = value.trim();
        if value.is_empty() || value.len() > 4_096 || value.contains(['\r', '\n']) {
            return Err(format!(
                "semantic module field on line {} is empty or too large",
                index + 1
            ));
        }
        if fields.insert(key.into(), value.into()).is_some() {
            return Err(format!(
                "semantic module field `{key}` is duplicated on line {}",
                index + 1
            ));
        }
    }
    flush(current, &mut fields, &mut builder)?;

    let source_digest = Digest::document_digest(source.as_bytes()).to_hex();
    let mut module = CompiledSemanticModule {
        schema_version: schema_version.into(),
        module_ref: module_ref.into(),
        revision: source_digest.clone(),
        owner_ref: take(&builder.module, "owner")?,
        source_ref: source_ref.into(),
        source_digest,
        module_digest: String::new(),
        subjects: builder
            .subjects
            .iter()
            .map(subject)
            .collect::<Result<_, _>>()?,
        resources: builder
            .resources
            .iter()
            .map(resource)
            .collect::<Result<_, _>>()?,
        value_bindings: builder
            .value_bindings
            .iter()
            .map(value_binding)
            .collect::<Result<_, _>>()?,
        relations: builder
            .relations
            .iter()
            .map(relation)
            .collect::<Result<_, _>>()?,
        role_assignments: builder.roles.iter().map(role).collect::<Result<_, _>>()?,
    };
    if builder.module.len() != 1 {
        return Err("semantic module header accepts only owner".into());
    }
    module.module_digest = module_digest(&module);
    Ok(module)
}

pub fn validate_modules(
    modules: &[CompiledSemanticModule],
    schema_version: &str,
    known_role_refs: &BTreeSet<&str>,
) -> Result<(), String> {
    if modules.len() > 128 {
        return Err("project selects more than 128 semantic modules".into());
    }
    let mut module_refs = BTreeSet::new();
    let mut available = BTreeSet::new();
    for module in modules {
        if module.schema_version != schema_version
            || !valid_reference(&module.module_ref)
            || !valid_reference(&module.revision)
            || !valid_reference(&module.owner_ref)
            || !valid_source_path(&module.source_ref)
            || module.source_digest.len() != 64
            || module.module_digest != module_digest(module)
            || !module_refs.insert(module.module_ref.as_str())
            || module.subjects.len() > 512
            || module.resources.len() > 1_024
            || module.value_bindings.len() > 1_024
            || module.relations.len() > 2_048
            || module.role_assignments.len() > 2_048
        {
            return Err("semantic module identity, digest, or size is invalid".into());
        }
        for value in &module.subjects {
            if !valid_reference(&value.subject_ref)
                || !valid_reference(&value.subject_kind)
                || value.subject_kind_term_id != term_id_for_reference(&value.subject_kind).to_hex()
                || !valid_reference(&value.authority_ref)
                || !valid_reference(&value.authority_revision)
                || !text(&value.presentation_label, 256)
                || !valid_language_tag(&value.language_tag)
                || !matches!(
                    value.sensitivity.as_str(),
                    "public" | "internal" | "personal" | "customer_confidential"
                )
                || (value.subject_kind.ends_with("/person") && value.sensitivity == "public")
                || !valid_reference(&value.evidence_ref)
                || !available.insert(value.subject_ref.as_str())
            {
                return Err("semantic module subject is invalid or duplicated".into());
            }
        }
        for value in &module.resources {
            if !valid_reference(&value.resource_ref)
                || !valid_reference(&value.resource_kind)
                || value.resource_kind_term_id
                    != term_id_for_reference(&value.resource_kind).to_hex()
                || !text(&value.presentation_label, 256)
                || !valid_language_tag(&value.language_tag)
                || !matches!(
                    value.access_mode.as_str(),
                    "reference_only" | "human_mediated" | "read_only" | "read_write"
                )
                || !matches!(
                    value.data_boundary.as_str(),
                    "metadata_only"
                        | "exact_reference"
                        | "project_content"
                        | "external_payload"
                        | "public_reference"
                )
                || !valid_reference(&value.evidence_ref)
                || !valid_typed_value(value)
                || !available.insert(value.resource_ref.as_str())
            {
                return Err("semantic module resource is invalid or duplicated".into());
            }
        }
        for value in &module.value_bindings {
            if !valid_reference(&value.value_ref)
                || !valid_reference(&value.term_source_ref)
                || !canonical_name(&value.term_name)
                || value.term_id
                    != TermId::from_declaration(&value.term_source_ref, &value.term_name).to_hex()
                || !valid_reference(&value.value_type_ref)
                || !matches!(value.cardinality.as_str(), "one" | "optional" | "many")
                || !valid_reference(&value.authority_ref)
                || !valid_reference(&value.producer_ref)
                || value
                    .selector
                    .as_deref()
                    .is_some_and(|value| !valid_selector(value))
                || !matches!(
                    value.materialization.as_str(),
                    "reference_only"
                        | "project_record"
                        | "external_artifact"
                        | "decision"
                        | "evidence"
                )
                || !valid_reference(&value.evidence_ref)
                || !available.insert(value.value_ref.as_str())
            {
                return Err("semantic value binding is invalid or duplicated".into());
            }
        }
    }
    let mut relations = BTreeSet::new();
    let mut assignments = BTreeSet::new();
    for module in modules {
        if !available.contains(module.owner_ref.as_str()) {
            return Err(format!(
                "semantic module owner {} is unresolved",
                module.owner_ref
            ));
        }
        for value in &module.resources {
            if !available.contains(value.owner_ref.as_str()) {
                return Err(format!(
                    "semantic resource owner {} is unresolved",
                    value.owner_ref
                ));
            }
        }
        for value in &module.relations {
            if !valid_reference(&value.relation_ref)
                || !available.contains(value.from_ref.as_str())
                || !valid_reference(&value.predicate_ref)
                || value.predicate_term_id != term_id_for_reference(&value.predicate_ref).to_hex()
                || !available.contains(value.to_ref.as_str())
                || !valid_reference(&value.authority_ref)
                || !valid_reference(&value.authority_revision)
                || !valid_reference(&value.evidence_ref)
                || !relations.insert(value.relation_ref.as_str())
            {
                return Err(
                    "semantic relation is invalid, duplicated, or has an unresolved endpoint"
                        .into(),
                );
            }
        }
        for value in &module.role_assignments {
            if !valid_reference(&value.assignment_ref)
                || !available.contains(value.assignee_ref.as_str())
                || !known_role_refs.contains(value.role_ref.as_str())
                || !valid_reference(&value.scope_ref)
                || !assignments.insert(value.assignment_ref.as_str())
            {
                return Err("semantic role assignment is invalid or unresolved".into());
            }
        }
    }
    Ok(())
}

fn section_kind(value: &str) -> Option<SectionKind> {
    match value {
        "意味モジュール" | "Semantic module" => Some(SectionKind::Module),
        "主体" | "Subject" => Some(SectionKind::Subject),
        "資源" | "Resource" => Some(SectionKind::Resource),
        "値束縛" | "Value binding" => Some(SectionKind::ValueBinding),
        "関係" | "Relation" => Some(SectionKind::Relation),
        "役割割当" | "Role assignment" => Some(SectionKind::Role),
        _ => None,
    }
}

fn field_key(value: &str) -> Option<&'static str> {
    Some(match value {
        "識別子" | "Id" => "id",
        "版" | "Revision" => "revision",
        "所有者" | "Owner" => "owner",
        "種別" | "Kind" => "kind",
        "表示名" | "Label" => "label",
        "言語" | "Language" => "language",
        "公開範囲" | "Sensitivity" => "sensitivity",
        "正本" | "Authority" => "authority",
        "正本の版" | "Authority revision" => "authority_revision",
        "根拠" | "Evidence" => "evidence",
        "アクセス" | "Access" => "access",
        "データ境界" | "Data boundary" => "data_boundary",
        "値の型" | "Value type" => "value_type",
        "値" | "Value" => "value",
        "用語ソース" | "Term source" => "term_source",
        "用語" | "Term" => "term",
        "件数" | "Cardinality" => "cardinality",
        "生成元" | "Producer" => "producer",
        "選択" | "Selector" => "selector",
        "実体化" | "Materialization" => "materialization",
        "始点" | "From" => "from",
        "述語" | "Predicate" => "predicate",
        "終点" | "To" => "to",
        "割当先" | "Assignee" => "assignee",
        "役割" | "Role" => "role",
        "範囲" | "Scope" => "scope",
        _ => return None,
    })
}

fn subject(fields: &BTreeMap<String, String>) -> Result<SemanticSubjectDefinition, String> {
    exact_fields(
        fields,
        &[
            "id",
            "kind",
            "label",
            "language",
            "sensitivity",
            "authority",
            "authority_revision",
            "evidence",
        ],
    )?;
    let kind = take(fields, "kind")?;
    Ok(SemanticSubjectDefinition {
        subject_ref: take(fields, "id")?,
        subject_kind_term_id: term_id_for_reference(&kind).to_hex(),
        subject_kind: kind,
        authority_ref: take(fields, "authority")?,
        authority_revision: take(fields, "authority_revision")?,
        presentation_label: take(fields, "label")?,
        language_tag: take(fields, "language")?,
        sensitivity: take(fields, "sensitivity")?,
        evidence_ref: take(fields, "evidence")?,
    })
}

fn resource(fields: &BTreeMap<String, String>) -> Result<SemanticResourceDefinition, String> {
    exact_fields(
        fields,
        &[
            "id",
            "kind",
            "owner",
            "label",
            "language",
            "access",
            "data_boundary",
            "evidence",
            "value_type",
            "value",
        ],
    )?;
    let kind = take(fields, "kind")?;
    let value_type_ref = fields.get("value_type").cloned();
    let value_type_term_id = value_type_ref
        .as_deref()
        .map(term_id_for_reference)
        .map(|value| value.to_hex());
    Ok(SemanticResourceDefinition {
        resource_ref: take(fields, "id")?,
        resource_kind_term_id: term_id_for_reference(&kind).to_hex(),
        resource_kind: kind,
        owner_ref: take(fields, "owner")?,
        presentation_label: take(fields, "label")?,
        language_tag: take(fields, "language")?,
        access_mode: take(fields, "access")?,
        data_boundary: take(fields, "data_boundary")?,
        value_type_ref,
        value_type_term_id,
        value: fields.get("value").cloned(),
        evidence_ref: take(fields, "evidence")?,
    })
}

fn value_binding(fields: &BTreeMap<String, String>) -> Result<SemanticValueBinding, String> {
    exact_fields(
        fields,
        &[
            "id",
            "term_source",
            "term",
            "value_type",
            "cardinality",
            "authority",
            "producer",
            "selector",
            "materialization",
            "evidence",
        ],
    )?;
    let term_source_ref = take(fields, "term_source")?;
    let term_name = take(fields, "term")?;
    Ok(SemanticValueBinding {
        value_ref: take(fields, "id")?,
        term_id: TermId::from_declaration(&term_source_ref, &term_name).to_hex(),
        term_source_ref,
        term_name,
        value_type_ref: take(fields, "value_type")?,
        cardinality: take(fields, "cardinality")?,
        authority_ref: take(fields, "authority")?,
        producer_ref: take(fields, "producer")?,
        selector: fields.get("selector").cloned(),
        materialization: take(fields, "materialization")?,
        evidence_ref: take(fields, "evidence")?,
    })
}

fn relation(fields: &BTreeMap<String, String>) -> Result<SemanticRelationDefinition, String> {
    exact_fields(
        fields,
        &[
            "id",
            "from",
            "predicate",
            "to",
            "authority",
            "authority_revision",
            "evidence",
        ],
    )?;
    let predicate = take(fields, "predicate")?;
    Ok(SemanticRelationDefinition {
        relation_ref: take(fields, "id")?,
        from_ref: take(fields, "from")?,
        predicate_term_id: term_id_for_reference(&predicate).to_hex(),
        predicate_ref: predicate,
        to_ref: take(fields, "to")?,
        authority_ref: take(fields, "authority")?,
        authority_revision: take(fields, "authority_revision")?,
        evidence_ref: take(fields, "evidence")?,
    })
}

fn role(fields: &BTreeMap<String, String>) -> Result<SemanticRoleAssignment, String> {
    exact_fields(fields, &["id", "assignee", "role", "scope"])?;
    Ok(SemanticRoleAssignment {
        assignment_ref: take(fields, "id")?,
        assignee_ref: take(fields, "assignee")?,
        role_ref: take(fields, "role")?,
        scope_ref: take(fields, "scope")?,
    })
}

fn take(fields: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    fields
        .get(key)
        .cloned()
        .ok_or_else(|| format!("semantic module requires `{key}`"))
}

fn exact_fields(fields: &BTreeMap<String, String>, accepted: &[&str]) -> Result<(), String> {
    if let Some(key) = fields.keys().find(|key| !accepted.contains(&key.as_str())) {
        return Err(format!(
            "semantic module field `{key}` is not valid in this section"
        ));
    }
    Ok(())
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && value.len() <= max
        && !value.contains(['\r', '\n'])
}

fn valid_source_path(value: &str) -> bool {
    value.ends_with(".sem")
        && !value.starts_with('/')
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn valid_language_tag(value: &str) -> bool {
    value == "*"
        || (value.len() <= 64
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part.len() <= 16
                    && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            }))
}

fn canonical_name(value: &str) -> bool {
    text(value, 256)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_selector(value: &str) -> bool {
    text(value, 1_024)
        && !value.starts_with('/')
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn valid_typed_value(value: &SemanticResourceDefinition) -> bool {
    match (
        &value.value_type_ref,
        &value.value_type_term_id,
        &value.value,
    ) {
        (None, None, None) => true,
        (Some(reference), Some(term_id), Some(raw)) => {
            valid_reference(reference)
                && *term_id == term_id_for_reference(reference).to_hex()
                && text(raw, 4_096)
                && (!reference.ends_with("/url")
                    || (raw.starts_with("https://")
                        && raw.len() <= 2_048
                        && !raw.chars().any(char::is_whitespace)))
        }
        _ => false,
    }
}

fn module_digest(module: &CompiledSemanticModule) -> String {
    let mut lines = vec![
        module.schema_version.clone(),
        module.module_ref.clone(),
        module.revision.clone(),
        module.owner_ref.clone(),
        module.source_ref.clone(),
        module.source_digest.clone(),
    ];
    lines.extend(module.subjects.iter().map(|value| {
        format!(
            "S\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            value.subject_ref,
            value.subject_kind,
            value.subject_kind_term_id,
            value.authority_ref,
            value.authority_revision,
            value.presentation_label,
            value.language_tag,
            value.sensitivity,
            value.evidence_ref
        )
    }));
    lines.extend(module.resources.iter().map(|value| {
        format!(
            "R\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            value.resource_ref,
            value.resource_kind,
            value.resource_kind_term_id,
            value.owner_ref,
            value.presentation_label,
            value.language_tag,
            value.access_mode,
            value.data_boundary,
            value.value_type_ref.as_deref().unwrap_or(""),
            value.value_type_term_id.as_deref().unwrap_or(""),
            value.value.as_deref().unwrap_or(""),
            value.evidence_ref
        )
    }));
    lines.extend(module.value_bindings.iter().map(|value| {
        format!(
            "V\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            value.value_ref,
            value.term_source_ref,
            value.term_name,
            value.term_id,
            value.value_type_ref,
            value.cardinality,
            value.authority_ref,
            value.producer_ref,
            value.selector.as_deref().unwrap_or(""),
            value.materialization,
            value.evidence_ref
        )
    }));
    lines.extend(module.relations.iter().map(|value| {
        format!(
            "E\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            value.relation_ref,
            value.from_ref,
            value.predicate_ref,
            value.predicate_term_id,
            value.to_ref,
            value.authority_ref,
            value.authority_revision,
            value.evidence_ref
        )
    }));
    lines.extend(module.role_assignments.iter().map(|value| {
        format!(
            "A\0{}\0{}\0{}\0{}",
            value.assignment_ref, value.assignee_ref, value.role_ref, value.scope_ref
        )
    }));
    Digest::document_digest(lines.join("\n").as_bytes()).to_hex()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORGANIZATION: &str = r#"# 意味モジュール
  所有者: subject/organization/hat

# 主体
  識別子: subject/organization/hat
  種別: kind/organization
  表示名: 例示組織
  言語: ja-JP
  公開範囲: public
  正本: identity/organization/hat
  正本の版: revision/hat/official/one
  根拠: reference/hat/official
"#;

    const WEBSITE: &str = r#"# 意味モジュール
  所有者: subject/organization/hat

# 資源
  識別子: resource/hat/website
  種別: kind/website
  所有者: subject/organization/hat
  表示名: 例示組織の公式Webサイト
  言語: ja-JP
  アクセス: reference_only
  データ境界: public_reference
  値の型: type/web/url
  値: https://example.invalid/
  根拠: reference/hat/official

# 関係
  識別子: relation/hat/website
  始点: subject/organization/hat
  述語: relation/has-website
  終点: resource/hat/website
  正本: authority/hat/official
  正本の版: revision/hat/official/one
  根拠: reference/hat/official
"#;

    const VALUE_BINDING: &str = r#"# 意味モジュール
  所有者: subject/example/project

# 主体
  識別子: subject/example/project
  種別: kind/project
  表示名: Example project
  言語: en-US
  公開範囲: internal
  正本: authority/example/project
  正本の版: revision/example/project/one
  根拠: reference/example/project

# 値束縛
  識別子: value/example/current-work
  用語ソース: example/work/language/work
  用語: CurrentWork
  値の型: kind/work/activity
  件数: many
  正本: workspace/example
  生成元: workspace/activity/current
  実体化: reference_only
  根拠: source/work/current
"#;

    #[test]
    fn compiles_localized_sem_source_to_stable_term_ids() {
        let organization = compile(
            "hat/public-context/modules/organizations/hat/identity",
            "modules/organizations/hat/identity.sem",
            ORGANIZATION,
            "0.10.0",
        )
        .unwrap();
        let website = compile(
            "hat/public-context/modules/organizations/hat/resources/official-website",
            "modules/organizations/hat/resources/official-website.sem",
            WEBSITE,
            "0.10.0",
        )
        .unwrap();
        assert_eq!(organization.subjects[0].presentation_label, "例示組織");
        assert_eq!(
            website.resources[0].value.as_deref(),
            Some("https://example.invalid/")
        );
        assert_eq!(
            website.relations[0].predicate_term_id,
            term_id_for_reference("relation/has-website").to_hex()
        );
        validate_modules(&[organization, website], "0.10.0", &BTreeSet::new()).unwrap();
    }

    #[test]
    fn compiles_exact_term_to_value_producer_binding() {
        let module = compile(
            "example/work/modules/values/current-work",
            "packages/example/work/modules/values/current-work.sem",
            VALUE_BINDING,
            "0.10.0",
        )
        .unwrap();
        let binding = module.value_bindings.first().unwrap();
        assert_eq!(binding.value_ref, "value/example/current-work");
        assert_eq!(binding.term_source_ref, "example/work/language/work");
        assert_eq!(binding.term_name, "CurrentWork");
        assert_eq!(
            binding.term_id,
            TermId::from_declaration("example/work/language/work", "CurrentWork").to_hex()
        );
        assert_eq!(binding.producer_ref, "workspace/activity/current");
        validate_modules(&[module], "0.10.0", &BTreeSet::new()).unwrap();
    }

    #[test]
    fn rejects_unresolved_cross_module_endpoint() {
        let organization = compile(
            "hat/public-context/modules/organizations/hat/identity",
            "modules/organizations/hat/identity.sem",
            ORGANIZATION,
            "0.10.0",
        )
        .unwrap();
        let mut website = compile(
            "hat/public-context/modules/organizations/hat/resources/official-website",
            "modules/organizations/hat/resources/official-website.sem",
            WEBSITE,
            "0.10.0",
        )
        .unwrap();
        website.relations[0].to_ref = "resource/missing".into();
        website.module_digest = module_digest(&website);
        assert!(validate_modules(&[organization, website], "0.10.0", &BTreeSet::new()).is_err());
    }
}
