use crate::{RoleId, TermId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ContextKind {
    TimeZone,
    ReferenceDate,
    Location,
    Calendar,
    Agent,
    Capability,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ContextScope {
    CurrentExpression,
    CurrentRequest,
    Conversation,
    Session,
    Task,
    Agent,
    Environment,
    PersistentAgentDefault,
    DomainSpecific,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextValidity {
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub superseded_by: Option<String>,
}

impl ContextScope {
    /// Scope containment is explicit; unrelated task/domain/environment scopes
    /// are not inferred from labels.
    pub fn permits(self, requested: Self) -> bool {
        self == requested
            || match self {
                Self::Agent | Self::PersistentAgentDefault => {
                    !matches!(requested, Self::Environment | Self::DomainSpecific)
                }
                Self::Session => matches!(
                    requested,
                    Self::CurrentExpression | Self::CurrentRequest | Self::Conversation
                ),
                Self::Conversation | Self::Task => {
                    matches!(requested, Self::CurrentExpression | Self::CurrentRequest)
                }
                Self::CurrentRequest => requested == Self::CurrentExpression,
                _ => false,
            }
    }
}
impl ContextValidity {
    pub fn valid(&self) -> bool {
        self.valid_from
            .zip(self.valid_until)
            .is_none_or(|(from, until)| from < until)
    }
    pub fn applicable(&self, at: Option<i64>) -> Option<bool> {
        if !self.valid() {
            return None;
        }
        if self.superseded_by.is_some() {
            return Some(false);
        }
        if self.valid_from.is_none() && self.valid_until.is_none() {
            return Some(true);
        }
        let at = at?;
        Some(
            self.valid_from.is_none_or(|from| at >= from)
                && self.valid_until.is_none_or(|until| at < until),
        )
    }
}

/// Association within an exact adopted definition revision. No label matching,
/// provider schema, user profile or copied memory value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextAssociation {
    pub kind: ContextKind,
    pub semantic_ref: TermId,
    pub role: RoleId,
    pub scope: ContextScope,
    pub validity: ContextValidity,
}
