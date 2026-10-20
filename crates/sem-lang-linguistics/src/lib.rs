//! Language-independent linguistic primitives and layered knowledge.
mod construction;
pub use construction::{ConstructionBound, ConstructionError, SurfaceMatch, SurfacePattern};
mod semantic_ids;
pub use semantic_ids::{
    DeclaredSemanticTerm, SEMANTIC_BASIS, STANDARD_SEMANTIC, STANDARD_SEMANTIC_OWNER, standard_term,
};

use sem_lang_core::{Constraint, RoleId, StableId, TermId, TypeId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct KnowledgeId(pub StableId<16>);

impl KnowledgeId {
    pub fn from_label(label: &str) -> Self {
        let mut x = 0xcbf29ce484222325u64;
        for byte in label.as_bytes() {
            x ^= *byte as u64;
            x = x.wrapping_mul(0x100000001b3);
        }
        let mut bytes = [0u8; 16];
        for byte in &mut bytes {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *byte = x as u8;
        }
        Self(StableId(bytes))
    }
}

impl Display for KnowledgeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum KnowledgeOrigin {
    Kernel,
    Curated,
    Learned,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct KernelConcept {
    pub path: String,
    pub term_id: TermId,
}

#[derive(Debug, Clone)]
pub struct KernelVocabulary {
    pub version: String,
    pub concepts: Vec<KernelConcept>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VocabularyLayer {
    LinguisticKernel,
    SemanticBasis,
    StandardSemantic,
    Domain,
}

#[derive(Debug, Clone)]
pub struct LayeredVocabulary {
    pub layer: VocabularyLayer,
    pub concepts: Vec<(String, TermId)>,
}

impl KernelVocabulary {
    pub fn layer(&self) -> VocabularyLayer {
        VocabularyLayer::LinguisticKernel
    }
}

pub fn semantic_basis_vocabulary() -> LayeredVocabulary {
    LayeredVocabulary {
        layer: VocabularyLayer::SemanticBasis,
        concepts: SEMANTIC_BASIS
            .iter()
            .map(|entry| (entry.canonical_label.to_owned(), entry.term_id()))
            .collect(),
    }
}

pub fn standard_semantic_vocabulary() -> LayeredVocabulary {
    LayeredVocabulary {
        layer: VocabularyLayer::StandardSemantic,
        concepts: STANDARD_SEMANTIC
            .iter()
            .map(|entry| (entry.canonical_label.to_owned(), entry.term_id()))
            .collect(),
    }
}

pub fn domain_vocabulary(
    concepts: impl IntoIterator<Item = (impl Into<String>, TermId)>,
) -> LayeredVocabulary {
    LayeredVocabulary {
        layer: VocabularyLayer::Domain,
        concepts: concepts
            .into_iter()
            .map(|(label, id)| (label.into(), id))
            .collect(),
    }
}

impl KernelVocabulary {
    pub fn embedded() -> Self {
        const PATHS: &[&str] = &[
            "kernel.form.surface",
            "kernel.form.grapheme-sequence",
            "kernel.form.token",
            "kernel.form.morpheme",
            "kernel.form.lexeme",
            "kernel.form.construction",
            "kernel.syntax.phrase",
            "kernel.syntax.head",
            "kernel.syntax.modifier",
            "kernel.syntax.argument",
            "kernel.syntax.adjunct",
            "kernel.syntax.dependency",
            "kernel.syntax.grammatical-relation",
            "kernel.semantic.value",
            "kernel.semantic.predicate",
            "kernel.semantic.argument",
            "kernel.semantic.role",
            "kernel.semantic.proposition",
            "kernel.semantic.reference",
            "kernel.semantic.quantification",
            "kernel.semantic.polarity",
            "kernel.semantic.modality",
            "kernel.semantic.temporal-relation",
            "kernel.role.actor",
            "kernel.role.theme",
            "kernel.role.experiencer",
            "kernel.role.source",
            "kernel.role.destination",
            "kernel.role.instrument",
            "kernel.role.time",
            "kernel.role.place",
            "kernel.role.result",
            "kernel.pragmatic.communicative-act",
            "kernel.pragmatic.politeness",
            "kernel.pragmatic.register",
            "kernel.pragmatic.evidentiality",
            "kernel.pragmatic.focus",
            "kernel.pragmatic.presupposition",
            "kernel.act.assertion",
            "kernel.act.question",
            "kernel.act.request",
            "kernel.act.proposal",
            "kernel.act.report",
            "kernel.act.confirmation",
            "kernel.act.correction",
            "kernel.act.rejection",
            "kernel.polarity.positive",
            "kernel.polarity.negative",
            "kernel.modality.required",
            "kernel.modality.permitted",
            "kernel.modality.possible",
            "kernel.discourse.speaker",
            "kernel.discourse.addressee",
            "kernel.discourse.topic",
            "kernel.discourse.antecedent",
            "kernel.discourse.turn-relation",
            "kernel.discourse.discourse-reference",
            "kernel.discourse.context",
            "kernel.entity.entity",
            "kernel.entity.event",
            "kernel.entity.action",
            "kernel.entity.state",
            "kernel.entity.relation",
        ];
        let concepts = PATHS
            .iter()
            .map(|path| KernelConcept {
                path: (*path).to_string(),
                term_id: standard_term_for_kernel_path(path)
                    .unwrap_or_else(|| TermId::from_declaration("sem-lang/kernel", path)),
            })
            .collect();
        Self {
            version: "kernel-0.10.0".to_string(),
            concepts,
        }
    }

    pub fn concept(&self, path: &str) -> Option<&KernelConcept> {
        self.concepts.iter().find(|concept| concept.path == path)
    }
}

fn standard_term_for_kernel_path(path: &str) -> Option<TermId> {
    let label = match path {
        "kernel.semantic.value" => "Value",
        "kernel.semantic.predicate" => "Predicate",
        "kernel.semantic.role" => "Role",
        "kernel.semantic.proposition" => "Proposition",
        "kernel.semantic.reference" => "Reference",
        "kernel.semantic.quantification" => "Quantification",
        "kernel.semantic.polarity" => "Polarity",
        "kernel.semantic.modality" => "Modality",
        "kernel.semantic.temporal-relation" => "Temporal",
        "kernel.role.time" => "Time",
        "kernel.role.place" => "Place",
        "kernel.act.assertion" => "Assertion",
        "kernel.act.question" => "Question",
        "kernel.act.request" => "Request",
        "kernel.act.proposal" => "Proposal",
        "kernel.act.report" => "Report",
        "kernel.act.confirmation" => "Confirmation",
        "kernel.act.correction" => "Correction",
        "kernel.act.rejection" => "Rejection",
        "kernel.entity.entity" => "Entity",
        "kernel.entity.event" => "Event",
        "kernel.entity.action" => "Action",
        "kernel.entity.state" => "State",
        "kernel.entity.relation" => "Relation",
        _ => return None,
    };
    standard_term(label).or_else(|| Some(TermId::from_declaration(STANDARD_SEMANTIC_OWNER, label)))
}

#[derive(Debug, Clone)]
pub struct FormPattern {
    pub surface: String,
    pub locale: Option<String>,
    pub construction: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyntacticPattern {
    pub head: Option<TermId>,
    pub relations: Vec<GrammaticalRelation>,
}

#[derive(Debug, Clone)]
pub struct GrammaticalRelation {
    pub relation: String,
    pub role: Option<RoleId>,
}

#[derive(Debug, Clone)]
pub struct SemanticRoleBinding {
    pub role: RoleId,
    pub target: TypeId,
}

#[derive(Debug, Clone)]
pub struct SemanticContribution {
    pub concept: TermId,
    pub roles: Vec<SemanticRoleBinding>,
    pub polarity: Option<TermId>,
    pub modality: Option<TermId>,
}

#[derive(Debug, Clone)]
pub struct PragmaticContribution {
    pub feature: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum EvidenceKind {
    UserConfirmation,
    UserCorrection,
    SuccessfulExecution,
    FailedExecution,
    TypeCompatibility,
    ContextualMatch,
    PositiveExample,
    NegativeExample,
    ExternalDefinition,
}

#[derive(Debug, Clone)]
pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub reference: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct EvidenceSet {
    pub items: Vec<EvidenceRef>,
}

#[derive(Debug, Clone)]
pub struct LinguisticKnowledgeEntry {
    pub id: KnowledgeId,
    pub origin: KnowledgeOrigin,
    pub form: Option<FormPattern>,
    pub syntax: Option<SyntacticPattern>,
    pub semantics: SemanticContribution,
    pub constraints: Vec<Constraint>,
    pub pragmatics: Vec<PragmaticContribution>,
    pub evidence: EvidenceSet,
    pub validity: Validity,
    pub confidence: f32,
    pub revision: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Validity {
    Active,
    Superseded,
    Rejected,
}

#[derive(Debug, Clone)]
pub struct KernelKnowledge {
    pub vocabulary: KernelVocabulary,
    pub entries: Vec<LinguisticKnowledgeEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct CuratedKnowledge {
    pub entries: Vec<LinguisticKnowledgeEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct LearnedKnowledge {
    pub entries: Vec<LinguisticKnowledgeEntry>,
}

#[derive(Debug, Clone)]
pub struct LinguisticKnowledge {
    pub kernel: KernelKnowledge,
    pub curated: CuratedKnowledge,
    pub learned: LearnedKnowledge,
}

pub type DefinitionOrigin = KnowledgeOrigin;
pub type LanguageDefinition = LinguisticKnowledgeEntry;
pub type KernelDefinitions = KernelKnowledge;
pub type CuratedDefinitions = CuratedKnowledge;
pub type InducedDefinitions = LearnedKnowledge;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LexemeId(pub StableId<16>);

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Lexeme {
    pub id: LexemeId,
    pub surface: String,
    pub locale: String,
}

impl LexemeId {
    pub fn from_surface(surface: &str, locale: &str) -> Self {
        Self(KnowledgeId::from_label(&format!("{locale}:{surface}")).0)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SenseId(pub StableId<16>);

impl Display for LexemeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Display for SenseId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticResidual {
    pub source_term: TermId,
    pub reason: ResidualReason,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ResidualReason {
    Opaque(OpaqueReason),
    DomainSpecific(String),
    Unresolved(String),
    MeaningNotPreserved(String),
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SemanticExpr {
    Primitive(TermId),
    Concept(TermId),
    Variable(String),
    Literal(String),
    Apply {
        predicate: TermId,
        arguments: Vec<(sem_lang_core::RoleId, Box<SemanticExpr>)>,
    },
    Conjunction(Vec<SemanticExpr>),
    /// Explicit adopted link to existing contextual memory, not a label heuristic.
    Contextual {
        association: sem_lang_core::ContextAssociation,
        variable: String,
    },
    Constraint(SemanticConstraint),
    Residual(SemanticResidual),
    Defined {
        term: TermId,
        expansion: Box<SemanticExpr>,
    },
}

impl SemanticExpr {
    pub fn contains_term(&self, term: TermId) -> bool {
        match self {
            Self::Primitive(value) | Self::Concept(value) => *value == term,
            Self::Constraint(value) => value.contains_term(term),
            Self::Contextual { association, .. } => association.semantic_ref == term,
            Self::Variable(_) | Self::Literal(_) | Self::Residual(_) => false,
            Self::Apply {
                predicate,
                arguments,
            } => *predicate == term || arguments.iter().any(|(_, value)| value.contains_term(term)),
            Self::Conjunction(values) => values.iter().any(|value| value.contains_term(term)),
            Self::Defined {
                term: value,
                expansion,
            } => *value == term || expansion.contains_term(term),
        }
    }

    pub fn contains_label(&self, label: &str) -> bool {
        standard_term(label).is_some_and(|term| self.contains_term(term))
    }

    pub fn canonical(&self) -> String {
        match self {
            Self::Primitive(term) => format!("primitive:{}", term.to_hex()),
            Self::Concept(term) => format!("concept:{}", term.to_hex()),
            Self::Variable(value) => format!("var:{value}"),
            Self::Literal(value) => format!("literal:{value}"),
            Self::Constraint(value) => format!("constraint:{}", value.canonical()),
            Self::Contextual {
                association,
                variable,
            } => format!(
                "context:{}",
                serde_json::to_string(&(association, variable))
                    .expect("closed contextual association contains no non-finite values")
            ),
            Self::Residual(value) => {
                format!("residual:{}:{:?}", value.source_term.to_hex(), value.reason)
            }
            Self::Apply {
                predicate,
                arguments,
            } => {
                let mut args: Vec<_> = arguments
                    .iter()
                    .map(|(role, value)| format!("{role}={}", value.canonical()))
                    .collect();
                args.sort();
                format!("apply:{}({})", predicate.to_hex(), args.join(","))
            }
            Self::Conjunction(values) => {
                let mut values: Vec<_> = values.iter().map(Self::canonical).collect();
                values.sort();
                format!("and({})", values.join(","))
            }
            Self::Defined { term, expansion } => {
                format!("defined:{}[{}]", term.to_hex(), expansion.canonical())
            }
        }
    }
}

/// A lossless typed constraint with validated, deterministic wire encoding.
/// The derived encoding is never deserialized as a second semantic authority.
/// Private fields prevent mutation from invalidating the checked representation.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub struct SemanticConstraint {
    value: Constraint,
    #[serde(skip)]
    canonical: String,
}

impl SemanticConstraint {
    fn contains_term(&self, term: TermId) -> bool {
        fn contains(value: &sem_lang_core::Value, term: TermId) -> bool {
            match value {
                sem_lang_core::Value::TermRef(id) => *id == term,
                sem_lang_core::Value::Sequence(values) => values.iter().any(|v| contains(v, term)),
                sem_lang_core::Value::Map(values) => values.values().any(|v| contains(v, term)),
                _ => false,
            }
        }
        match &self.value {
            Constraint::TypeConstraint { value, .. } => contains(value, term),
            Constraint::RelationConstraint { value, target, .. } => {
                contains(value, term) || contains(target, term)
            }
            Constraint::ScopeConstraint { .. } => false,
        }
    }
    pub fn value(&self) -> &Constraint {
        &self.value
    }
    pub fn canonical(&self) -> &str {
        &self.canonical
    }
}

impl TryFrom<Constraint> for SemanticConstraint {
    type Error = serde_json::Error;
    fn try_from(value: Constraint) -> Result<Self, Self::Error> {
        let canonical = serde_json::to_string(&value)?;
        Ok(Self { value, canonical })
    }
}

impl<'de> serde::Deserialize<'de> for SemanticConstraint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Constraint::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

impl PartialEq for SemanticConstraint {
    fn eq(&self, other: &Self) -> bool {
        self.canonical == other.canonical
    }
}
impl Eq for SemanticConstraint {}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PrimitiveSemantic {
    Value,
    Type,
    Predicate,
    Relation,
    Role,
    Proposition,
    Reference,
    Variable,
    Constraint,
    Composition,
    Quantification,
    Polarity,
    Modality,
    Temporal,
    Spatial,
    Causality,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum OpaqueReason {
    DomainSpecific(String),
    InsufficientEvidence(String),
    ExplicitNoExpand,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SemanticDefinition {
    Primitive(PrimitiveSemantic),
    Composition(SemanticExpr),
    Alias(TermId),
    Refined {
        base: TermId,
        constraints: Vec<Constraint>,
    },
    Opaque {
        reason: OpaqueReason,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExpansionQuality {
    Exact,
    Conservative,
    Partial,
    Lossy,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExpansionStop {
    Primitive,
    Opaque,
    Unresolved,
    CycleDetected,
    MaximumExpansionDepth,
    ExplicitNoExpand,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExpansionStep {
    pub from: TermId,
    pub to: TermId,
    pub from_label: String,
    pub to_label: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SemanticExpansionTrace {
    pub source: TermId,
    pub steps: Vec<ExpansionStep>,
    pub result: SemanticExpr,
    pub quality: ExpansionQuality,
    pub stop: ExpansionStop,
    pub residuals: Vec<SemanticResidual>,
}

#[derive(Debug, Clone)]
pub struct SenseDefinition {
    pub id: SenseId,
    pub lexeme: LexemeId,
    pub semantic_target: TermId,
    pub target_label: String,
    pub semantic_definition: Option<SemanticDefinition>,
    pub constraints: Vec<Constraint>,
    pub evidence: EvidenceSet,
}

#[derive(Debug, Clone, thiserror::Error, Eq, PartialEq)]
pub enum ExpansionError {
    #[error("invalid typed semantic constraint")]
    InvalidConstraint,
    #[error("semantic definition cycle at {0}")]
    Cycle(TermId),
    #[error("maximum semantic expansion depth exceeded")]
    MaximumDepth,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticExpander {
    definitions: BTreeMap<TermId, (String, SemanticDefinition)>,
    max_depth: usize,
}

#[derive(Debug, Clone)]
pub struct ExpandedSemantic {
    pub result: SemanticExpr,
    pub quality: ExpansionQuality,
    pub stop: ExpansionStop,
    pub steps: Vec<ExpansionStep>,
    pub residuals: Vec<SemanticResidual>,
}

impl SemanticExpander {
    pub fn new() -> Self {
        Self {
            definitions: BTreeMap::new(),
            max_depth: 32,
        }
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }

    pub fn insert(&mut self, term: TermId, definition: SemanticDefinition) {
        self.insert_named(term, term.to_hex(), definition);
    }

    pub fn insert_named(
        &mut self,
        term: TermId,
        label: impl Into<String>,
        definition: SemanticDefinition,
    ) {
        self.definitions.insert(term, (label.into(), definition));
    }

    pub fn definition(&self, term: TermId) -> Option<&SemanticDefinition> {
        self.definitions
            .get(&term)
            .map(|(_, definition)| definition)
    }

    pub fn terms(&self) -> impl Iterator<Item = TermId> + '_ {
        self.definitions.keys().copied()
    }

    pub fn expand(&self, source: TermId) -> Result<SemanticExpansionTrace, ExpansionError> {
        let mut expanded = ExpandedSemantic {
            result: SemanticExpr::Concept(source),
            quality: ExpansionQuality::Exact,
            stop: ExpansionStop::Unresolved,
            steps: Vec::new(),
            residuals: Vec::new(),
        };
        let mut stack = BTreeSet::new();
        expanded.result = self.walk(source, 0, &mut stack, &mut expanded)?;
        Ok(SemanticExpansionTrace {
            source,
            steps: expanded.steps,
            result: expanded.result,
            quality: expanded.quality,
            stop: expanded.stop,
            residuals: expanded.residuals,
        })
    }

    fn walk(
        &self,
        term: TermId,
        depth: usize,
        stack: &mut BTreeSet<TermId>,
        out: &mut ExpandedSemantic,
    ) -> Result<SemanticExpr, ExpansionError> {
        if depth >= self.max_depth {
            out.stop = ExpansionStop::MaximumExpansionDepth;
            return Err(ExpansionError::MaximumDepth);
        }
        if !stack.insert(term) {
            out.stop = ExpansionStop::CycleDetected;
            return Err(ExpansionError::Cycle(term));
        }
        let Some((label, definition)) = self.definitions.get(&term) else {
            stack.remove(&term);
            out.stop = ExpansionStop::Unresolved;
            return Ok(SemanticExpr::Concept(term));
        };
        let result = match definition {
            SemanticDefinition::Primitive(_) => {
                out.stop = ExpansionStop::Primitive;
                SemanticExpr::Primitive(term)
            }
            SemanticDefinition::Opaque { reason } => {
                out.stop = if matches!(reason, OpaqueReason::ExplicitNoExpand) {
                    ExpansionStop::ExplicitNoExpand
                } else {
                    ExpansionStop::Opaque
                };
                out.quality = ExpansionQuality::Lossy;
                let residual = SemanticResidual {
                    source_term: term,
                    reason: ResidualReason::Opaque(reason.clone()),
                };
                out.residuals.push(residual.clone());
                SemanticExpr::Residual(residual)
            }
            SemanticDefinition::Alias(target) => {
                out.steps.push(ExpansionStep {
                    from: term,
                    to: *target,
                    from_label: label.clone(),
                    to_label: self.label(*target),
                });
                self.walk(*target, depth + 1, stack, out)?
            }
            SemanticDefinition::Refined { base, constraints } => {
                out.steps.push(ExpansionStep {
                    from: term,
                    to: *base,
                    from_label: label.clone(),
                    to_label: self.label(*base),
                });
                let base_expr = self.walk(*base, depth + 1, stack, out)?;
                let constraints = constraints
                    .iter()
                    .map(|constraint| {
                        SemanticConstraint::try_from(constraint.clone())
                            .map(SemanticExpr::Constraint)
                            .map_err(|_| ExpansionError::InvalidConstraint)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                SemanticExpr::Defined {
                    term,
                    expansion: Box::new(SemanticExpr::Conjunction(
                        [vec![base_expr], constraints].concat(),
                    )),
                }
            }
            SemanticDefinition::Composition(expression) => {
                self.collect_steps(term, expression, out);
                let expanded = self.walk_expr(expression, depth + 1, stack, out)?;
                out.stop = if out.residuals.is_empty() {
                    ExpansionStop::Primitive
                } else {
                    ExpansionStop::Opaque
                };
                SemanticExpr::Defined {
                    term,
                    expansion: Box::new(expanded),
                }
            }
        };
        stack.remove(&term);
        Ok(result)
    }

    fn walk_expr(
        &self,
        expression: &SemanticExpr,
        depth: usize,
        stack: &mut BTreeSet<TermId>,
        out: &mut ExpandedSemantic,
    ) -> Result<SemanticExpr, ExpansionError> {
        match expression {
            SemanticExpr::Concept(term) => self.walk(*term, depth, stack, out),
            SemanticExpr::Apply {
                predicate,
                arguments,
            } => Ok(SemanticExpr::Apply {
                predicate: *predicate,
                arguments: arguments
                    .iter()
                    .map(|(role, value)| {
                        Ok((*role, Box::new(self.walk_expr(value, depth, stack, out)?)))
                    })
                    .collect::<Result<_, ExpansionError>>()?,
            }),
            SemanticExpr::Conjunction(values) => Ok(SemanticExpr::Conjunction(
                values
                    .iter()
                    .map(|value| self.walk_expr(value, depth, stack, out))
                    .collect::<Result<_, _>>()?,
            )),
            SemanticExpr::Residual(residual) => {
                out.quality = ExpansionQuality::Partial;
                out.residuals.push(residual.clone());
                Ok(SemanticExpr::Residual(residual.clone()))
            }
            value => Ok(value.clone()),
        }
    }

    fn collect_steps(&self, source: TermId, expression: &SemanticExpr, out: &mut ExpandedSemantic) {
        match expression {
            SemanticExpr::Apply {
                predicate,
                arguments,
            } => {
                out.steps.push(ExpansionStep {
                    from: source,
                    to: *predicate,
                    from_label: self.label(source),
                    to_label: self.label(*predicate),
                });
                for (_, argument) in arguments {
                    self.collect_steps(source, argument, out);
                }
            }
            SemanticExpr::Conjunction(values) => {
                for value in values {
                    self.collect_steps(source, value, out);
                }
            }
            _ => {}
        }
    }

    fn label(&self, term: TermId) -> String {
        self.definitions
            .get(&term)
            .map(|(label, _)| label.clone())
            .unwrap_or_else(|| term.to_hex())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum KnowledgeError {
    #[error("knowledge entry already exists: {0}")]
    Duplicate(KnowledgeId),
    #[error("learned knowledge cannot replace kernel entry: {0}")]
    KernelOverride(KnowledgeId),
    #[error("knowledge entry has an invalid origin")]
    InvalidOrigin,
}

impl LinguisticKnowledge {
    pub fn new() -> Self {
        let vocabulary = KernelVocabulary::embedded();
        let entries = vocabulary
            .concepts
            .iter()
            .map(|concept| LinguisticKnowledgeEntry {
                id: KnowledgeId::from_label(&concept.path),
                origin: KnowledgeOrigin::Kernel,
                form: None,
                syntax: None,
                semantics: SemanticContribution {
                    concept: concept.term_id,
                    roles: Vec::new(),
                    polarity: None,
                    modality: None,
                },
                constraints: Vec::new(),
                pragmatics: Vec::new(),
                evidence: EvidenceSet::default(),
                validity: Validity::Active,
                confidence: 1.0,
                revision: "kernel-0.10.0".to_string(),
            })
            .collect();
        Self {
            kernel: KernelKnowledge {
                vocabulary,
                entries,
            },
            curated: CuratedKnowledge::default(),
            learned: LearnedKnowledge::default(),
        }
    }

    pub fn add_curated(
        &mut self,
        mut entry: LinguisticKnowledgeEntry,
    ) -> Result<(), KnowledgeError> {
        entry.origin = KnowledgeOrigin::Curated;
        self.ensure_unique(entry.id)?;
        self.curated.entries.push(entry);
        Ok(())
    }

    pub fn add_learned(
        &mut self,
        mut entry: LinguisticKnowledgeEntry,
    ) -> Result<(), KnowledgeError> {
        entry.origin = KnowledgeOrigin::Learned;
        if self
            .kernel
            .entries
            .iter()
            .any(|kernel| kernel.id == entry.id)
        {
            return Err(KnowledgeError::KernelOverride(entry.id));
        }
        self.ensure_unique(entry.id)?;
        self.learned.entries.push(entry);
        Ok(())
    }

    pub fn all_entries(&self) -> impl Iterator<Item = &LinguisticKnowledgeEntry> {
        self.kernel
            .entries
            .iter()
            .chain(self.curated.entries.iter())
            .chain(self.learned.entries.iter())
    }

    pub fn validate(&self) -> Result<(), KnowledgeError> {
        let mut ids = std::collections::BTreeSet::new();
        for entry in self.all_entries() {
            if !ids.insert(entry.id) {
                return Err(KnowledgeError::Duplicate(entry.id));
            }
        }
        Ok(())
    }

    fn ensure_unique(&self, id: KnowledgeId) -> Result<(), KnowledgeError> {
        if self.all_entries().any(|entry| entry.id == id) {
            Err(KnowledgeError::Duplicate(id))
        } else {
            Ok(())
        }
    }
}

impl Default for LinguisticKnowledge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_is_stable_and_domain_free() {
        let left = KernelVocabulary::embedded();
        let right = KernelVocabulary::embedded();
        assert_eq!(left.concepts, right.concepts);
        assert!(left.concept("kernel.role.theme").is_some());
        assert!(left.concept("reservation").is_none());
    }

    #[test]
    fn learned_entries_cannot_replace_kernel_entries() {
        let mut knowledge = LinguisticKnowledge::new();
        let kernel_id = knowledge.kernel.entries[0].id;
        let entry = LinguisticKnowledgeEntry {
            id: kernel_id,
            origin: KnowledgeOrigin::Learned,
            form: None,
            syntax: None,
            semantics: knowledge.kernel.entries[0].semantics.clone(),
            constraints: Vec::new(),
            pragmatics: Vec::new(),
            evidence: EvidenceSet::default(),
            validity: Validity::Active,
            confidence: 0.8,
            revision: "r1".to_string(),
        };
        assert!(matches!(
            knowledge.add_learned(entry),
            Err(KnowledgeError::KernelOverride(_))
        ));
    }
}
