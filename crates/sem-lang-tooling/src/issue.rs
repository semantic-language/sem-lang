use serde::{Deserialize, Serialize};

/// Stable failure envelope exposed at tooling boundaries. The human detail is
/// intentionally separate from the machine fields so editor clients never
/// need to parse an error string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolingIssue {
    pub code: String,
    pub class: String,
    pub owner: String,
    pub recovery: String,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolingError {
    #[error("project input could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("project input is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("project.sem is invalid: {0}")]
    Project(String),
    #[error("unsupported schema version in {0}")]
    Version(String),
    #[error("semantic definition package is invalid: {0}")]
    Vocabulary(String),
    #[error("capability catalog contains an invalid reference: {0}")]
    Capability(String),
    #[error("project graph is unavailable or invalid: {0}")]
    Graph(String),
}

impl ToolingError {
    pub fn issue(&self) -> ToolingIssue {
        let (code, class, owner, recovery) = match self {
            Self::Io(_) => (
                "project-input-unavailable",
                "availability",
                "sem-lang/project-input",
                "external-change",
            ),
            Self::Json(_) => (
                "project-input-invalid",
                "contract",
                "sem-lang/project-input",
                "owner-action",
            ),
            Self::Project(_) => (
                "project-source-invalid",
                "contract",
                "sem-lang/project-source",
                "owner-action",
            ),
            Self::Version(_) => (
                "schema-version-unsupported",
                "contract",
                "sem-lang/schema",
                "owner-action",
            ),
            Self::Vocabulary(_) => (
                "semantic-definition-invalid",
                "contract",
                "sem-lang/linguistics",
                "owner-action",
            ),
            Self::Capability(_) => (
                "capability-contract-invalid",
                "contract",
                "sem-lang/capability",
                "owner-action",
            ),
            Self::Graph(_) => (
                "project-graph-unavailable",
                "availability",
                "sem-lang/project-graph",
                "external-change",
            ),
        };
        ToolingIssue {
            code: code.into(),
            class: class.into(),
            owner: owner.into(),
            recovery: recovery.into(),
            detail: self.to_string(),
        }
    }
}
