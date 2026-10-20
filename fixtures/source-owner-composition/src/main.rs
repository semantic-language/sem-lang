//! Private composition fixture. Context is explicit and never sent to a generic producer.
use sem_lang_frontend::{
    CompileContext,
    current::{source_structure::source_to_structural, structural::StructuralBudget},
};
use serde::Deserialize;
use std::{io::Read, path::Path};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    plan: hat_source_curator::input::OwnerInput,
    context: CompileContext,
    budget: StructuralBudget,
}
async fn run(path: &Path, socket: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(131_073)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 131_072 {
        return Err("source-owner-input-limit".into());
    }
    let owner: Owner = serde_json::from_slice(&bytes)?;
    hat_source_curator::input_service::serve(owner.plan, socket, |tree| {
        let input = source_to_structural(&tree, owner.context.clone(), owner.budget.clone())?;
        serde_json::to_value(input).map_err(|_| "source-owner-output-invalid")
    })
    .await
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [owner, socket] => run(Path::new(owner), Path::new(socket)).await,
        _ => Err("source-owner-arguments".into()),
    };
    if result.is_err() {
        eprintln!("source-owner-unavailable");
        std::process::exit(1);
    }
}
