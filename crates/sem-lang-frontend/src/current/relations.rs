//! Bounded natural joins over explicitly scoped current assertions.
//! Rows are alternatives, never independent bags of slot values.
use super::materialization::Materialization;
use super::{
    ConstraintStatus, EvaluationBound, MAX_ITEMS, MultiplicityResult, ProcessingError, Reason,
    RelationResult, ResolutionIssue, SemanticResolution, ensure_bound,
};
use sem_lang_core::{
    Argument, Binding, Claim, Expr, GraphNode, NodeId, RoleId, TermId, Value, Variable,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Pattern {
    pub predicate: TermId,
    pub variables: Vec<(RoleId, String)>,
    pub guards: Vec<(RoleId, Value)>,
    pub nested: Vec<(RoleId, Pattern)>,
    pub cardinalities: BTreeMap<String, Vec<sem_lang_vocab::Cardinality>>,
}

type Row<'a> = BTreeMap<String, &'a Value>;

#[derive(Clone, Default)]
struct Evidence<'a> {
    values: Row<'a>,
    incomplete_structure: bool,
}

// Alternative proofs of the same tuple are a disjunction. An exact proof
// satisfies a partial one without choosing between different tuples.
fn insert_evidence<'a>(rows: &mut Vec<Evidence<'a>>, row: Evidence<'a>) {
    if let Some(existing) = rows.iter_mut().find(|old| old.values == row.values) {
        existing.incomplete_structure &= row.incomplete_structure;
    } else {
        rows.push(row);
    }
}

impl Pattern {
    fn bounds(&self, name: &str) -> Vec<(usize, Option<usize>)> {
        self.cardinalities
            .get(name)
            .into_iter()
            .flatten()
            .map(|c| c.bounds())
            .chain(self.nested.iter().flat_map(|(_, p)| p.bounds(name)))
            .collect()
    }
    pub fn occurrences(&self) -> usize {
        self.variables.len()
            + self
                .nested
                .iter()
                .map(|(_, p)| p.occurrences())
                .sum::<usize>()
    }
    pub fn count(&self) -> usize {
        1 + self.nested.iter().map(|(_, p)| p.count()).sum::<usize>()
    }
    fn names(&self) -> BTreeSet<String> {
        self.variables
            .iter()
            .map(|(_, name)| name.clone())
            .chain(self.nested.iter().flat_map(|(_, p)| p.names()))
            .collect()
    }

    fn row<'a>(
        &self,
        arguments: &'a [Argument],
        nodes: &'a BTreeMap<NodeId, GraphNode>,
        allowed: &impl Fn(&str, &Value) -> Result<bool, ProcessingError>,
        depth: usize,
    ) -> Result<Option<(Evidence<'a>, BTreeSet<String>)>, ProcessingError> {
        ensure_bound(EvaluationBound::ExpressionDepth, depth, 32)?;
        if !self.guards.iter().all(|(role, value)| {
            arguments
                .iter()
                .any(|arg| arg.role == Some(*role) && arg.value == *value)
        }) {
            return Ok(None);
        }
        let mut row = Row::new();
        let mut missing = BTreeSet::new();
        let mut incomplete_structure = false;
        for (role, variable) in &self.variables {
            let value = arguments
                .iter()
                .find(|arg| arg.role == Some(*role))
                .map(|arg| &arg.value);
            match value {
                Some(value)
                    if !matches!(value, Value::TypedHole(_))
                        && !matches!(value, Value::Node(id) if nodes.get(id).is_some_and(|node| matches!(node.expr, Expr::TypedHole(_)))) =>
                {
                    if (!matches!(value, Value::Nil) && !allowed(variable, value)?)
                        || row.get(variable).is_some_and(|v| *v != value)
                    {
                        return Ok(None);
                    }
                    row.insert(variable.clone(), value);
                }
                _ => {
                    missing.insert(variable.clone());
                }
            }
        }
        for (role, pattern) in &self.nested {
            let value = arguments
                .iter()
                .find(|arg| arg.role == Some(*role))
                .map(|arg| &arg.value);
            let Some(value) = value else {
                missing.extend(pattern.names());
                incomplete_structure |= pattern.names().is_empty();
                continue;
            };
            if matches!(value, Value::TypedHole(_)) {
                missing.extend(pattern.names());
                incomplete_structure |= pattern.names().is_empty();
                continue;
            }
            let Value::Node(reference) = value else {
                return Ok(None);
            };
            let node = nodes
                .get(reference)
                .ok_or(ProcessingError::InvalidReference)?;
            if node.id != *reference {
                return Err(ProcessingError::InvalidReference);
            }
            let Expr::Apply(expression) = &node.expr else {
                if matches!(node.expr, Expr::TypedHole(_)) {
                    missing.extend(pattern.names());
                    incomplete_structure |= pattern.names().is_empty();
                    continue;
                }
                return Ok(None);
            };
            if expression.operator != pattern.predicate {
                return Ok(None);
            }
            let Some((nested, unresolved)) =
                pattern.row(&expression.arguments, nodes, allowed, depth + 1)?
            else {
                return Ok(None);
            };
            if nested
                .values
                .iter()
                .any(|(name, value)| row.get(name).is_some_and(|v| v != value))
            {
                return Ok(None);
            }
            row.extend(nested.values);
            incomplete_structure |= nested.incomplete_structure;
            missing.extend(unresolved);
        }
        for name in &missing {
            row.remove(name);
        }
        Ok(Some((
            Evidence {
                values: row,
                incomplete_structure,
            },
            missing,
        )))
    }

    fn rows<'a>(
        &self,
        claims: &[&'a Claim],
        nodes: &'a BTreeMap<NodeId, GraphNode>,
        allowed: &impl Fn(&str, &Value) -> Result<bool, ProcessingError>,
    ) -> Result<Vec<Evidence<'a>>, ProcessingError> {
        let mut rows = Vec::new();
        for claim in claims {
            if claim.predicate != self.predicate {
                continue;
            }
            if let Some((row, _)) = self.row(&claim.arguments, nodes, allowed, 0)? {
                insert_evidence(&mut rows, row);
            }
        }
        Ok(rows)
    }
}

fn ordered_values<'a>(
    values: impl Iterator<Item = &'a Value>,
    budget: &mut Materialization,
) -> Result<Vec<Value>, ProcessingError> {
    let mut unique = BTreeMap::new();
    for value in values {
        let key = budget.ordering_key(value)?;
        unique.insert(key, value);
    }
    unique
        .into_values()
        .map(|value| budget.copy(value))
        .collect()
}

/// Apply the same typed relation selection to post-join constraints. This is a
/// borrowed view of the existing read, not a second semantic evidence reader.
pub(super) fn scoped_claims<'a>(
    patterns: &[Pattern],
    claims: &[&'a Claim],
    nodes: &BTreeMap<NodeId, GraphNode>,
    result: &SemanticResolution,
    allowed: impl Fn(&str, &Value) -> Result<bool, ProcessingError>,
) -> Result<Vec<&'a Claim>, ProcessingError> {
    let mut selected = Vec::new();
    for claim in claims {
        let relevant: Vec<_> = patterns
            .iter()
            .filter(|p| p.predicate == claim.predicate)
            .collect();
        let mut matches = true;
        for pattern in relevant {
            let Some((row, _)) = pattern.row(&claim.arguments, nodes, &allowed, 0)? else {
                matches = false;
                break;
            };
            let names = pattern.names();
            let relation = result
                .relations
                .iter()
                .find(|r| names.iter().all(|n| r.variables.contains(n)))
                .ok_or(ProcessingError::ContractViolation)?;
            if !relation.rows.iter().any(|solution| {
                row.values.iter().all(|(name, value)| {
                    solution
                        .iter()
                        .find(|b| b.variable.id == *name)
                        .is_none_or(|b| b.value == **value)
                })
            }) {
                matches = false;
                break;
            }
        }
        if matches {
            selected.push(*claim);
        }
    }
    Ok(selected)
}

/// Connected components avoid multiplying unrelated relation alternatives.
/// Within a component a shared variable is an equality, not a union of values.
pub(super) fn resolve(
    patterns: &[Pattern],
    claims: &[&Claim],
    nodes: &BTreeMap<NodeId, GraphNode>,
    result: &mut SemanticResolution,
    allowed: impl Fn(&str, &Value) -> Result<bool, ProcessingError>,
) -> Result<(), ProcessingError> {
    let SemanticResolution {
        bindings,
        issues,
        multiplicities,
        relations,
        ..
    } = result;
    let fixed = std::mem::take(bindings);
    let names: Vec<_> = patterns.iter().map(Pattern::names).collect();
    let evidence: Vec<_> = patterns
        .iter()
        .map(|p| p.rows(claims, nodes, &allowed))
        .collect::<Result<_, _>>()?;
    let mut remaining: BTreeSet<_> = (0..patterns.len()).collect();
    let mut attempts = 0;
    let mut budget = Materialization::default();
    while let Some(first) = remaining.pop_first() {
        let mut component = vec![first];
        let mut variables = names[first].clone();
        loop {
            let linked: Vec<_> = remaining
                .iter()
                .copied()
                .filter(|i| !variables.is_disjoint(&names[*i]))
                .collect();
            if linked.is_empty() {
                break;
            }
            for index in linked {
                remaining.remove(&index);
                variables.extend(names[index].iter().cloned());
                component.push(index);
            }
        }
        let mut joined = vec![Evidence::default()];
        for index in &component {
            if evidence[*index].is_empty() {
                continue;
            }
            let mut next = Vec::new();
            for left in &joined {
                for right in &evidence[*index] {
                    attempts += 1;
                    ensure_bound(EvaluationBound::PairWork, attempts, MAX_ITEMS * MAX_ITEMS)?;
                    if right
                        .values
                        .iter()
                        .any(|(name, value)| left.values.get(name).is_some_and(|v| v != value))
                    {
                        continue;
                    }
                    let mut row = left.clone();
                    row.values.extend(right.values.clone());
                    row.incomplete_structure |= right.incomplete_structure;
                    insert_evidence(&mut next, row);
                    ensure_bound(EvaluationBound::Alternatives, next.len(), MAX_ITEMS)?;
                }
            }
            joined = next;
        }
        // Accepted contextual values constrain shared variables, but cannot
        // invent a value absent from a selected relational assertion.
        joined.retain(|row| {
            fixed.iter().all(|binding| {
                row.values
                    .get(&binding.variable.id)
                    .is_none_or(|value| **value == binding.value)
            })
        });
        let conflict = joined.is_empty();
        relations.push(RelationResult {
            variables: variables.iter().cloned().collect(),
            rows: joined
                .iter()
                .map(|row| {
                    row.values
                        .iter()
                        .map(|(name, value)| {
                            Ok(Binding {
                                variable: Variable {
                                    id: name.clone(),
                                    expected: None,
                                },
                                value: budget.copy(value)?,
                            })
                        })
                        .collect::<Result<Vec<_>, ProcessingError>>()
                })
                .collect::<Result<Vec<_>, _>>()?,
        });
        let mut component_has_issue = false;
        for variable in variables {
            let mut minimum = 0;
            let mut maximum: Option<usize> = None;
            for (min, max) in patterns.iter().flat_map(|p| p.bounds(&variable)) {
                minimum = minimum.max(min);
                maximum = match (maximum, max) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            if maximum.is_some_and(|max| minimum > max) {
                return Err(ProcessingError::ContractViolation);
            }
            let candidates = ordered_values(
                if conflict {
                    component
                        .iter()
                        .flat_map(|i| &evidence[*i])
                        .filter_map(|row| row.values.get(&variable).copied())
                        .collect::<Vec<_>>()
                } else {
                    joined
                        .iter()
                        .filter_map(|row| row.values.get(&variable).copied())
                        .collect()
                }
                .into_iter()
                .chain(
                    fixed
                        .iter()
                        .filter(|binding| conflict && binding.variable.id == variable)
                        .map(|binding| &binding.value),
                )
                .filter(|value| !matches!(value, Value::Nil)),
                &mut budget,
            )?;
            let variable_conflict = conflict
                || (!candidates.is_empty()
                    && joined.iter().any(|row| {
                        row.values
                            .get(&variable)
                            .is_some_and(|value| matches!(value, Value::Nil))
                    }));
            let missing = !conflict && joined.iter().any(|row| !row.values.contains_key(&variable));
            let valid_count =
                candidates.len() >= minimum && maximum.is_none_or(|max| candidates.len() <= max);
            multiplicities.push(MultiplicityResult {
                variable: variable.clone(),
                minimum,
                maximum,
                observed: candidates.len(),
                status: if variable_conflict {
                    ConstraintStatus::Conflicting
                } else if missing {
                    ConstraintStatus::Unresolved
                } else if valid_count {
                    ConstraintStatus::Satisfied
                } else {
                    ConstraintStatus::Unsatisfied
                },
            });
            if !variable_conflict && !missing && valid_count {
                bindings.push(Binding {
                    variable: Variable {
                        id: variable,
                        expected: None,
                    },
                    value: if maximum == Some(1) && candidates.len() == 1 {
                        candidates.into_iter().next().expect("one candidate")
                    } else {
                        Value::Sequence(candidates)
                    },
                });
            } else {
                component_has_issue = true;
                issues.push(ResolutionIssue {
                    reason: if variable_conflict {
                        Reason::ConflictingEvidence
                    } else if missing {
                        Reason::MissingContext
                    } else if maximum == Some(1) && candidates.len() > 1 {
                        Reason::Ambiguous
                    } else {
                        Reason::CardinalityViolation
                    },
                    variable: Some(variable),
                    candidates,
                    constraint: None,
                });
            }
        }
        // A binding from another relation is not evidence that an absent
        // relation holds, even when it happens to bind all the same variables.
        if !component_has_issue
            && (component.iter().any(|i| evidence[*i].is_empty())
                || joined.iter().any(|row| row.incomplete_structure))
        {
            issues.push(ResolutionIssue {
                reason: Reason::MissingContext,
                variable: None,
                candidates: vec![],
                constraint: None,
            });
        }
    }
    bindings.extend(fixed.into_iter().filter(|binding| {
        !names
            .iter()
            .any(|component| component.contains(&binding.variable.id))
    }));
    // A relational result cannot satisfy an unresolved contextual requirement.
    bindings.retain(|binding| {
        !issues
            .iter()
            .any(|issue| issue.variable.as_deref() == Some(binding.variable.id.as_str()))
    });
    bindings.sort_by(|a, b| a.variable.id.cmp(&b.variable.id));
    multiplicities.sort_by(|a, b| a.variable.cmp(&b.variable));
    issues.sort_by(|a, b| a.variable.cmp(&b.variable));
    Ok(())
}
