//! Generic, data-owned procedure definitions.
//!
//! A procedure package describes a reusable way of working.  It is not a WBS,
//! schedule, capability implementation, or execution log.  Projects select an
//! exact immutable package; concrete executions remain in the workspace graph.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use zixcel_interaction::valid_reference;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePackageSelection {
    pub manifest: String,
    pub digest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePackage {
    pub schema_version: String,
    pub package_ref: String,
    pub revision: String,
    pub owner_ref: String,
    pub roles: Vec<ProcedureRoleDefinition>,
    /// Typed interfaces used by steps. These describe what a value means and
    /// who owns it; they never embed the value or external payload itself.
    pub data_roles: Vec<ProcedureDataRoleDefinition>,
    pub procedures: Vec<ProcedureDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedureRoleDefinition {
    pub role_ref: String,
    pub label: String,
    pub executor_kind: String,
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedureDataRoleDefinition {
    pub data_role_ref: String,
    pub direction: String,
    pub value_kind_ref: String,
    pub label: String,
    pub authority: String,
    pub sensitivity: String,
    pub materialization: String,
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedureDefinition {
    pub procedure_ref: String,
    pub label: String,
    pub purpose: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    pub steps: Vec<ProcedureStepDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedureStepDefinition {
    pub step_ref: String,
    pub order: u32,
    pub label: String,
    pub purpose: String,
    #[serde(default)]
    pub input_roles: Vec<String>,
    #[serde(default)]
    pub output_roles: Vec<String>,
    pub completion_conditions: Vec<String>,
    #[serde(default)]
    pub actor_role_refs: Vec<String>,
    #[serde(default)]
    pub operation_requirements: Vec<ProcedureOperationRequirement>,
    pub next_step_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcedureOperationRequirement {
    pub requirement_ref: String,
    pub operation_id: String,
    #[serde(default)]
    pub optional: bool,
}

pub(crate) fn validate_package(
    package: &ProcedurePackage,
    schema_version: &str,
) -> Result<(), String> {
    fn text(value: &str, max: usize) -> bool {
        !value.trim().is_empty()
            && value == value.trim()
            && value.len() <= max
            && !value.contains(['\r', '\n'])
    }

    if package.schema_version != schema_version
        || !valid_reference(&package.package_ref)
        || !valid_reference(&package.revision)
        || !valid_reference(&package.owner_ref)
        || package.roles.is_empty()
        || package.roles.len() > 256
        || package.data_roles.is_empty()
        || package.data_roles.len() > 1_024
        || package.procedures.is_empty()
        || package.procedures.len() > 256
    {
        return Err("procedure package identity or size is invalid".into());
    }

    let mut data_role_refs = BTreeMap::new();
    for role in &package.data_roles {
        if !valid_reference(&role.data_role_ref)
            || !valid_reference(&role.value_kind_ref)
            || !text(&role.label, 256)
            || !text(&role.description, 2_048)
            || !matches!(role.direction.as_str(), "input" | "output" | "inout")
            || !matches!(
                role.authority.as_str(),
                "external_owner" | "project" | "procedure_caller" | "operation_provider"
            )
            || !matches!(
                role.sensitivity.as_str(),
                "public" | "internal" | "personal" | "customer_confidential" | "restricted"
            )
            || !matches!(
                role.materialization.as_str(),
                "reference_only" | "project_record" | "external_artifact" | "decision" | "evidence"
            )
            || (role.sensitivity == "restricted" && role.materialization != "reference_only")
            || data_role_refs
                .insert(role.data_role_ref.as_str(), role)
                .is_some()
        {
            return Err("procedure data role is invalid or duplicated".into());
        }
    }

    let mut role_refs = BTreeSet::new();
    for role in &package.roles {
        if !valid_reference(&role.role_ref)
            || !text(&role.label, 256)
            || !matches!(
                role.executor_kind.as_str(),
                "person" | "agent" | "tool" | "system"
            )
            || role.responsibilities.is_empty()
            || role
                .responsibilities
                .iter()
                .chain(&role.constraints)
                .any(|value| !text(value, 2_048))
            || !role_refs.insert(role.role_ref.as_str())
        {
            return Err("procedure role is invalid or duplicated".into());
        }
    }

    let mut procedure_refs = BTreeSet::new();
    for procedure in &package.procedures {
        if !valid_reference(&procedure.procedure_ref)
            || !text(&procedure.label, 256)
            || !text(&procedure.purpose, 2_048)
            || procedure.steps.is_empty()
            || procedure.steps.len() > 256
            || procedure
                .source_refs
                .iter()
                .any(|value| !valid_reference(value))
            || !procedure_refs.insert(procedure.procedure_ref.as_str())
        {
            return Err("procedure definition is invalid or duplicated".into());
        }

        let mut steps = BTreeMap::new();
        let mut orders = BTreeSet::new();
        for step in &procedure.steps {
            if !valid_reference(&step.step_ref)
                || step.order == 0
                || !orders.insert(step.order)
                || !text(&step.label, 256)
                || !text(&step.purpose, 4_096)
                || step.completion_conditions.is_empty()
                || step
                    .completion_conditions
                    .iter()
                    .any(|value| !text(value, 2_048))
                || step.input_roles.iter().any(|value| {
                    data_role_refs
                        .get(value.as_str())
                        .is_none_or(|role| !matches!(role.direction.as_str(), "input" | "inout"))
                })
                || step.output_roles.iter().any(|value| {
                    data_role_refs
                        .get(value.as_str())
                        .is_none_or(|role| !matches!(role.direction.as_str(), "output" | "inout"))
                })
                || step.actor_role_refs.is_empty()
                || step
                    .actor_role_refs
                    .iter()
                    .any(|value| !role_refs.contains(value.as_str()))
                || step.operation_requirements.iter().any(|requirement| {
                    !valid_reference(&requirement.requirement_ref)
                        || !valid_reference(&requirement.operation_id)
                })
                || steps.insert(step.step_ref.as_str(), step).is_some()
            {
                return Err("procedure step is invalid or ambiguous".into());
            }
            let mut requirements = BTreeSet::new();
            if step
                .operation_requirements
                .iter()
                .any(|value| !requirements.insert(value.requirement_ref.as_str()))
            {
                return Err("procedure operation requirement is duplicated".into());
            }
        }

        let mut incoming = BTreeSet::new();
        for step in &procedure.steps {
            if let Some(next) = &step.next_step_ref {
                let Some(next_step) = steps.get(next.as_str()) else {
                    return Err("procedure next step does not exist in the same procedure".into());
                };
                if next_step.order <= step.order || !incoming.insert(next.as_str()) {
                    return Err("procedure next-step relation is cyclic or ambiguous".into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> ProcedurePackage {
        ProcedurePackage {
            schema_version: "0.10.0".into(),
            package_ref: "example/work".into(),
            revision: "revision/example/one".into(),
            owner_ref: "subject/example/owner".into(),
            roles: vec![ProcedureRoleDefinition {
                role_ref: "role/example/operator".into(),
                label: "Operator".into(),
                executor_kind: "person".into(),
                responsibilities: vec!["Operate the selected procedure.".into()],
                constraints: vec!["Do not invent missing input.".into()],
            }],
            data_roles: vec![
                ProcedureDataRoleDefinition {
                    data_role_ref: "input/example/source".into(),
                    direction: "input".into(),
                    value_kind_ref: "kind/example/source".into(),
                    label: "Source".into(),
                    authority: "external_owner".into(),
                    sensitivity: "internal".into(),
                    materialization: "reference_only".into(),
                    required: true,
                    description: "Exact source reference.".into(),
                },
                ProcedureDataRoleDefinition {
                    data_role_ref: "output/example/prepared".into(),
                    direction: "output".into(),
                    value_kind_ref: "kind/example/prepared".into(),
                    label: "Prepared input".into(),
                    authority: "project".into(),
                    sensitivity: "internal".into(),
                    materialization: "project_record".into(),
                    required: true,
                    description: "Prepared input record.".into(),
                },
                ProcedureDataRoleDefinition {
                    data_role_ref: "input/example/prepared".into(),
                    direction: "input".into(),
                    value_kind_ref: "kind/example/prepared".into(),
                    label: "Prepared input".into(),
                    authority: "project".into(),
                    sensitivity: "internal".into(),
                    materialization: "project_record".into(),
                    required: true,
                    description: "Prepared input record.".into(),
                },
                ProcedureDataRoleDefinition {
                    data_role_ref: "output/example/result".into(),
                    direction: "output".into(),
                    value_kind_ref: "kind/example/result".into(),
                    label: "Result".into(),
                    authority: "project".into(),
                    sensitivity: "internal".into(),
                    materialization: "project_record".into(),
                    required: true,
                    description: "Recorded result.".into(),
                },
            ],
            procedures: vec![ProcedureDefinition {
                procedure_ref: "method/example/work".into(),
                label: "Example work".into(),
                purpose: "Prove that procedure data stays outside the language kernel.".into(),
                source_refs: vec!["reference/example/source".into()],
                steps: vec![
                    ProcedureStepDefinition {
                        step_ref: "method/example/work/step/prepare".into(),
                        order: 1,
                        label: "Prepare".into(),
                        purpose: "Prepare exact inputs.".into(),
                        input_roles: vec!["input/example/source".into()],
                        output_roles: vec!["output/example/prepared".into()],
                        completion_conditions: vec!["The exact input is selected.".into()],
                        actor_role_refs: vec!["role/example/operator".into()],
                        operation_requirements: vec![],
                        next_step_ref: Some("method/example/work/step/apply".into()),
                    },
                    ProcedureStepDefinition {
                        step_ref: "method/example/work/step/apply".into(),
                        order: 2,
                        label: "Apply".into(),
                        purpose: "Apply the prepared input.".into(),
                        input_roles: vec!["input/example/prepared".into()],
                        output_roles: vec!["output/example/result".into()],
                        completion_conditions: vec!["The result is recorded.".into()],
                        actor_role_refs: vec!["role/example/operator".into()],
                        operation_requirements: vec![ProcedureOperationRequirement {
                            requirement_ref: "requirement/example/apply".into(),
                            operation_id: "example/apply".into(),
                            optional: false,
                        }],
                        next_step_ref: None,
                    },
                ],
            }],
        }
    }

    #[test]
    fn accepts_generic_ordered_procedure() {
        validate_package(&package(), "0.10.0").unwrap();
    }

    #[test]
    fn rejects_backward_or_shared_next_step() {
        let mut value = package();
        value.procedures[0].steps[1].next_step_ref =
            Some("method/example/work/step/prepare".into());
        assert!(validate_package(&value, "0.10.0").is_err());
    }
}
