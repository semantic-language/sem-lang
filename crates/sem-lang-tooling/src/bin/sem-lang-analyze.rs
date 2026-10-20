use sem_lang_tooling::WorkspaceAnalyzer;
use std::fs;

fn main() {
    if let Err(error) = run() {
        eprintln!("sem-lang-analyze: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let project = args
        .next()
        .ok_or("usage: sem-lang-analyze <project.sem> <instruction.sem>")?;
    let source = args
        .next()
        .ok_or("usage: sem-lang-analyze <project.sem> <instruction.sem>")?;
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let analysis = WorkspaceAnalyzer::load(project)?.analyze(&fs::read_to_string(source)?);
    println!("{}", serde_json::to_string_pretty(&analysis)?);
    if analysis
        .diagnostics
        .iter()
        .any(|item| item.severity == sem_lang_tooling::Severity::Error)
    {
        std::process::exit(2);
    }
    Ok(())
}
