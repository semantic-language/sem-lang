//! Stable sem-lang project-state contracts.
//!
//! Durable records and relations are owned by `workspace_graph`. This module
//! contains only the typed domain projection shared with analyzers and editor
//! clients; it has no file grammar or persistence logic.

use crate::{Diagnostic, ProjectionEdge, ProjectionNode};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;
use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalParseError(&'static str);

impl Display for TemporalParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for TemporalParseError {}

/// A validated RFC 3339 instant. Ordering is chronological, never lexical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExactInstant(String);

impl ExactInstant {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn value(&self) -> OffsetDateTime {
        OffsetDateTime::parse(&self.0, &Rfc3339).expect("ExactInstant is validated at construction")
    }
}

impl TryFrom<String> for ExactInstant {
    type Error = TemporalParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        OffsetDateTime::parse(&value, &Rfc3339)
            .map_err(|_| TemporalParseError("instant must be an RFC 3339 timestamp"))?;
        Ok(Self(value))
    }
}

impl From<ExactInstant> for String {
    fn from(value: ExactInstant) -> Self {
        value.0
    }
}

impl FromStr for ExactInstant {
    type Err = TemporalParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl Display for ExactInstant {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
impl From<&str> for ExactInstant {
    fn from(value: &str) -> Self {
        value.parse().expect("test instant must be valid")
    }
}

impl PartialOrd for ExactInstant {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExactInstant {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value().cmp(&other.value())
    }
}

/// A validated Gregorian civil date serialized as `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CivilDate(String);

impl CivilDate {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CivilDate {
    type Error = TemporalParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let format = time::macros::format_description!("[year]-[month]-[day]");
        Date::parse(&value, &format)
            .map_err(|_| TemporalParseError("civil date must be a real YYYY-MM-DD date"))?;
        Ok(Self(value))
    }
}

impl From<CivilDate> for String {
    fn from(value: CivilDate) -> Self {
        value.0
    }
}

impl FromStr for CivilDate {
    type Err = TemporalParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl Display for CivilDate {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
impl From<&str> for CivilDate {
    fn from(value: &str) -> Self {
        value.parse().expect("test civil date must be valid")
    }
}

/// A bounded IANA-style timezone identity. Offset computation remains the
/// responsibility of the temporal provider; sem-lang does not guess zones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TimeZoneId(String);

impl TimeZoneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TimeZoneId {
    type Error = TemporalParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = value == "UTC"
            || (value.len() <= 128
                && value == value.trim()
                && value.contains('/')
                && !value.contains("//")
                && value.split('/').all(|part| {
                    !part.is_empty()
                        && part != "."
                        && part != ".."
                        && part.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+')
                        })
                }));
        valid.then_some(Self(value)).ok_or(TemporalParseError(
            "timezone must be UTC or an IANA-style identifier",
        ))
    }
}

impl From<TimeZoneId> for String {
    fn from(value: TimeZoneId) -> Self {
        value.0
    }
}

impl FromStr for TimeZoneId {
    type Err = TemporalParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl Display for TimeZoneId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
impl From<&str> for TimeZoneId {
    fn from(value: &str) -> Self {
        value.parse().expect("test timezone must be valid")
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceProjection {
    pub nodes: Vec<ProjectionNode>,
    pub edges: Vec<ProjectionEdge>,
    pub diagnostics: Vec<Diagnostic>,
    pub presentation: WorkspacePresentation,
    /// Validated project-relative databases which contributed to this
    /// immutable projection. Editor clients may watch these paths, but must
    /// rebuild the analyzer instead of mutating its snapshot.
    pub input_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePresentation {
    pub records: Vec<WorkspaceRecordPresentation>,
    pub activities: Vec<WorkspaceActivityPresentation>,
    pub history: Vec<WorkspaceHistoryPresentation>,
    pub connections: Vec<WorkspaceConnectionPresentation>,
    pub plans: Vec<WorkspacePlanEntry>,
    pub timeline: Vec<WorkspaceTimelineEntry>,
    pub dependencies: Vec<WorkspaceDependencyEntry>,
    pub milestones: Vec<WorkspaceMilestonePresentation>,
    pub record_sets: Vec<WorkspaceRecordSetPresentation>,
    pub outputs: Vec<WorkspaceOutputPresentation>,
    pub procedure_runs: Vec<WorkspaceProcedureRunPresentation>,
    pub step_runs: Vec<WorkspaceStepRunPresentation>,
}

impl WorkspacePresentation {
    /// Select immutable completed-work records by civil date. Period
    /// aggregation is deterministic and does not inspect filenames, source
    /// paths, or mutable task state. Both bounds are inclusive.
    pub fn history_refs_in_period(&self, start: &CivilDate, end: &CivilDate) -> Vec<String> {
        let mut values = self
            .history
            .iter()
            .filter(|value| {
                let date = value.occurred_at.value().date();
                let format = time::macros::format_description!("[year]-[month]-[day]");
                Date::parse(start.as_str(), &format).is_ok_and(|lower| {
                    Date::parse(end.as_str(), &format)
                        .is_ok_and(|upper| date >= lower && date <= upper)
                })
            })
            .map(|value| value.history_ref.clone())
            .collect::<Vec<_>>();
        values.sort();
        values
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceHistoryPresentation {
    pub history_ref: String,
    pub former_activity_ref: String,
    pub outcome: String,
    pub occurred_at: ExactInstant,
    pub source_revision: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceActivityPresentation {
    pub activity_ref: String,
    pub next_ref: String,
    pub method_ref: String,
    pub authority_refs: Vec<String>,
    pub completion_condition: String,
    pub history_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConnectionPresentation {
    pub connection_ref: String,
    pub lifecycle: String,
    pub resource_kind: String,
    pub requested_capabilities: Vec<String>,
    pub access: Option<String>,
    pub payload_mode: Option<String>,
    pub synchronization: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecordPresentation {
    pub record_id: String,
    /// Exact human-authored source locator. The graph never copies source text.
    pub source: Option<WorkspaceSourceLocator>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSourceLocator {
    /// Stable graph identity of the statement this locator specifies.
    pub semantic_unit_ref: String,
    pub source_ref: String,
    /// Provenance for the complete authored document revision.
    pub document_digest: String,
    /// Digest of only the exact bounded statement bytes.
    pub unit_digest: String,
    /// Zero-based inclusive line.
    pub start_line: u32,
    /// Zero-based inclusive line.
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTimelineEntry {
    pub time_ref: String,
    pub activity_ref: String,
    pub starts_at: ExactInstant,
    pub ends_at: ExactInstant,
    pub timezone: TimeZoneId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePlanEntry {
    pub plan_ref: String,
    pub task_ref: String,
    pub parent_task_ref: Option<String>,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDependencyEntry {
    pub prerequisite_ref: String,
    pub dependent_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMilestonePresentation {
    pub milestone_ref: String,
    pub activity_ref: String,
    pub label: String,
    pub state: String,
    pub planned_at: ExactInstant,
    pub actual_at: Option<ExactInstant>,
    pub timezone: TimeZoneId,
    pub target_value: String,
    pub actual_value: Option<String>,
    pub variance: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecordSetPresentation {
    pub record_set_ref: String,
    pub activity_ref: String,
    pub label: String,
    pub state: String,
    pub period_start: CivilDate,
    pub period_end: CivilDate,
    /// Exact records derived by the workspace owner for the accepted graph revision.
    pub record_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceOutputPresentation {
    pub output_ref: String,
    pub activity_ref: String,
    pub label: String,
    pub state: String,
    pub role: String,
    pub operation_id: String,
    pub evidence_refs: Vec<String>,
}

/// One concrete application of an immutable procedure revision to work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceProcedureRunPresentation {
    pub run_ref: String,
    pub procedure_ref: String,
    pub activity_ref: String,
    pub state: String,
    pub current_step_ref: Option<String>,
}

/// Runtime state for one procedure step.  The definition remains in its
/// selected package; inputs, outputs and blockers are graph references only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceStepRunPresentation {
    pub step_run_ref: String,
    pub procedure_run_ref: String,
    pub step_ref: String,
    pub state: String,
    pub actor_ref: Option<String>,
    #[serde(default)]
    pub input_bindings: Vec<WorkspaceDataBinding>,
    #[serde(default)]
    pub output_bindings: Vec<WorkspaceDataBinding>,
    #[serde(default)]
    pub blocking_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDataBinding {
    pub data_role_ref: String,
    pub value_ref: String,
}

/// Canonical activity lifecycle. Scheduling and ordered planning are separate
/// graph relations and cannot own or infer this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Planned,
    Active,
    Blocked,
    Completed,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_selection_uses_record_dates_and_inclusive_boundaries() {
        let mut presentation = WorkspacePresentation {
            records: Vec::new(),
            activities: Vec::new(),
            history: Vec::new(),
            connections: Vec::new(),
            plans: Vec::new(),
            timeline: Vec::new(),
            dependencies: Vec::new(),
            milestones: Vec::new(),
            record_sets: Vec::new(),
            outputs: Vec::new(),
            procedure_runs: Vec::new(),
            step_runs: Vec::new(),
        };
        for (history_ref, occurred_at) in [
            ("history/june/record", "2026-06-30T23:59:59+09:00"),
            ("history/july/record", "2026-07-01T00:00:00+09:00"),
            ("history/september/record", "2026-09-30T23:59:59+09:00"),
            ("history/october/record", "2026-10-01T00:00:00+09:00"),
        ] {
            presentation.history.push(WorkspaceHistoryPresentation {
                history_ref: history_ref.into(),
                former_activity_ref: "activity/example/task".into(),
                outcome: "completed".into(),
                occurred_at: occurred_at.into(),
                source_revision: "a".repeat(64),
                evidence_refs: Vec::new(),
            });
        }
        assert_eq!(
            presentation.history_refs_in_period(&"2026-07-01".into(), &"2026-09-30".into(),),
            vec![
                "history/july/record".to_string(),
                "history/september/record".to_string()
            ]
        );
        assert!(
            presentation
                .history_refs_in_period(&"2026-08-01".into(), &"2026-08-31".into())
                .is_empty()
        );
    }
}
