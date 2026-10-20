//! Opaque reference/adoption operations; never loads or invokes a capability.
use clap::Subcommand;
use sem_lang_memory::bindings::{self, AdoptionState, DefinitionRef, SemanticBinding};
use sem_lang_store_fs::FsRepository;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{fs::File, io::Read};

#[derive(Subcommand)]
pub enum BindingCommand {
    Write {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        file: String,
        #[arg(long, value_parser = state)]
        state: AdoptionState,
        #[arg(long)]
        decision_ref: String,
        #[arg(long)]
        expected_revision: Option<String>,
        #[arg(long)]
        expected_commit_revision: String,
    },
    Read {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        revision: Option<String>,
    },
    Resolve {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        namespace: String,
        /// JSON exact DefinitionRef, not a label or current-head selector.
        #[arg(long)]
        file: String,
    },
}

fn state(value: &str) -> Result<AdoptionState, String> {
    serde_json::from_value(Value::String(value.into())).map_err(|e| e.to_string())
}

fn load<T: DeserializeOwned>(file: &str) -> Result<T, Box<dyn std::error::Error>> {
    let mut bytes = vec![];
    File::open(file)?.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("binding input exceeds limit".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn run(command: BindingCommand) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(match command {
        BindingCommand::Write {
            repo,
            namespace,
            file,
            state,
            decision_ref,
            expected_revision,
            expected_commit_revision,
        } => {
            let items: Vec<SemanticBinding> = load(&file)?;
            let mut repository = FsRepository::open_existing(repo)?;
            json!({"revision": bindings::write(&mut repository, &namespace, expected_revision.as_ref(), &items, state, &decision_ref, &serde_json::from_str(&expected_commit_revision)?)?})
        }
        BindingCommand::Read {
            repo,
            namespace,
            revision,
        } => {
            let repository = FsRepository::open_existing(repo)?;
            let value = match revision {
                Some(id) => Some((
                    id.clone(),
                    bindings::read_revision(&repository, &namespace, &id)?,
                )),
                None => bindings::read(&repository, &namespace)?,
            };
            json!(value.map(|(revision, memory)| json!({"revision": revision, "memory": memory})))
        }
        BindingCommand::Resolve {
            repo,
            namespace,
            file,
        } => {
            let reference: DefinitionRef = load(&file)?;
            let repository = FsRepository::open_existing(repo)?;
            json!(bindings::resolve(&repository, &namespace, &reference)?)
        }
    })
}
