//! Vocabulary module definitions, external loading, and registry validation.

mod definition_validation;
pub mod document;
mod source;

use sem_lang_core::{RoleId, TermId, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocabularyModule {
    pub schema_version: String,
    pub vocabulary_id: String,
    pub version: String,
    pub dependencies: Vec<String>,
    pub terms: Vec<TermDefinition>,
    pub types: Vec<TypeDefinition>,
    pub roles: Vec<RoleDefinition>,
    pub lexical_bindings: Vec<LexicalBinding>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermDefinition {
    pub id: TermId,
    pub label: String,
    pub description: String,
    pub deprecation: Option<Deprecation>,
    pub replacement: Option<Replacement>,
    pub semantic_definition: Option<sem_lang_linguistics::SemanticDefinition>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeDefinition {
    pub id: TypeId,
    pub label: String,
    pub super_types: Vec<TypeId>,
    pub description: String,
    pub deprecation: Option<Deprecation>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDefinition {
    pub id: RoleId,
    pub label: String,
    pub cardinality: Cardinality,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LexicalBinding {
    pub language_tag: String,
    pub surface_form: String,
    pub normalized_form: String,
    pub contribution_kind: ContributionKind,
    pub target_term: TermId,
    pub grammatical_condition: Option<sem_lang_linguistics::SurfacePattern>,
    pub semantic_type_condition: Option<TypeId>,
    pub priority: u32,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub provenance: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deprecation {
    pub reason: String,
    pub deprecated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    pub replacement_term: TermId,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Cardinality {
    Zero,
    ZeroOrOne,
    ExactlyOne,
    ZeroOrMany,
    OneOrMany,
    Bounded { min: usize, max: usize },
}

impl Cardinality {
    /// Number of distinct role bindings, not the size of a structured Value.
    pub fn bounds(self) -> (usize, Option<usize>) {
        match self {
            Self::Zero => (0, Some(0)),
            Self::ZeroOrOne => (0, Some(1)),
            Self::ExactlyOne => (1, Some(1)),
            Self::ZeroOrMany => (0, None),
            Self::OneOrMany => (1, None),
            Self::Bounded { min, max } => (min, Some(max)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ContributionKind {
    Denotational,
    Predicative,
    Relational,
    Operational,
    Structural,
    Temporal,
    Modal,
    Polarity,
    Referential,
    Pragmatic,
    Evidential,
    Dialogue,
}

impl FromStr for ContributionKind {
    type Err = VocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "Denotational" => Ok(Self::Denotational),
            "Predicative" => Ok(Self::Predicative),
            "Relational" => Ok(Self::Relational),
            "Operational" => Ok(Self::Operational),
            "Structural" => Ok(Self::Structural),
            "Temporal" => Ok(Self::Temporal),
            "Modal" => Ok(Self::Modal),
            "Polarity" => Ok(Self::Polarity),
            "Referential" => Ok(Self::Referential),
            "Pragmatic" => Ok(Self::Pragmatic),
            "Evidential" => Ok(Self::Evidential),
            "Dialogue" => Ok(Self::Dialogue),
            other => Err(VocabularyError::InvalidFormat(format!(
                "unknown contribution kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VocabularyRegistry {
    modules: BTreeMap<String, VocabularyModule>,
}

#[derive(Debug, Clone)]
pub struct CrossReferenceError {
    pub location: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum VocabularyError {
    DuplicateModule(String),
    UnknownDependency(String),
    CrossReference(CrossReferenceError),
    OntologyCycle(Vec<String>),
    InvalidFormat(String),
}

impl Display for VocabularyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateModule(id) => write!(f, "duplicate module {id}"),
            Self::UnknownDependency(dep) => write!(f, "unknown dependency {dep}"),
            Self::CrossReference(c) => {
                write!(f, "cross reference error at {}: {}", c.location, c.message)
            }
            Self::OntologyCycle(c) => write!(f, "ontology cycle {c:?}"),
            Self::InvalidFormat(message) => {
                write!(f, "invalid language-definition source: {message}")
            }
        }
    }
}

impl std::error::Error for VocabularyError {}

impl VocabularyModule {
    pub fn manifest_text(&self) -> String {
        format!(
            "{}:{}:{}",
            self.vocabulary_id, self.schema_version, self.version
        )
    }

    /// Compile an authored sem-lang language-definition source.
    ///
    /// JSON is deliberately not accepted here. JSON remains available for
    /// bounded wire contracts and receipts, but it is not a language source.
    pub fn from_sem(raw: &str) -> Result<Self, VocabularyError> {
        let module = source::compile(raw)?;
        module.validate_version()?;
        Ok(module)
    }

    /// Compile a language source whose identity and version are derived from
    /// its selected package path. Package-owned sources must not repeat either
    /// value in authored `.sem` text.
    pub fn from_package_sem(
        vocabulary_id: &str,
        version: &str,
        raw: &str,
    ) -> Result<Self, VocabularyError> {
        let module = source::compile_in_package(raw, vocabulary_id, version)?;
        module.validate_version()?;
        Ok(module)
    }

    /// Render the canonical, reviewable source form used as a project input.
    pub fn to_sem(&self) -> Result<String, VocabularyError> {
        self.validate_version()?;
        source::render(self)
    }

    /// Render a package-owned source. Its package-relative path carries the
    /// identity and its package manifest carries the version.
    pub fn to_package_sem(&self) -> Result<String, VocabularyError> {
        self.validate_version()?;
        source::render_in_package(self)
    }

    fn validate_version(&self) -> Result<(), VocabularyError> {
        if self.schema_version != "0.10.0" || self.version != "0.10.0" {
            return Err(VocabularyError::InvalidFormat(
                "unsupported declaration version; explicit source migration required".into(),
            ));
        }
        Ok(())
    }
}

impl VocabularyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Imported meaning is explicit and direct, never ambient registry state.
    /// The containing adopted revision pins the complete module set; names here
    /// select within that set and do not discover a newer external dependency.
    fn visible_modules<'a>(
        &'a self,
        module: &'a VocabularyModule,
    ) -> Result<Vec<&'a VocabularyModule>, VocabularyError> {
        let mut visible = vec![module];
        let mut seen = BTreeSet::new();
        for id in &module.dependencies {
            if id == &module.vocabulary_id || !seen.insert(id) {
                return Err(VocabularyError::InvalidFormat(
                    "duplicate or self dependency".into(),
                ));
            }
            visible.push(
                self.get(id)
                    .ok_or_else(|| VocabularyError::UnknownDependency(id.clone()))?,
            );
        }
        Ok(visible)
    }

    pub fn register(&mut self, module: VocabularyModule) -> Result<(), VocabularyError> {
        module.validate_version()?;
        if self.modules.contains_key(&module.vocabulary_id) {
            return Err(VocabularyError::DuplicateModule(module.vocabulary_id));
        }
        if module.roles.iter().any(|role| {
            let (min, max) = role.cardinality.bounds();
            max.is_some_and(|max| min > max)
        }) {
            return Err(VocabularyError::InvalidFormat(
                "invalid cardinality range".into(),
            ));
        }
        let term_ids: BTreeSet<TermId> = module.terms.iter().map(|t| t.id).collect();
        let type_ids: BTreeSet<TypeId> = module.types.iter().map(|t| t.id).collect();
        let role_ids: BTreeSet<RoleId> = module.roles.iter().map(|role| role.id).collect();
        if term_ids.len() != module.terms.len()
            || type_ids.len() != module.types.len()
            || role_ids.len() != module.roles.len()
            || self.modules().any(|existing| {
                existing
                    .terms
                    .iter()
                    .any(|term| term_ids.contains(&term.id))
                    || existing.types.iter().any(|ty| type_ids.contains(&ty.id))
                    || existing
                        .roles
                        .iter()
                        .any(|role| role_ids.contains(&role.id))
            })
        {
            return Err(VocabularyError::InvalidFormat(
                "duplicate semantic identity".into(),
            ));
        }

        let visible = self.visible_modules(&module)?;
        let visible_terms: BTreeSet<_> = visible
            .iter()
            .flat_map(|m| m.terms.iter().map(|t| t.id))
            .collect();
        let visible_types: BTreeSet<_> = visible
            .iter()
            .flat_map(|m| m.types.iter().map(|t| t.id))
            .collect();
        for binding in &module.lexical_bindings {
            if !visible_terms.contains(&binding.target_term) {
                return Err(VocabularyError::CrossReference(CrossReferenceError {
                    location: binding.surface_form.clone(),
                    message: format!("unknown target term {:?}", binding.target_term),
                }));
            }
            if let Some(ty) = binding.semantic_type_condition
                && !visible_types.contains(&ty)
            {
                return Err(VocabularyError::CrossReference(CrossReferenceError {
                    location: binding.surface_form.clone(),
                    message: format!("unknown semantic type {:?}", ty),
                }));
            }
        }

        for ty in &module.types {
            for parent in &ty.super_types {
                if !visible_types.contains(parent) {
                    return Err(VocabularyError::CrossReference(CrossReferenceError {
                        location: ty.label.clone(),
                        message: format!("unknown super type {:?}", parent),
                    }));
                }
            }
        }

        if has_cycle(&module.types) {
            return Err(VocabularyError::OntologyCycle(vec![
                module.vocabulary_id.clone(),
            ]));
        }
        self.modules.insert(module.vocabulary_id.clone(), module);
        Ok(())
    }

    pub fn register_sem(&mut self, raw: &str) -> Result<(), VocabularyError> {
        self.register(VocabularyModule::from_sem(raw)?)
    }

    pub fn get(&self, id: &str) -> Option<&VocabularyModule> {
        self.modules.get(id)
    }

    pub fn modules(&self) -> impl Iterator<Item = &VocabularyModule> {
        self.modules.values()
    }
}

fn has_cycle(types: &[TypeDefinition]) -> bool {
    let mut seen = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    let map: BTreeMap<_, _> = types
        .iter()
        .map(|ty| (ty.id, ty.super_types.clone()))
        .collect();

    fn walk(
        root: TypeId,
        map: &BTreeMap<TypeId, Vec<TypeId>>,
        visiting: &mut BTreeSet<TypeId>,
        seen: &mut BTreeSet<TypeId>,
    ) -> bool {
        if seen.contains(&root) {
            return false;
        }
        if !visiting.insert(root) {
            return true;
        }
        for parent in map.get(&root).into_iter().flatten() {
            if walk(*parent, map, visiting, seen) {
                return true;
            }
        }
        visiting.remove(&root);
        seen.insert(root);
        false
    }

    types
        .iter()
        .any(|ty| walk(ty.id, &map, &mut visiting, &mut seen))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_source_compiles_identity_and_rejects_json_input() {
        let source = r#"# 言語
  識別子: fixture
  版: 0.10.0
  依存: なし

# 用語
  名称: display label
  説明: explicit meaning
  意味: Primitive/Relation

# 表現
  言語: en
  表記: do
  正規形: do
  種別: Operational
  対象: display label
  優先度: 7
  有効開始: 100
  有効終了: 200
  出典: declaration:fixture
"#;
        let module = VocabularyModule::from_sem(source).unwrap();
        let mut registry = VocabularyRegistry::new();
        registry.register_sem(source).unwrap();
        assert_eq!(
            registry.get("fixture").unwrap().terms[0].id,
            TermId::from_declaration("fixture", "display label")
        );
        assert_eq!(
            VocabularyModule::from_sem(&module.to_sem().unwrap())
                .unwrap()
                .manifest_text(),
            module.manifest_text()
        );
        assert!(VocabularyModule::from_sem(r#"{"version":"0.10.0"}"#).is_err());
        assert!(VocabularyModule::from_sem(&source.replace("版: 0.10.0", "版: 0.1.0")).is_err());
        assert!(
            VocabularyModule::from_sem(&source.replace("名称: display label", "unknown: value"))
                .is_err()
        );
    }
}
