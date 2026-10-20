use super::{
    Cardinality, LexicalBinding, RoleDefinition, TermDefinition, TypeDefinition, VocabularyError,
    VocabularyModule,
};
use sem_lang_core::{RoleId, TermId, TypeId};
use sem_lang_linguistics::{PrimitiveSemantic, SemanticDefinition, SemanticExpr};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Section {
    Language,
    Term,
    Type,
    Role,
    Surface,
}

type Fields = BTreeMap<String, String>;

pub(super) fn compile(raw: &str) -> Result<VocabularyModule, VocabularyError> {
    compile_with_identity(raw, None)
}

pub(super) fn compile_in_package(
    raw: &str,
    vocabulary_id: &str,
    version: &str,
) -> Result<VocabularyModule, VocabularyError> {
    compile_with_identity(raw, Some((vocabulary_id, version)))
}

fn compile_with_identity(
    raw: &str,
    identity: Option<(&str, &str)>,
) -> Result<VocabularyModule, VocabularyError> {
    let mut language = Fields::new();
    let mut terms = Vec::new();
    let mut types = Vec::new();
    let mut roles = Vec::new();
    let mut surfaces = Vec::new();
    let mut current: Option<(Section, Fields, usize)> = None;

    for (offset, source_line) in raw.lines().enumerate() {
        let line_number = offset + 1;
        let line = source_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(title) = line.strip_prefix('#') {
            finish(
                current.take(),
                &mut language,
                &mut terms,
                &mut types,
                &mut roles,
                &mut surfaces,
                identity,
            )?;
            let section = match title.trim() {
                "言語" | "Language" => Section::Language,
                "用語" | "Term" => Section::Term,
                "型" | "Type" => Section::Type,
                "役割" | "Role" => Section::Role,
                "表現" | "Surface" => Section::Surface,
                other => return invalid(line_number, format!("unknown section: {other}")),
            };
            current = Some((section, Fields::new(), line_number));
            continue;
        }
        let Some((_, fields, _)) = current.as_mut() else {
            return invalid(line_number, "a field must follow a section heading");
        };
        let Some((key, value)) = line.split_once(':') else {
            return invalid(line_number, "a field must use `name: value`");
        };
        let key = canonical_field_name(key.trim());
        let value = value.trim();
        if key.is_empty() || fields.insert(key.into(), value.into()).is_some() {
            return invalid(line_number, "field name must be non-empty and unique");
        }
    }
    finish(
        current,
        &mut language,
        &mut terms,
        &mut types,
        &mut roles,
        &mut surfaces,
        identity,
    )?;
    if language.is_empty() {
        return invalid(1, "exactly one Language section is required");
    }
    reject_unknown(&language, &["識別子", "版", "依存"])?;
    let (vocabulary_id, version) = match identity {
        Some((vocabulary_id, version)) => (vocabulary_id.to_owned(), version.to_owned()),
        None => (take(&language, "識別子")?, take(&language, "版")?),
    };
    Ok(VocabularyModule {
        schema_version: "0.10.0".into(),
        vocabulary_id,
        version,
        dependencies: list(optional(&language, "依存").unwrap_or("なし")),
        terms,
        types,
        roles,
        lexical_bindings: surfaces,
    })
}

fn canonical_field_name(value: &str) -> &str {
    match value {
        "Identifier" => "識別子",
        "Version" => "版",
        "Dependencies" => "依存",
        "Name" => "名称",
        "Description" => "説明",
        "Meaning" => "意味",
        "Language" => "言語",
        "Surface" => "表記",
        "Normalized" => "正規形",
        "Kind" => "種別",
        "Target" => "対象",
        "Semantic type" => "意味型",
        "Priority" => "優先度",
        "Valid from" => "有効開始",
        "Valid until" => "有効終了",
        "Provenance" => "出典",
        other => other,
    }
}

pub(super) fn render(module: &VocabularyModule) -> Result<String, VocabularyError> {
    render_with_identity(module, true)
}

pub(super) fn render_in_package(module: &VocabularyModule) -> Result<String, VocabularyError> {
    render_with_identity(module, false)
}

fn render_with_identity(
    module: &VocabularyModule,
    include_identity: bool,
) -> Result<String, VocabularyError> {
    let mut output = String::new();
    let term_names = module
        .terms
        .iter()
        .map(|term| (term.id, term.label.as_str()))
        .collect::<BTreeMap<_, _>>();
    output.push_str("# 言語\n");
    if include_identity {
        field(&mut output, "識別子", &module.vocabulary_id);
        field(&mut output, "版", &module.version);
    }
    field(
        &mut output,
        "依存",
        &if module.dependencies.is_empty() {
            "なし".into()
        } else {
            module.dependencies.join(", ")
        },
    );
    for term in &module.terms {
        if term.deprecation.is_some() || term.replacement.is_some() {
            return Err(VocabularyError::InvalidFormat(
                "deprecated/replaced terms must be migrated before rendering source".into(),
            ));
        }
        if term.id != TermId::from_declaration(&module.vocabulary_id, &term.label) {
            return Err(VocabularyError::InvalidFormat(format!(
                "term `{}` does not have its compiler-derived identity",
                term.label
            )));
        }
        output.push_str("\n# 用語\n");
        field(&mut output, "名称", &term.label);
        field(&mut output, "説明", &term.description);
        let mut arguments = Vec::new();
        let semantic = match &term.semantic_definition {
            None => "なし".into(),
            Some(SemanticDefinition::Primitive(value)) => format!("Primitive/{value:?}"),
            Some(SemanticDefinition::Alias(value)) => {
                format!("Alias/{}", local_term_name(&term_names, *value)?)
            }
            Some(SemanticDefinition::Composition(SemanticExpr::Apply {
                predicate,
                arguments: values,
            })) => {
                for (role, expression) in values {
                    arguments.push((
                        format!("引数/{}", role.to_hex()),
                        render_expression(expression, &term_names)?,
                    ));
                }
                format!(
                    "Composition/Apply/{}",
                    local_term_name(&term_names, *predicate)?
                )
            }
            Some(_) => {
                return Err(VocabularyError::InvalidFormat(
                    "this semantic definition has no authored source form".into(),
                ));
            }
        };
        field(&mut output, "意味", &semantic);
        for (name, value) in arguments {
            field(&mut output, &name, &value);
        }
    }
    for ty in &module.types {
        if ty.deprecation.is_some() {
            return Err(VocabularyError::InvalidFormat(
                "deprecated types must be migrated before rendering source".into(),
            ));
        }
        output.push_str("\n# 型\n");
        field(&mut output, "TypeId", &ty.id.to_hex());
        field(&mut output, "名称", &ty.label);
        field(
            &mut output,
            "上位型",
            &if ty.super_types.is_empty() {
                "なし".into()
            } else {
                ty.super_types
                    .iter()
                    .map(TypeId::to_hex)
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        );
        field(&mut output, "説明", &ty.description);
    }
    for role in &module.roles {
        output.push_str("\n# 役割\n");
        field(&mut output, "RoleId", &role.id.to_hex());
        field(&mut output, "名称", &role.label);
        field(
            &mut output,
            "多重度",
            &match role.cardinality {
                Cardinality::Bounded { min, max } => format!("Bounded/{min}/{max}"),
                other => format!("{other:?}"),
            },
        );
        field(&mut output, "説明", &role.description);
    }
    for binding in &module.lexical_bindings {
        if binding.grammatical_condition.is_some() {
            return Err(VocabularyError::InvalidFormat(
                "grammatical conditions need a dedicated authored syntax before migration".into(),
            ));
        }
        output.push_str("\n# 表現\n");
        field(&mut output, "言語", &binding.language_tag);
        field(&mut output, "表記", &binding.surface_form);
        field(&mut output, "正規形", &binding.normalized_form);
        field(
            &mut output,
            "種別",
            &format!("{:?}", binding.contribution_kind),
        );
        field(
            &mut output,
            "対象",
            local_term_name(&term_names, binding.target_term)?,
        );
        if let Some(value) = binding.semantic_type_condition {
            field(&mut output, "意味型", &value.to_hex());
        }
        field(&mut output, "優先度", &binding.priority.to_string());
        if let Some(value) = binding.valid_from {
            field(&mut output, "有効開始", &value.to_string());
        }
        if let Some(value) = binding.valid_until {
            field(&mut output, "有効終了", &value.to_string());
        }
        field(&mut output, "出典", &binding.provenance);
    }
    Ok(output)
}

fn field(output: &mut String, name: &str, value: &str) {
    output.push_str("  ");
    output.push_str(name);
    output.push_str(": ");
    output.push_str(value);
    output.push('\n');
}

fn finish(
    current: Option<(Section, Fields, usize)>,
    language: &mut Fields,
    terms: &mut Vec<TermDefinition>,
    types: &mut Vec<TypeDefinition>,
    roles: &mut Vec<RoleDefinition>,
    surfaces: &mut Vec<LexicalBinding>,
    identity: Option<(&str, &str)>,
) -> Result<(), VocabularyError> {
    let Some((section, fields, line)) = current else {
        return Ok(());
    };
    match section {
        Section::Language => {
            if !language.is_empty() {
                return invalid(line, "Language section is duplicated");
            }
            if let Some((vocabulary_id, version)) = identity {
                if fields.contains_key("識別子") || fields.contains_key("版") {
                    return invalid(
                        line,
                        "package-owned language identity and version come from its path and manifest",
                    );
                }
                *language = fields;
                language.insert("識別子".into(), vocabulary_id.into());
                language.insert("版".into(), version.into());
            } else {
                *language = fields;
            }
        }
        Section::Term => {
            reject_unknown_with_prefix(&fields, &["名称", "説明", "意味"], "引数/")?;
            let vocabulary_id = language_id(language, line)?;
            let dependencies = language_dependencies(language);
            let label = take(&fields, "名称")?;
            terms.push(TermDefinition {
                id: TermId::from_declaration(vocabulary_id, &label),
                label,
                description: take(&fields, "説明")?,
                deprecation: None,
                replacement: None,
                semantic_definition: semantic(
                    optional(&fields, "意味").unwrap_or("なし"),
                    &fields,
                    vocabulary_id,
                    &dependencies,
                    line,
                )?,
            });
        }
        Section::Type => {
            reject_unknown(&fields, &["TypeId", "名称", "上位型", "説明"])?;
            types.push(TypeDefinition {
                id: type_id(&take(&fields, "TypeId")?, line)?,
                label: take(&fields, "名称")?,
                super_types: list(optional(&fields, "上位型").unwrap_or("なし"))
                    .into_iter()
                    .map(|value| type_id(&value, line))
                    .collect::<Result<_, _>>()?,
                description: take(&fields, "説明")?,
                deprecation: None,
            });
        }
        Section::Role => {
            reject_unknown(&fields, &["RoleId", "名称", "多重度", "説明"])?;
            roles.push(RoleDefinition {
                id: role_id(&take(&fields, "RoleId")?, line)?,
                label: take(&fields, "名称")?,
                cardinality: cardinality(&take(&fields, "多重度")?, line)?,
                description: take(&fields, "説明")?,
            });
        }
        Section::Surface => {
            reject_unknown(
                &fields,
                &[
                    "言語",
                    "表記",
                    "正規形",
                    "種別",
                    "対象",
                    "意味型",
                    "優先度",
                    "有効開始",
                    "有効終了",
                    "出典",
                ],
            )?;
            let vocabulary_id = language_id(language, line)?;
            let dependencies = language_dependencies(language);
            let target = take(&fields, "対象")?;
            surfaces.push(LexicalBinding {
                language_tag: take(&fields, "言語")?,
                surface_form: take(&fields, "表記")?,
                normalized_form: take(&fields, "正規形")?,
                contribution_kind: take(&fields, "種別")?.parse()?,
                target_term: term_reference(&target, vocabulary_id, &dependencies, line)?,
                grammatical_condition: None,
                semantic_type_condition: optional(&fields, "意味型")
                    .filter(|value| *value != "なし")
                    .map(|value| type_id(value, line))
                    .transpose()?,
                priority: number(&take(&fields, "優先度")?, line, "priority")?,
                valid_from: signed_optional(&fields, "有効開始", line)?,
                valid_until: signed_optional(&fields, "有効終了", line)?,
                provenance: take(&fields, "出典")?,
            });
        }
    }
    Ok(())
}

fn semantic(
    value: &str,
    fields: &Fields,
    vocabulary_id: &str,
    dependencies: &[String],
    line: usize,
) -> Result<Option<SemanticDefinition>, VocabularyError> {
    if is_none(value) {
        return Ok(None);
    }
    if let Some(value) = value.strip_prefix("Primitive/") {
        let primitive = match value {
            "Value" => PrimitiveSemantic::Value,
            "Type" => PrimitiveSemantic::Type,
            "Predicate" => PrimitiveSemantic::Predicate,
            "Relation" => PrimitiveSemantic::Relation,
            "Role" => PrimitiveSemantic::Role,
            "Proposition" => PrimitiveSemantic::Proposition,
            "Reference" => PrimitiveSemantic::Reference,
            "Variable" => PrimitiveSemantic::Variable,
            "Constraint" => PrimitiveSemantic::Constraint,
            "Composition" => PrimitiveSemantic::Composition,
            "Quantification" => PrimitiveSemantic::Quantification,
            "Polarity" => PrimitiveSemantic::Polarity,
            "Modality" => PrimitiveSemantic::Modality,
            "Temporal" => PrimitiveSemantic::Temporal,
            "Spatial" => PrimitiveSemantic::Spatial,
            "Causality" => PrimitiveSemantic::Causality,
            other => return invalid(line, format!("unknown primitive semantic: {other}")),
        };
        return Ok(Some(SemanticDefinition::Primitive(primitive)));
    }
    if let Some(value) = value.strip_prefix("Alias/") {
        return Ok(Some(SemanticDefinition::Alias(term_reference(
            value,
            vocabulary_id,
            dependencies,
            line,
        )?)));
    }
    if let Some(predicate) = value.strip_prefix("Composition/Apply/") {
        let mut arguments = Vec::new();
        for (key, value) in fields.iter().filter(|(key, _)| key.starts_with("引数/")) {
            arguments.push((
                role_id(key.trim_start_matches("引数/"), line)?,
                Box::new(expression(value, vocabulary_id, dependencies, line)?),
            ));
        }
        if arguments.is_empty() {
            return invalid(
                line,
                "Composition/Apply requires at least one `引数/<RoleId>` field",
            );
        }
        return Ok(Some(SemanticDefinition::Composition(SemanticExpr::Apply {
            predicate: term_reference(predicate, vocabulary_id, dependencies, line)?,
            arguments,
        })));
    }
    invalid(
        line,
        "semantic must be `なし`, `Primitive/<kind>`, `Alias/<term>`, or `Composition/Apply/<term>`",
    )
}

fn expression(
    value: &str,
    vocabulary_id: &str,
    dependencies: &[String],
    line: usize,
) -> Result<SemanticExpr, VocabularyError> {
    for (prefix, constructor) in [
        (
            "Primitive/",
            SemanticExpr::Primitive as fn(TermId) -> SemanticExpr,
        ),
        (
            "Concept/",
            SemanticExpr::Concept as fn(TermId) -> SemanticExpr,
        ),
    ] {
        if let Some(id) = value.strip_prefix(prefix) {
            return Ok(constructor(term_reference(
                id,
                vocabulary_id,
                dependencies,
                line,
            )?));
        }
    }
    if let Some(value) = value.strip_prefix("Variable/") {
        return Ok(SemanticExpr::Variable(value.into()));
    }
    if let Some(value) = value.strip_prefix("Literal/") {
        return Ok(SemanticExpr::Literal(value.into()));
    }
    invalid(line, "unsupported semantic expression")
}

fn render_expression(
    value: &SemanticExpr,
    term_names: &BTreeMap<TermId, &str>,
) -> Result<String, VocabularyError> {
    match value {
        SemanticExpr::Primitive(id) => {
            Ok(format!("Primitive/{}", local_term_name(term_names, *id)?))
        }
        SemanticExpr::Concept(id) => Ok(format!("Concept/{}", local_term_name(term_names, *id)?)),
        SemanticExpr::Variable(value) => Ok(format!("Variable/{value}")),
        SemanticExpr::Literal(value) => Ok(format!("Literal/{value}")),
        _ => Err(VocabularyError::InvalidFormat(
            "nested semantic expression has no authored source form".into(),
        )),
    }
}

fn cardinality(value: &str, line: usize) -> Result<Cardinality, VocabularyError> {
    match value {
        "Zero" => Ok(Cardinality::Zero),
        "ZeroOrOne" => Ok(Cardinality::ZeroOrOne),
        "ExactlyOne" => Ok(Cardinality::ExactlyOne),
        "ZeroOrMany" => Ok(Cardinality::ZeroOrMany),
        "OneOrMany" => Ok(Cardinality::OneOrMany),
        other if other.starts_with("Bounded/") => {
            let Some((min, max)) = other[8..].split_once('/') else {
                return invalid(line, "bounded cardinality must be Bounded/<min>/<max>");
            };
            Ok(Cardinality::Bounded {
                min: number(min, line, "minimum cardinality")?,
                max: number(max, line, "maximum cardinality")?,
            })
        }
        other => invalid(line, format!("unknown cardinality: {other}")),
    }
}

fn list(value: &str) -> Vec<String> {
    if is_none(value) {
        Vec::new()
    } else {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from)
            .collect()
    }
}

fn is_none(value: &str) -> bool {
    value == "なし" || value.eq_ignore_ascii_case("none")
}

fn language_id(language: &Fields, line: usize) -> Result<&str, VocabularyError> {
    optional(language, "識別子")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| line_error(line, "Language section must precede semantic declarations"))
}

fn language_dependencies(language: &Fields) -> Vec<String> {
    list(optional(language, "依存").unwrap_or("なし"))
}

fn term_reference(
    value: &str,
    vocabulary_id: &str,
    dependencies: &[String],
    line: usize,
) -> Result<TermId, VocabularyError> {
    let (owner, name) = value
        .split_once("::")
        .map_or((vocabulary_id, value), |(owner, name)| (owner, name));
    if owner.is_empty() || name.is_empty() || name.trim() != name {
        return invalid(line, "term reference must be a canonical name");
    }
    if owner != vocabulary_id && !dependencies.iter().any(|dependency| dependency == owner) {
        return invalid(
            line,
            format!("term reference owner `{owner}` is not a declared dependency"),
        );
    }
    Ok(TermId::from_declaration(owner, name))
}

fn local_term_name<'a>(
    term_names: &'a BTreeMap<TermId, &'a str>,
    term_id: TermId,
) -> Result<&'a str, VocabularyError> {
    term_names.get(&term_id).copied().ok_or_else(|| {
        VocabularyError::InvalidFormat(
            "rendering an external term requires its selected definition package".into(),
        )
    })
}

fn take(fields: &Fields, key: &str) -> Result<String, VocabularyError> {
    fields
        .get(key)
        .cloned()
        .ok_or_else(|| VocabularyError::InvalidFormat(format!("missing field `{key}`")))
}

fn optional<'a>(fields: &'a Fields, key: &str) -> Option<&'a str> {
    fields.get(key).map(String::as_str)
}

fn reject_unknown(fields: &Fields, allowed: &[&str]) -> Result<(), VocabularyError> {
    if let Some(key) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(VocabularyError::InvalidFormat(format!(
            "unknown language-source field `{key}`"
        )));
    }
    Ok(())
}

fn reject_unknown_with_prefix(
    fields: &Fields,
    allowed: &[&str],
    prefix: &str,
) -> Result<(), VocabularyError> {
    if let Some(key) = fields
        .keys()
        .find(|key| !allowed.contains(&key.as_str()) && !key.starts_with(prefix))
    {
        return Err(VocabularyError::InvalidFormat(format!(
            "unknown language-source field `{key}`"
        )));
    }
    Ok(())
}

fn type_id(value: &str, line: usize) -> Result<TypeId, VocabularyError> {
    TypeId::from_hex(value).map_err(|_| line_error(line, "invalid TypeId"))
}

fn role_id(value: &str, line: usize) -> Result<RoleId, VocabularyError> {
    RoleId::from_hex(value).map_err(|_| line_error(line, "invalid RoleId"))
}

fn number<T: std::str::FromStr>(
    value: &str,
    line: usize,
    name: &str,
) -> Result<T, VocabularyError> {
    value
        .parse()
        .map_err(|_| line_error(line, format!("invalid {name}")))
}

fn signed_optional(
    fields: &Fields,
    key: &str,
    line: usize,
) -> Result<Option<i64>, VocabularyError> {
    optional(fields, key)
        .filter(|value| *value != "なし")
        .map(|value| number(value, line, key))
        .transpose()
}

fn invalid<T>(line: usize, message: impl Into<String>) -> Result<T, VocabularyError> {
    Err(line_error(line, message))
}

fn line_error(line: usize, message: impl Into<String>) -> VocabularyError {
    VocabularyError::InvalidFormat(format!("line {line}: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_explicit_sem_source_and_rejects_json() {
        let source = r#"
# 言語
  識別子: fixture
  版: 0.10.0
  依存: なし

# 用語
  名称: Relation
  説明: explicit relation
  意味: Primitive/Relation

# 表現
  言語: ja-JP
  表記: 関係
  正規形: 関係
  種別: Relational
  対象: Relation
  優先度: 100
  出典: fixture
"#;
        let module = compile(source).unwrap();
        assert_eq!(module.vocabulary_id, "fixture");
        assert_eq!(module.terms.len(), 1);
        assert_eq!(module.lexical_bindings.len(), 1);
        assert_eq!(
            module.terms[0].id,
            TermId::from_declaration("fixture", "Relation")
        );
        assert!(!module.to_sem().unwrap().contains("TermId"));
        assert!(
            compile(&source.replace(
                "  名称: Relation\n",
                "  TermId: 42000000000000000000000000000001\n  名称: Relation\n"
            ))
            .is_err()
        );
        assert!(compile(r#"{"version":"0.10.0"}"#).is_err());
    }

    #[test]
    fn accepts_localized_or_english_schema_labels_without_changing_meaning() {
        let source = r#"
# Language
  Identifier: fixture/english
  Version: 0.10.0
  Dependencies: none

# Term
  Name: Request
  Description: request
  Meaning: Primitive/Proposition

# Surface
  Language: ja-JP
  Surface: 依頼
  Normalized: 依頼
  Kind: Structural
  Target: Request
  Priority: 100
  Provenance: fixture
"#;
        let module = compile(source).unwrap();
        assert_eq!(module.vocabulary_id, "fixture/english");
        assert_eq!(module.terms[0].label, "Request");
        assert_eq!(module.lexical_bindings[0].surface_form, "依頼");
    }
}
