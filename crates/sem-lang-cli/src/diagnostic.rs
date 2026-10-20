//! CLI only encodes owner errors; it does not classify commit failures anew.
use serde_json::{Value, json};
use std::error::Error;

pub fn encode(error: &(dyn Error + 'static)) -> Value {
    macro_rules! owner {
        ($type:ty, $domain:literal) => {
            if let Some(source) = error.downcast_ref::<$type>() {
                return json!({"domain": $domain, "error": source, "message": error.to_string()});
            }
        }
    }
    owner!(sem_lang_context::ContextError, "context");
    owner!(sem_lang_repository::RepositoryError, "repository");
    owner!(sem_lang_store_fs::FsRepositoryError, "repository");
    owner!(sem_lang::PacketApplyError, "packet");
    owner!(sem_lang::restoration::RestorationError, "restoration");
    owner!(sem_lang::feedback::FeedbackError, "feedback");
    if let Some(source) = error.downcast_ref::<sem_lang_tooling::ToolingError>() {
        return json!({"domain":"tooling", "error":source.issue(), "message":source.to_string()});
    }
    owner!(
        sem_lang_frontend::current::ConfirmationError,
        "confirmation"
    );
    if let Some(source) = error.downcast_ref::<std::io::Error>() {
        return json!({"domain":"input", "error":{"Io":{"kind":format!("{:?}",source.kind()),"detail":source.to_string()}}, "message":source.to_string()});
    }
    // Non-owner command parsing/formatting errors. Never a fallback for a
    // Foundation variant: all owner paths above serialize their actual enum.
    json!({"domain":"input", "error":{"InvalidInput":error.to_string()}, "message":error.to_string()})
}
