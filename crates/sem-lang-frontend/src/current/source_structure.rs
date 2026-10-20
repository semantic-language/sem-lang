//! Translate a generic source representation into an explicitly supplied language context.
//! Zixcel owns source shape; this owner decides which shapes the language can interpret.
use super::structural::{SourceNode, SourceShape, StructuralBudget, StructuralInput};
use crate::CompileContext;
use sem_lang_core::Value;
use zixcel_json_structure::{SourceShape as RawShape, SourceTree};

/// Attach language-owned context without inventing meaning from keys or array positions.
/// The result still needs the ordinary semantic resolver/authority checks.
///
/// # Errors
/// Rejects invalid trees/budgets, ordered sequences and non-i64 source numbers.
pub fn source_to_structural(
    tree: &SourceTree,
    context: CompileContext,
    budget: StructuralBudget,
) -> Result<StructuralInput, &'static str> {
    tree.validate()?;
    if budget.max_nodes == 0
        || budget.max_nodes > 1024
        || tree.nodes.len() > budget.max_nodes
        || budget.max_depth > 32
        || budget.max_edges == 0
        || budget.max_candidates == 0
        || budget.max_steps == 0
    {
        return Err("source-semantic-budget-invalid");
    }
    zixcel_json_structure::check_bytes(tree, budget.max_bytes)?;
    let nodes = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(id, node)| {
            let shape = match &node.shape {
                RawShape::Record(fields) => {
                    SourceShape::Record(fields.values().map(|n| format!("n{n}")).collect())
                }
                RawShape::Sequence(_) => return Err("source-sequence-unsupported"),
                RawShape::Scalar(value) => SourceShape::Value(match value {
                    serde_json::Value::Null => Value::Nil,
                    serde_json::Value::Bool(v) => Value::Bool(*v),
                    serde_json::Value::String(v) => Value::Text(v.clone()),
                    serde_json::Value::Number(v) => {
                        Value::Integer(v.as_i64().ok_or("source-number-unsupported")?)
                    }
                    _ => return Err("source-scalar-invalid"),
                }),
            };
            Ok(SourceNode {
                identity: format!("n{id}"),
                locator: node.locator.clone(),
                shape,
            })
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    Ok(StructuralInput {
        reference: tree.reference.clone(),
        root: format!("n{}", tree.root),
        nodes,
        constraints: vec![],
        context,
        budget,
    })
}
