//! Derived language resources and immutable compiled language artifacts.
mod types;
pub use types::TypeEvaluationError;

mod pipeline;
pub use pipeline::{PipelineOperator, PipelineShapeError, validate_pipeline_shape};

use sem_lang_core::{Digest, TermId};
use sem_lang_linguistics::{
    EvidenceSet, ExpansionError, KnowledgeOrigin, LinguisticKnowledge, LinguisticKnowledgeEntry,
    OpaqueReason, SemanticContribution, SemanticDefinition, SemanticExpander,
    SemanticExpansionTrace, SemanticExpr, SemanticResidual, SenseDefinition,
};
use sem_lang_vocab::{ContributionKind, VocabularyRegistry};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct LexicalResource {
    pub surface: String,
    pub semantic: SemanticContribution,
    pub origin: KnowledgeOrigin,
    pub provenance: String,
}

#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    pub entries: Vec<LexicalResource>,
}

#[derive(Debug, Clone, Default)]
pub struct Morphology {
    pub rules: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConstructionSet {
    pub constructions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticBindings {
    pub bindings: Vec<(String, TermId)>,
}

#[derive(Debug, Clone, Default)]
pub struct PragmaticBindings {
    pub bindings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DisambiguationRules {
    pub rules: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GeneratedLanguageResources {
    pub lexicon: Lexicon,
    pub morphology: Morphology,
    pub construction_set: ConstructionSet,
    pub semantic_bindings: SemanticBindings,
    pub pragmatic_bindings: PragmaticBindings,
    pub disambiguation_rules: DisambiguationRules,
    pub senses: CompiledSenseIndex,
    pub semantic_definitions: CompiledSemanticDefinitions,
    pub expansions: CompiledExpansionTable,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledSenseIndex {
    pub senses: Vec<SenseDefinition>,
}

#[derive(Debug, Clone)]
pub struct CompiledSemanticDefinition {
    pub term: TermId,
    pub label: String,
    pub definition: SemanticDefinition,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledSemanticDefinitions {
    pub entries: Vec<CompiledSemanticDefinition>,
}

#[derive(Debug, Clone)]
pub struct CompiledExpansion {
    pub term: TermId,
    pub trace: SemanticExpansionTrace,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledExpansionTable {
    pub entries: Vec<CompiledExpansion>,
}

#[derive(Debug, Clone)]
pub struct ResolvedLinguisticKnowledge {
    pub revision: String,
    pub entries: Vec<LinguisticKnowledgeEntry>,
}

pub struct KnowledgeResolver;

impl KnowledgeResolver {
    pub fn resolve(
        knowledge: &LinguisticKnowledge,
        revision: impl Into<String>,
    ) -> ResolvedLinguisticKnowledge {
        let mut entries: Vec<_> = knowledge
            .all_entries()
            .filter(|entry| entry.validity == sem_lang_linguistics::Validity::Active)
            .cloned()
            .collect();
        entries.sort_by_key(|entry| entry.id);
        ResolvedLinguisticKnowledge {
            revision: revision.into(),
            entries,
        }
    }
}

pub struct LanguageResourceBuilder;

impl LanguageResourceBuilder {
    pub fn build(
        resolved: &ResolvedLinguisticKnowledge,
        registry: &VocabularyRegistry,
    ) -> GeneratedLanguageResources {
        let mut lexicon = Vec::new();
        for module in registry.modules() {
            for binding in &module.lexical_bindings {
                lexicon.push(LexicalResource {
                    surface: binding.surface_form.clone(),
                    semantic: SemanticContribution {
                        concept: binding.target_term,
                        roles: Vec::new(),
                        polarity: None,
                        modality: None,
                    },
                    origin: KnowledgeOrigin::Curated,
                    provenance: binding.provenance.clone(),
                });
            }
        }
        for entry in &resolved.entries {
            if let Some(form) = &entry.form {
                lexicon.push(LexicalResource {
                    surface: form.surface.clone(),
                    semantic: entry.semantics.clone(),
                    origin: entry.origin,
                    provenance: format!("linguistic-knowledge:{}", entry.id),
                });
            }
        }
        lexicon.sort_by(|left, right| left.surface.cmp(&right.surface));
        let mut construction_set = Vec::new();
        let mut semantic_bindings = Vec::new();
        let mut pragmatic_bindings = Vec::new();
        for entry in &resolved.entries {
            if let Some(form) = &entry.form
                && let Some(construction) = &form.construction
            {
                construction_set.push(construction.clone());
            }
            semantic_bindings.push((entry.id.to_string(), entry.semantics.concept));
            pragmatic_bindings.extend(
                entry
                    .pragmatics
                    .iter()
                    .map(|item| format!("{}={}", item.feature, item.value)),
            );
        }
        construction_set.sort();
        semantic_bindings.sort_by(|left, right| left.0.cmp(&right.0));
        pragmatic_bindings.sort();
        GeneratedLanguageResources {
            lexicon: Lexicon { entries: lexicon },
            morphology: Morphology::default(),
            construction_set: ConstructionSet {
                constructions: construction_set,
            },
            semantic_bindings: SemanticBindings {
                bindings: semantic_bindings,
            },
            pragmatic_bindings: PragmaticBindings {
                bindings: pragmatic_bindings,
            },
            disambiguation_rules: DisambiguationRules::default(),
            senses: CompiledSenseIndex {
                senses: builtin_senses(registry),
            },
            semantic_definitions: CompiledSemanticDefinitions {
                entries: semantic_definitions(registry)
                    .into_iter()
                    .map(|(term, (label, definition))| CompiledSemanticDefinition {
                        term,
                        label,
                        definition,
                    })
                    .collect(),
            },
            expansions: CompiledExpansionTable {
                entries: compiled_expansions(registry),
            },
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LanguageBuildError {
    #[error("invalid linguistic knowledge: {0}")]
    InvalidKnowledge(String),
}

#[derive(Debug, Clone)]
pub struct CompiledLanguage {
    pub revision: String,
    pub digest: Digest,
    pub resources: GeneratedLanguageResources,
    registry: VocabularyRegistry,
    semantic_expander: SemanticExpander,
    definitions: BTreeMap<TermId, (String, SemanticDefinition)>,
}

impl CompiledLanguage {
    pub fn build(
        knowledge: &LinguisticKnowledge,
        registry: &VocabularyRegistry,
        revision: impl Into<String>,
    ) -> Result<Self, LanguageBuildError> {
        knowledge
            .validate()
            .map_err(|error| LanguageBuildError::InvalidKnowledge(error.to_string()))?;
        let resolved = KnowledgeResolver::resolve(knowledge, revision);
        let resources = LanguageResourceBuilder::build(&resolved, registry);
        // Definitions and binding constraints affect interpretation even when
        // every label remains unchanged. Include their lossless typed input.
        let mut bytes = serde_json::to_vec(&registry.modules().collect::<Vec<_>>())
            .map_err(|error| LanguageBuildError::InvalidKnowledge(error.to_string()))?;
        bytes.extend(canonical_resource_bytes(&resources));
        let digest = Digest::document_digest(&bytes);
        Ok(Self {
            revision: resolved.revision,
            digest,
            resources,
            registry: registry.clone(),
            semantic_expander: build_semantic_expander(registry),
            definitions: semantic_definitions(registry),
        })
    }

    pub fn embedded(registry: &VocabularyRegistry) -> Result<Self, LanguageBuildError> {
        Self::build(&LinguisticKnowledge::new(), registry, "embedded")
    }

    pub fn registry(&self) -> &VocabularyRegistry {
        &self.registry
    }

    pub fn semantic_expander(&self) -> &SemanticExpander {
        &self.semantic_expander
    }

    pub fn term(&self, label: &str) -> Option<TermId> {
        unique(
            self.definitions
                .iter()
                .filter_map(|(id, (name, _))| (name == label).then_some(*id)),
        )
    }

    pub fn term_by_label(&self, label: &str) -> Option<TermId> {
        self.term(label)
    }

    pub fn type_by_label(&self, label: &str) -> Option<sem_lang_core::TypeId> {
        unique(
            self.registry
                .modules()
                .flat_map(|module| module.types.iter())
                .filter(|item| item.label == label)
                .map(|item| item.id),
        )
    }

    pub fn definition(&self, label: &str) -> Option<&SemanticDefinition> {
        self.definition_for_term(self.term(label)?)
    }

    fn definition_for_term(&self, term: TermId) -> Option<&SemanticDefinition> {
        self.definitions
            .get(&term)
            .map(|(_, definition)| definition)
    }

    fn has_explicit_definition(&self, term: TermId) -> bool {
        self.registry
            .modules()
            .flat_map(|module| module.terms.iter())
            .any(|item| item.id == term && item.semantic_definition.is_some())
    }

    fn lexical_candidates(
        &self,
        text: &str,
        locale: &str,
        evaluation: LexicalEvaluation<'_>,
    ) -> Result<Vec<LexicalCandidate>, SemanticCompileError> {
        let matching = self
            .registry
            .modules()
            .flat_map(|module| module.lexical_bindings.iter())
            .filter(|binding| {
                (binding.language_tag == "*"
                    || binding.language_tag.split(',').any(|tag| tag == locale))
                    && text.contains(&binding.surface_form)
            });
        let mut candidates = Vec::new();
        for binding in matching {
            let mut conditions = Vec::new();
            if let LexicalEvaluation::Current(at, state) = evaluation {
                if binding
                    .valid_from
                    .zip(binding.valid_until)
                    .is_some_and(|(from, until)| from >= until)
                {
                    return Err(SemanticCompileError::InvalidCondition);
                }
                if (binding.valid_from.is_some() || binding.valid_until.is_some()) && at.is_none() {
                    return Err(SemanticCompileError::MissingEvaluationInstant);
                }
                if at.is_some_and(|at| {
                    binding.valid_from.is_some_and(|from| at < from)
                        || binding.valid_until.is_some_and(|until| at >= until)
                }) {
                    continue;
                }
                if let Some(ty) = binding.semantic_type_condition {
                    let value = sem_lang_core::Value::TermRef(binding.target_term);
                    match self.matches_type(&value, state, ty)? {
                        Some(true) => (),
                        Some(false) => continue,
                        None => {
                            conditions.push(sem_lang_core::Constraint::TypeConstraint { value, ty })
                        }
                    }
                }
            }
            let complete_surface = match binding
                .grammatical_condition
                .as_ref()
                .map(|condition| condition.match_surface(text))
                .transpose()?
            {
                Some(sem_lang_linguistics::SurfaceMatch::Complete) => true,
                Some(sem_lang_linguistics::SurfaceMatch::Prefix) => false,
                Some(sem_lang_linguistics::SurfaceMatch::Rejected) => continue,
                None => binding.surface_form == text,
            };
            candidates.push(LexicalCandidate {
                complete_surface,
                locale: if binding.language_tag == "*" {
                    "*"
                } else {
                    locale
                }
                .into(),
                surface: binding.surface_form.clone(),
                normalized_form: binding.normalized_form.clone(),
                target: binding.target_term,
                contribution_kind: binding.contribution_kind.clone(),
                priority: binding.priority,
                conditions,
            });
        }
        candidates.sort_by(|left, right| {
            let left_defined = self.has_explicit_definition(left.target);
            let right_defined = self.has_explicit_definition(right.target);
            right_defined
                .cmp(&left_defined)
                .then_with(|| right.surface.len().cmp(&left.surface.len()))
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.conditions.len().cmp(&right.conditions.len()))
                .then_with(|| left.locale.cmp(&right.locale))
        });
        Ok(candidates)
    }

    pub fn compile_semantics(
        &self,
        text: &str,
        locale: &str,
    ) -> Result<SemanticCompilation, SemanticCompileError> {
        let normalized = text.trim().trim_end_matches(['。', '.', '!', '?']);
        let candidates =
            self.lexical_candidates(normalized, locale, LexicalEvaluation::Definitions)?;
        self.compile_candidates(normalized, locale, candidates)
    }

    /// Current-state selection evaluates applicability before resolving lexical
    /// ambiguity. Definition-only inspection is not proof of current validity.
    /// Never guesses a wall clock or treats unevaluated conditions as true.
    pub fn compile_current_semantics(
        &self,
        text: &str,
        locale: &str,
        at: Option<i64>,
        state: &sem_lang_core::RuntimeState,
    ) -> Result<SemanticCompilation, SemanticCompileError> {
        let normalized = text.trim().trim_end_matches(['。', '.', '!', '?']);
        let candidates =
            self.lexical_candidates(normalized, locale, LexicalEvaluation::Current(at, state))?;
        self.compile_candidates(normalized, locale, candidates)
    }

    fn compile_candidates(
        &self,
        normalized: &str,
        locale: &str,
        candidates: Vec<LexicalCandidate>,
    ) -> Result<SemanticCompilation, SemanticCompileError> {
        let selected = candidates
            .iter()
            .find(|candidate| self.has_explicit_definition(candidate.target))
            .or_else(|| candidates.first())
            .ok_or_else(|| SemanticCompileError::UnknownSurface(normalized.to_string()))?;
        // Alternative declarations of the same meaning are a disjunction:
        // an independently satisfied declaration does not require an unrelated
        // pending condition. Competing meanings remain candidates below.
        let selected = candidates
            .iter()
            .filter(|candidate| {
                candidate.target == selected.target
                    && candidate.surface == selected.surface
                    && candidate.contribution_kind == selected.contribution_kind
            })
            .min_by_key(|candidate| {
                (
                    candidate.conditions.len() + usize::from(!candidate.complete_surface),
                    !candidate.complete_surface,
                )
            })
            .ok_or(SemanticCompileError::InvalidCondition)?;
        let surface = selected.surface.clone();
        let target = selected.target;
        let meanings = candidates
            .iter()
            // Distinct grammatical contributions may compose (e.g. an action
            // and its object). Competing meanings in the same contribution
            // must remain ambiguous regardless of priority or insertion order.
            .filter(|candidate| {
                candidate.surface == surface
                    && candidate.contribution_kind == selected.contribution_kind
            })
            .map(|candidate| candidate.target)
            .collect::<std::collections::BTreeSet<_>>();
        if meanings.len() > 1 {
            return Err(SemanticCompileError::AmbiguousSurface {
                surface,
                candidates: meanings.into_iter().collect(),
            });
        }
        let pragmatic = candidates
            .iter()
            .find(|candidate| matches!(candidate.contribution_kind, ContributionKind::Pragmatic))
            .map(|candidate| candidate.normalized_form.clone())
            .unwrap_or_default();
        let lexeme_id = sem_lang_linguistics::LexemeId::from_surface(&surface, &selected.locale);
        let lexeme = sem_lang_linguistics::Lexeme {
            id: lexeme_id,
            surface: surface.clone(),
            locale: selected.locale.clone(),
        };
        let mut senses = self
            .resources
            .senses
            .senses
            .iter()
            .filter(|sense| sense.lexeme == lexeme_id && sense.semantic_target == target);
        let sense = senses
            .next()
            .ok_or(SemanticCompileError::InvalidSenseReference)?
            .clone();
        if senses.next().is_some() {
            return Err(SemanticCompileError::InvalidSenseReference);
        }
        let trace = self
            .semantic_expander
            .expand(target)
            .map_err(SemanticCompileError::Expansion)?;
        let proposition = trace.result.clone();
        let canonical = proposition.canonical();
        let proposition_digest = Digest::proposition_digest(canonical.as_bytes());
        let interaction_digest =
            Digest::interaction_digest(format!("{locale}:{pragmatic}:{normalized}").as_bytes());
        let residuals = trace.residuals.clone();
        let interaction_act = (!pragmatic.is_empty()).then(|| pragmatic.clone());
        Ok(SemanticCompilation {
            complete_surface: selected.complete_surface,
            surface: surface.clone(),
            lexeme,
            sense,
            concept: target,
            definition: self.definition_for_term(target).cloned(),
            trace,
            normalized: CanonicalSemanticIr {
                expr: proposition,
                proposition_digest,
                interaction_digest,
                residuals,
            },
            pragmatic,
            interaction_act,
            conditions: selected.conditions.clone(),
        })
    }

    pub fn explain(&self, text: &str, locale: &str) -> LanguageExplanation {
        let mut matches = Vec::new();
        for entry in &self.resources.lexicon.entries {
            if text.contains(&entry.surface) {
                matches.push(LanguageMatch {
                    surface: entry.surface.clone(),
                    origin: entry.origin,
                    provenance: entry.provenance.clone(),
                    concept: entry.semantic.concept,
                });
            }
        }
        matches.sort_by(|left, right| left.surface.cmp(&right.surface));
        LanguageExplanation {
            text: text.to_string(),
            locale: locale.to_string(),
            matches,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LanguageMatch {
    pub surface: String,
    pub origin: KnowledgeOrigin,
    pub provenance: String,
    pub concept: TermId,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SemanticCompileError {
    #[error("missing or inconsistent declared sense reference")]
    InvalidSenseReference,
    #[error(transparent)]
    TypeEvaluation(#[from] TypeEvaluationError),
    #[error("current lexical selection requires an evaluation instant")]
    MissingEvaluationInstant,
    #[error(transparent)]
    Construction(#[from] sem_lang_linguistics::ConstructionError),
    #[error("invalid lexical condition")]
    InvalidCondition,
    #[error("unknown surface: {0}")]
    UnknownSurface(String),
    #[error("ambiguous surface: {surface}")]
    AmbiguousSurface {
        surface: String,
        candidates: Vec<TermId>,
    },
    #[error("semantic expansion: {0}")]
    Expansion(#[from] ExpansionError),
}

#[derive(Clone, Copy)]
enum LexicalEvaluation<'a> {
    Definitions,
    Current(Option<i64>, &'a sem_lang_core::RuntimeState),
}

#[derive(Debug, Clone)]
pub struct SemanticCompilation {
    pub complete_surface: bool,
    pub surface: String,
    pub lexeme: sem_lang_linguistics::Lexeme,
    pub sense: SenseDefinition,
    pub concept: TermId,
    pub definition: Option<SemanticDefinition>,
    pub trace: SemanticExpansionTrace,
    pub normalized: CanonicalSemanticIr,
    pub pragmatic: String,
    pub interaction_act: Option<String>,
    /// Exact conditions still awaiting type evidence in current compilation.
    pub conditions: Vec<sem_lang_core::Constraint>,
}

#[derive(Debug, Clone)]
pub struct CanonicalSemanticIr {
    pub expr: SemanticExpr,
    pub proposition_digest: Digest,
    pub interaction_digest: Digest,
    pub residuals: Vec<SemanticResidual>,
}

/// Deterministic evidence produced when comparing two canonical semantic
/// expressions. This is deliberately independent of graph storage: callers
/// may project the result to Zixcel, but sem-lang remains the authority for
/// meaning and candidate generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticMatchMethod {
    ExactCanonical,
    Structural,
    Similarity,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticMatch {
    pub score: f32,
    pub method: SemanticMatchMethod,
}

impl SemanticMatch {
    /// Compare normalized expressions without using an embedding model or
    /// mutable external state. The result is evidence only; promotion is a
    /// separate, authorized operation.
    #[must_use]
    pub fn compare(left: &CanonicalSemanticIr, right: &CanonicalSemanticIr) -> Self {
        let left_expression = left.expr.canonical();
        let right_expression = right.expr.canonical();
        if left_expression == right_expression {
            return Self {
                score: 1.0,
                method: SemanticMatchMethod::ExactCanonical,
            };
        }

        let left_tokens = token_set(&left_expression);
        let right_tokens = token_set(&right_expression);
        let union = left_tokens.union(&right_tokens).count();
        let score = if union == 0 {
            0.0
        } else {
            left_tokens.intersection(&right_tokens).count() as f32 / union as f32
        };
        let method = if structural_head(&left_expression) == structural_head(&right_expression)
            && score > 0.0
        {
            SemanticMatchMethod::Structural
        } else {
            SemanticMatchMethod::Similarity
        };
        Self { score, method }
    }
}

fn token_set(value: &str) -> std::collections::BTreeSet<&str> {
    value
        .split(|character: char| {
            matches!(
                character,
                ':' | '(' | ')' | ',' | '=' | '[' | ']' | '{' | '}'
            )
        })
        .filter(|token| !token.is_empty())
        .collect()
}

fn structural_head(value: &str) -> &str {
    value.split(':').next().unwrap_or_default()
}

fn semantic_definitions(
    registry: &VocabularyRegistry,
) -> BTreeMap<TermId, (String, SemanticDefinition)> {
    let mut definitions = BTreeMap::new();
    for module in registry.modules() {
        for item in &module.terms {
            let definition =
                item.semantic_definition
                    .clone()
                    .unwrap_or_else(|| SemanticDefinition::Opaque {
                        reason: OpaqueReason::InsufficientEvidence(item.label.clone()),
                    });
            definitions
                .entry(item.id)
                .or_insert((item.label.clone(), definition));
        }
    }
    definitions
}

fn builtin_senses(registry: &VocabularyRegistry) -> Vec<SenseDefinition> {
    let definitions = semantic_definitions(registry);
    let mut senses = Vec::new();
    for module in registry.modules() {
        for binding in &module.lexical_bindings {
            // Registration has already checked explicit dependency visibility
            // and global identity uniqueness. Locale-only modules own lexemes,
            // not copies of their imported semantic definitions.
            let Some((label, definition)) = definitions.get(&binding.target_term) else {
                continue;
            };
            for locale in binding.language_tag.split(',') {
                let lexeme =
                    sem_lang_linguistics::LexemeId::from_surface(&binding.surface_form, locale);
                // Applicability belongs to lexical declarations; identical declarations
                // of this locale/surface/meaning refer to one canonical sense.
                if senses.iter().any(|sense: &SenseDefinition| {
                    sense.lexeme == lexeme && sense.semantic_target == binding.target_term
                }) {
                    continue;
                }
                senses.push(SenseDefinition {
                    id: sem_lang_linguistics::SenseId(
                        sem_lang_linguistics::KnowledgeId::from_label(&format!(
                            "{}:{}:{}",
                            locale, binding.surface_form, binding.target_term
                        ))
                        .0,
                    ),
                    lexeme,
                    semantic_target: binding.target_term,
                    target_label: label.clone(),
                    semantic_definition: Some(definition.clone()),
                    constraints: Vec::new(),
                    evidence: EvidenceSet::default(),
                });
            }
        }
    }
    senses
}

#[derive(Debug, Clone)]
struct LexicalCandidate {
    complete_surface: bool,
    locale: String,
    surface: String,
    normalized_form: String,
    target: TermId,
    contribution_kind: ContributionKind,
    priority: u32,
    conditions: Vec<sem_lang_core::Constraint>,
}

fn build_semantic_expander(registry: &VocabularyRegistry) -> SemanticExpander {
    let mut expander = SemanticExpander::new();
    for (term, (label, definition)) in semantic_definitions(registry) {
        expander.insert_named(term, label, definition);
    }
    expander
}

fn unique<T: Copy + Eq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn compiled_expansions(registry: &VocabularyRegistry) -> Vec<CompiledExpansion> {
    let expander = build_semantic_expander(registry);
    expander
        .terms()
        .filter_map(|term| {
            expander
                .expand(term)
                .ok()
                .map(|trace| CompiledExpansion { term, trace })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct LanguageExplanation {
    pub text: String,
    pub locale: String,
    pub matches: Vec<LanguageMatch>,
}

fn canonical_resource_bytes(resources: &GeneratedLanguageResources) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in &resources.lexicon.entries {
        out.extend_from_slice(entry.surface.as_bytes());
        out.push(0);
        out.extend_from_slice(entry.semantic.concept.to_hex().as_bytes());
        out.push(0);
        out.extend_from_slice(format!("{:?}", entry.origin).as_bytes());
        out.push(0);
        out.extend_from_slice(entry.provenance.as_bytes());
        out.push(0xff);
    }
    for construction in &resources.construction_set.constructions {
        out.extend_from_slice(construction.as_bytes());
        out.push(0xfe);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_linguistics::{LinguisticKnowledge, SemanticDefinition, SemanticExpander};

    #[test]
    fn build_is_deterministic() {
        let knowledge = LinguisticKnowledge::new();
        let registry = VocabularyRegistry::new();
        let left = CompiledLanguage::build(&knowledge, &registry, "r1").expect("build");
        let right = CompiledLanguage::build(&knowledge, &registry, "r1").expect("build");
        assert_eq!(left.digest.to_hex(), right.digest.to_hex());
    }

    #[test]
    fn registered_lexical_data_drives_compilation() {
        let language = test_language();
        let reservation = language.term("Reservation").expect("fixture term");
        let binding = language
            .registry()
            .modules()
            .flat_map(|module| module.lexical_bindings.iter())
            .find(|binding| binding.surface_form == "予約" && binding.target_term == reservation)
            .expect("fixture binding");
        let result = language
            .compile_semantics("予約してください。", "ja-JP")
            .expect("semantic compilation");
        assert_eq!(result.concept, binding.target_term);
        assert_eq!(result.lexeme.surface, binding.surface_form);
        assert!(result.interaction_act.is_some());
    }

    #[test]
    fn pragmatic_variation_does_not_change_the_selected_concept() {
        let language = test_language();
        let direct = language
            .compile_semantics("予約ください。", "ja-JP")
            .unwrap();
        let polite = language
            .compile_semantics("予約お願いします。", "ja-JP")
            .unwrap();
        assert_eq!(
            direct.normalized.proposition_digest,
            polite.normalized.proposition_digest
        );
        assert_ne!(
            direct.normalized.interaction_digest,
            polite.normalized.interaction_digest
        );
    }

    #[test]
    fn unregistered_surface_is_not_fabricated() {
        let language = test_language();
        let result = language.compile_semantics("未登録語", "ja-JP");
        assert!(matches!(
            result,
            Err(SemanticCompileError::UnknownSurface(surface)) if surface == "未登録語"
        ));
    }

    #[test]
    fn semantic_expander_rejects_cycles() {
        let mut expander = SemanticExpander::new();
        let a = term("A");
        let b = term("B");
        expander.insert(a, SemanticDefinition::Alias(b));
        expander.insert(b, SemanticDefinition::Alias(a));
        assert!(expander.expand(a).is_err());
    }

    #[test]
    fn semantic_layers_keep_domain_out_of_kernel() {
        let kernel = sem_lang_linguistics::KernelVocabulary::embedded();
        let basis = sem_lang_linguistics::semantic_basis_vocabulary();
        let standard = sem_lang_linguistics::standard_semantic_vocabulary();
        assert_eq!(
            kernel.layer(),
            sem_lang_linguistics::VocabularyLayer::LinguisticKernel
        );
        assert_eq!(
            basis.layer,
            sem_lang_linguistics::VocabularyLayer::SemanticBasis
        );
        assert_eq!(
            standard.layer,
            sem_lang_linguistics::VocabularyLayer::StandardSemantic
        );
        assert!(kernel.concept("reservation").is_none());
        assert!(
            basis
                .concepts
                .iter()
                .all(|(label, _)| label != "Reservation")
        );
    }

    #[test]
    fn semantic_match_is_deterministic_and_score_is_not_authority() {
        let term = term("Reservation");
        let make = |expr: SemanticExpr| CanonicalSemanticIr {
            proposition_digest: Digest::proposition_digest(expr.canonical().as_bytes()),
            interaction_digest: Digest::interaction_digest(b"observe"),
            expr,
            residuals: Vec::new(),
        };
        let left = make(SemanticExpr::Concept(term));
        let same = make(SemanticExpr::Concept(term));
        let different = make(SemanticExpr::Literal("reservation".into()));
        assert_eq!(
            SemanticMatch::compare(&left, &same),
            SemanticMatch {
                score: 1.0,
                method: SemanticMatchMethod::ExactCanonical
            }
        );
        let candidate = SemanticMatch::compare(&left, &different);
        assert!(candidate.score < 1.0);
        assert_ne!(candidate.method, SemanticMatchMethod::ExactCanonical);
    }

    #[test]
    fn supplied_semantic_definition_is_expanded_without_parser_branches() {
        let language = test_language();
        let result = language
            .compile_semantics("予約", "ja-JP")
            .expect("compiled");
        assert!(matches!(
            result.sense.semantic_definition,
            Some(SemanticDefinition::Composition(_))
        ));
        assert!(!result.trace.result.canonical().is_empty());
    }

    #[test]
    fn an_unprovided_polysemy_is_reported_as_unknown_surface() {
        let language = test_language();
        assert!(language.compile_semantics("同じ表面", "ja-JP").is_err());
    }

    fn test_language() -> CompiledLanguage {
        let mut registry = VocabularyRegistry::new();
        registry
            .register(sem_lang_fixtures::domain::reservation_fixture_module())
            .expect("reservation fixture");
        CompiledLanguage::embedded(&registry).expect("language")
    }

    fn term(label: &str) -> TermId {
        match label {
            "A" => TermId::new_random(1001, 0x7f),
            "B" => TermId::new_random(1002, 0x7f),
            "Reservation" => TermId::new_random(4, 1),
            _ => panic!("test term must be explicitly declared"),
        }
    }
}
