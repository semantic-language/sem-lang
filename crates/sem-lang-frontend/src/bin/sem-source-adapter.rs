//! Pure language-owner composition. No default context, state, grant or effect.
use sem_lang_frontend::{
    CompileContext,
    current::{source_structure::source_to_structural, structural::StructuralBudget},
};
use serde::Deserialize;
use std::io::{Read, Write};
use zixcel_json_structure::SourceTree;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    tree: SourceTree,
    context: CompileContext,
    budget: StructuralBudget,
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().count() != 1 {
        return Err("source-adapter-arguments".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin().take(131_073).read_to_end(&mut bytes)?;
    if bytes.len() > 131_072 {
        return Err("source-adapter-input-limit".into());
    }
    let request: Request = serde_json::from_slice(&bytes)?;
    let output = source_to_structural(&request.tree, request.context, request.budget)?;
    let bytes = serde_json::to_vec(&output)?;
    if bytes.len() > 131_072 {
        return Err("source-adapter-output-limit".into());
    }
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}
fn main() {
    if run().is_err() {
        eprintln!("source-adapter-rejected");
        std::process::exit(1);
    }
}
