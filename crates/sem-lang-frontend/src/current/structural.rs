//! owner-neutral, bounded structural unification.
//! No source-name lookup, lexical compilation, induction, mutation or adoption.
use super::*;
use sem_lang_core::{ContextAssociation, RoleId, Variable};
use sem_lang_linguistics::SemanticDefinition;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceNode {
    pub identity: String,
    pub locator: String,
    pub shape: SourceShape,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SourceShape {
    Value(Value),
    /// Unordered containment. Source ordering never assigns semantic roles.
    Record(Vec<String>),
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SourceConstraint {
    Equal(String, String),
    Different(String, String),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralBudget {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_candidates: usize,
    pub max_steps: usize,
    pub max_depth: usize,
    pub max_bytes: usize,
}
impl Default for StructuralBudget {
    fn default() -> Self {
        Self {
            max_nodes: 128,
            max_edges: 256,
            max_candidates: 128,
            max_steps: 32768,
            max_depth: 32,
            max_bytes: 65536,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralInput {
    pub reference: String,
    pub root: String,
    pub nodes: Vec<SourceNode>,
    pub constraints: Vec<SourceConstraint>,
    pub context: CompileContext,
    pub budget: StructuralBudget,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralCandidate {
    pub concept: TermId,
    pub expression: SemanticExpr,
    pub bindings: Vec<Binding>,
    pub issues: Vec<ResolutionIssue>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralResolution {
    pub semantic_revision: String,
    pub subject: sem_lang_context::ContextSubject,
    pub context_refs: Vec<(
        sem_lang_context::ContextReference,
        sem_lang_context::ContextStatus,
    )>,
    pub reason: Option<Reason>,
    pub candidates: Vec<StructuralCandidate>,
    pub input_digest: String,
    pub meaning_digest: String,
    /// Exact source/owner evidence lineage, not part of semantic equivalence.
    pub source_refs: Vec<String>,
    pub evidence_refs: Vec<NodeId>,
}
type Result<T> = std::result::Result<T, ProcessingError>;
type Row = BTreeMap<String, Value>;
struct Work<'a> {
    budget: &'a StructuralBudget,
    steps: usize,
    nodes: BTreeMap<&'a str, &'a SourceNode>,
    copies: super::materialization::Materialization,
}
impl Work<'_> {
    fn step(&mut self, depth: usize) -> Result<()> {
        self.steps = self.steps.saturating_add(1);
        ensure_bound(
            EvaluationBound::PairWork,
            self.steps,
            self.budget.max_steps.min(1_000_000),
        )?;
        ensure_bound(
            EvaluationBound::ExpressionDepth,
            depth,
            self.budget.max_depth.min(32),
        )
    }
    fn value(&mut self, v: &Value, depth: usize) -> Result<()> {
        self.step(depth)?;
        match v {
            Value::Nil
            | Value::Bool(_)
            | Value::Integer(_)
            | Value::Decimal { .. }
            | Value::Entity(_) => (),
            Value::Float(v) if v.is_finite() => (),
            Value::Text(t) => ensure_bound(
                EvaluationBound::MaterializationBytes,
                t.len(),
                self.budget.max_bytes.min(65536),
            )?,
            Value::Sequence(v) => {
                for x in v {
                    self.value(x, depth + 1)?
                }
            }
            Value::Map(v) => {
                for (k, x) in v {
                    ensure_bound(EvaluationBound::MaterializationBytes, k.len(), 512)?;
                    self.value(x, depth + 1)?
                }
            }
            _ => return Err(ProcessingError::InvalidInput),
        }
        Ok(())
    }
    fn visit(
        &mut self,
        id: &str,
        stack: &mut BTreeSet<String>,
        seen: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<()> {
        self.step(depth)?;
        if !stack.insert(id.into()) {
            return Err(ProcessingError::InvalidInput);
        }
        let node = *self
            .nodes
            .get(id)
            .ok_or(ProcessingError::InvalidReference)?;
        seen.insert(id.into());
        match &node.shape {
            SourceShape::Record(children) => {
                if children.iter().collect::<BTreeSet<_>>().len() != children.len() {
                    return Err(ProcessingError::InvalidInput);
                }
                for c in children {
                    self.visit(c, stack, seen, depth + 1)?;
                }
            }
            SourceShape::Value(v) => self.value(v, depth + 1)?,
            SourceShape::Unknown => (),
        }
        stack.remove(id);
        Ok(())
    }
    fn matches(
        &mut self,
        expr: &SemanticExpr,
        id: &str,
        row: &Row,
        depth: usize,
    ) -> Result<Vec<Row>> {
        self.step(depth)?;
        let node = *self
            .nodes
            .get(id)
            .ok_or(ProcessingError::InvalidReference)?;
        match (expr, &node.shape) {
            (SemanticExpr::Variable(name), SourceShape::Value(value)) => {
                if row.get(name).is_some_and(|prior| prior != value) {
                    return Ok(vec![]);
                }
                let mut next = self.copies.copy_serialized(row)?;
                next.insert(name.clone(), self.copies.copy(value)?);
                Ok(vec![next])
            }
            (SemanticExpr::Literal(literal), SourceShape::Value(Value::Text(value)))
                if literal == value =>
            {
                Ok(vec![self.copies.copy_serialized(row)?])
            }
            (SemanticExpr::Apply { arguments, .. }, SourceShape::Record(children))
                if arguments.len() == children.len() =>
            {
                ensure_bound(EvaluationBound::Arguments, arguments.len(), 64)?;
                self.assign(arguments, children, row, depth + 1)
            }
            (SemanticExpr::Defined { expansion, .. }, _) => {
                self.matches(expansion, id, row, depth + 1)
            }
            _ => Ok(vec![]),
        }
    }
    fn assign(
        &mut self,
        args: &[(RoleId, Box<SemanticExpr>)],
        children: &[String],
        row: &Row,
        depth: usize,
    ) -> Result<Vec<Row>> {
        self.step(depth)?;
        let Some(((_, expr), rest)) = args.split_first() else {
            return Ok(vec![self.copies.copy_serialized(row)?]);
        };
        let mut rows = Vec::new();
        for (i, id) in children.iter().enumerate() {
            let remaining = children
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, v)| v.clone())
                .collect::<Vec<_>>();
            for matched in self.matches(expr, id, row, depth)? {
                for completed in self.assign(rest, &remaining, &matched, depth)? {
                    if !rows.contains(&completed) {
                        rows.push(completed);
                    }
                    ensure_bound(EvaluationBound::Alternatives, rows.len(), 64)?;
                }
            }
        }
        Ok(rows)
    }
}

impl CurrentStateCompiler<'_> {
    pub fn resolve_structure(&self, input: &StructuralInput) -> Result<StructuralResolution> {
        let ctx = &input.context;
        if self.semantic_revision()? != ctx.semantic_revision {
            return Err(ProcessingError::StaleRevision);
        }
        if !token(&input.reference) || !token(&input.root) {
            return Err(ProcessingError::InvalidInput);
        }
        ensure_bound(
            EvaluationBound::ValueItems,
            input.nodes.len(),
            input.budget.max_nodes.min(1024),
        )?;
        let edges = input
            .nodes
            .iter()
            .map(|n| match &n.shape {
                SourceShape::Record(c) => c.len(),
                _ => 0,
            })
            .sum::<usize>();
        ensure_bound(
            EvaluationBound::Arguments,
            edges.saturating_add(input.constraints.len()),
            input.budget.max_edges.min(2048),
        )?;
        let mut work = Work {
            budget: &input.budget,
            steps: 0,
            nodes: BTreeMap::new(),
            copies: super::materialization::Materialization::default(),
        };
        for node in &input.nodes {
            if !token(&node.identity)
                || node.locator.len() > 4096
                || node.locator.contains('\0')
                || work.nodes.insert(&node.identity, node).is_some()
            {
                return Err(ProcessingError::InvalidInput);
            }
        }
        let mut seen = BTreeSet::new();
        work.visit(&input.root, &mut BTreeSet::new(), &mut seen, 0)?;
        if seen.len() != input.nodes.len() {
            return Err(ProcessingError::InvalidInput);
        }
        let bytes = super::materialization::Materialization::encode(
            input,
            input.budget.max_bytes.min(65536),
        )?;
        let claims = self.scoped_claims(ctx)?;
        let evidence_refs = claims.iter().map(|c| c.id).collect::<Vec<_>>();
        let (claims, claim_constraints) = self.applicable_claims(&claims)?;
        let read = self.read_current(claims, ctx)?;
        let mut reason = None;
        for constraint in &input.constraints {
            work.step(0)?;
            let (a, b, equal) = match constraint {
                SourceConstraint::Equal(a, b) => (a, b, true),
                SourceConstraint::Different(a, b) => (a, b, false),
            };
            let a = &work
                .nodes
                .get(a.as_str())
                .ok_or(ProcessingError::InvalidReference)?
                .shape;
            let b = &work
                .nodes
                .get(b.as_str())
                .ok_or(ProcessingError::InvalidReference)?
                .shape;
            match (a, b) {
                (SourceShape::Value(a), SourceShape::Value(b)) if (a == b) != equal => {
                    reason = Some(Reason::ConflictingEvidence)
                }
                (SourceShape::Value(_), SourceShape::Value(_)) => (),
                _ => {
                    if reason.is_none() {
                        reason = Some(Reason::MissingContext)
                    }
                }
            }
        }
        let definitions = self
            .registry
            .modules()
            .flat_map(|m| &m.terms)
            .collect::<Vec<_>>();
        ensure_bound(
            EvaluationBound::Patterns,
            definitions.len(),
            input.budget.max_candidates.min(512),
        )?;
        let mut candidates = Vec::new();
        for definition in definitions {
            work.step(0)?;
            let Some(SemanticDefinition::Composition(expr)) = &definition.semantic_definition
            else {
                continue;
            };
            let (pattern, guards, associations) = parts(expr)?;
            let Some(pattern) = pattern else { continue };
            if !self.structural_role_bounds(pattern, &mut work, 0)? {
                continue;
            }
            for row in work.matches(pattern, &input.root, &Row::new(), 0)? {
                let mut result = SemanticResolution {
                    semantic_revision: ctx.semantic_revision.clone(),
                    context_refs: vec![],
                    observation_ref: input.reference.clone(),
                    subject: ctx.subject.clone(),
                    concept: Some(definition.id),
                    expression: Some(work.copies.copy_serialized(expr)?),
                    bindings: row
                        .into_iter()
                        .map(|(id, value)| Binding {
                            variable: Variable { id, expected: None },
                            value,
                        })
                        .collect(),
                    issues: vec![],
                    evidence_refs: evidence_refs.clone(),
                    claim_constraints: claim_constraints.clone(),
                    multiplicities: vec![],
                    relations: vec![],
                    identity: String::new(),
                };
                self.resolve_contextual(&associations, &read, ctx, &mut result)?;
                for condition in &claim_constraints {
                    if matches!(
                        condition.status,
                        ConstraintStatus::Unresolved
                            | ConstraintStatus::Conflicting
                            | ConstraintStatus::Circular
                    ) {
                        result.issues.push(issue(
                            if condition.status == ConstraintStatus::Conflicting {
                                Reason::ConflictingEvidence
                            } else {
                                Reason::MissingContext
                            },
                        ));
                    }
                }
                let mut rejected = false;
                for guard in &guards {
                    work.step(0)?;
                    match self.resolved_constraint_truth(guard, &result, &read.claims)? {
                        constraints::Truth::True => (),
                        constraints::Truth::False => rejected = true,
                        constraints::Truth::Conflict => {
                            result.issues.push(issue(Reason::ConflictingEvidence))
                        }
                        constraints::Truth::Unknown => {
                            result.issues.push(issue(Reason::MissingContext))
                        }
                    }
                }
                if !rejected {
                    candidates.push(StructuralCandidate {
                        concept: definition.id,
                        expression: work.copies.copy_serialized(expr)?,
                        bindings: result.bindings,
                        issues: result.issues,
                    });
                    ensure_bound(EvaluationBound::Alternatives, candidates.len(), 64)?;
                }
            }
        }
        let mut keyed = candidates
            .into_iter()
            .map(|c| {
                Ok((
                    (
                        c.concept,
                        super::materialization::Materialization::encode(&c.bindings, 65536)?,
                    ),
                    c,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        let mut candidates = keyed.into_iter().map(|(_, c)| c).collect::<Vec<_>>();
        candidates.dedup();
        if reason.is_none() {
            reason = if candidates.is_empty() {
                Some(
                    if input
                        .nodes
                        .iter()
                        .any(|n| matches!(n.shape, SourceShape::Unknown))
                    {
                        Reason::MissingContext
                    } else {
                        Reason::UnknownExpression
                    },
                )
            } else if candidates.iter().any(|c| {
                c.issues
                    .iter()
                    .any(|i| i.reason == Reason::ConflictingEvidence)
            }) {
                Some(Reason::ConflictingEvidence)
            } else if candidates.iter().any(|c| !c.issues.is_empty()) {
                Some(Reason::MissingContext)
            } else if candidates.len() > 1 {
                Some(Reason::Ambiguous)
            } else {
                None
            };
        }
        let meaning_digest = Digest::from_serializable(1, &(&reason, &candidates))
            .map_err(|_| ProcessingError::ContractViolation)?
            .to_hex();
        Ok(StructuralResolution {
            semantic_revision: ctx.semantic_revision.clone(),
            subject: ctx.subject.clone(),
            context_refs: read
                .context
                .entries
                .iter()
                .map(|e| (e.reference.clone(), e.status))
                .collect(),
            reason,
            candidates,
            input_digest: Digest::object_digest(&bytes).to_hex(),
            meaning_digest,
            source_refs: seen.into_iter().collect(),
            evidence_refs,
        })
    }

    fn structural_role_bounds(
        &self,
        expr: &SemanticExpr,
        work: &mut Work<'_>,
        depth: usize,
    ) -> Result<bool> {
        work.step(depth)?;
        match expr {
            SemanticExpr::Apply { arguments, .. } => {
                ensure_bound(EvaluationBound::Arguments, arguments.len(), 64)?;
                for (role, child) in arguments {
                    let cardinality = self
                        .registry
                        .modules()
                        .flat_map(|m| &m.roles)
                        .find(|r| r.id == *role)
                        .ok_or(ProcessingError::InvalidReference)?
                        .cardinality;
                    let (min, max) = cardinality.bounds();
                    if min > 1
                        || max == Some(0)
                        || !self.structural_role_bounds(child, work, depth + 1)?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            SemanticExpr::Defined { expansion, .. } => {
                self.structural_role_bounds(expansion, work, depth + 1)
            }
            SemanticExpr::Literal(_) | SemanticExpr::Variable(_) => Ok(true),
            _ => Ok(false),
        }
    }
}
fn issue(reason: Reason) -> ResolutionIssue {
    ResolutionIssue {
        reason,
        variable: None,
        candidates: vec![],
        constraint: None,
    }
}
type Parts<'a> = (
    Option<&'a SemanticExpr>,
    Vec<sem_lang_core::Constraint>,
    Vec<(ContextAssociation, String)>,
);
fn parts(expr: &SemanticExpr) -> Result<Parts<'_>> {
    let mut pattern = None;
    let mut guards = vec![];
    let mut associations = vec![];
    let expressions = match expr {
        SemanticExpr::Conjunction(expressions) => expressions.as_slice(),
        _ => std::slice::from_ref(expr),
    };
    for expression in expressions {
        match expression {
            SemanticExpr::Apply { .. } if pattern.is_none() => pattern = Some(expression),
            SemanticExpr::Constraint(c) => guards.push(c.value().clone()),
            SemanticExpr::Contextual {
                association,
                variable,
            } => associations.push((association.clone(), variable.clone())),
            _ => return Ok((None, vec![], vec![])),
        }
    }
    Ok((pattern, guards, associations))
}
