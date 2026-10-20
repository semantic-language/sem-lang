//! Owned behavior, procedure and semantic package loading.

use super::*;

pub(super) fn load_behavior_packages(
    base: &Path,
    manifest: &ProjectManifest,
    operations: &BTreeMap<String, Vec<(String, String, zixcel_interaction::ActionContract)>>,
) -> Result<Vec<BehaviorPackagePresentation>, ToolingError> {
    if manifest.behavior_packages.len() > 128 {
        return Err(ToolingError::Capability(
            "project selects more than 128 behavior packages".into(),
        ));
    }
    let mut selected_paths = BTreeSet::new();
    let mut package_refs = BTreeSet::new();
    let mut packages = Vec::<BehaviorPackageV1>::new();
    for selection in &manifest.behavior_packages {
        if !selection.manifest.ends_with(".json")
            || !selected_paths.insert(selection.manifest.clone())
        {
            return Err(ToolingError::Capability(
                "behavior package manifests must be unique project-relative JSON files".into(),
            ));
        }
        let path = resolve_project_file(
            base,
            &selection.manifest,
            "behavior package manifest",
            MAX_PROJECT_DOCUMENT_BYTES,
        )?;
        let bytes = fs::read(path)?;
        if Digest::document_digest(&bytes).to_hex() != selection.digest_sha256 {
            return Err(ToolingError::Capability(format!(
                "behavior package {} digest does not match the project selection",
                selection.manifest
            )));
        }
        let package = parse_behavior_package_json(&bytes)
            .map_err(|error| ToolingError::Capability(error.to_string()))?;
        let path_identity = path_identity::logical_reference_between(
            Path::new(&selection.manifest),
            &[".sem-lang", "packages"],
            "behavior-package.json",
        )
        .map_err(ToolingError::Capability)?;
        if package.package_ref != path_identity {
            return Err(ToolingError::Capability(format!(
                "behavior package identity must equal its package path: {path_identity}"
            )));
        }
        if selection.publisher_ref != package.owner {
            return Err(ToolingError::Capability(format!(
                "behavior package {} publisher does not match its declared owner",
                package.package_ref
            )));
        }
        if !valid_reference(&selection.admission_ref) {
            return Err(ToolingError::Capability(format!(
                "behavior package {} admission reference is invalid",
                package.package_ref
            )));
        }
        if !package_refs.insert(package.package_ref.clone()) {
            return Err(ToolingError::Capability(format!(
                "behavior package {} is selected more than once",
                package.package_ref
            )));
        }
        packages.push(package);
    }
    packages.sort_by(|left, right| left.package_ref.cmp(&right.package_ref));

    let mut providers = BTreeMap::<String, String>::new();
    for package in &packages {
        for implementation in &package.implementations {
            if providers
                .insert(
                    implementation.behavior_ref.clone(),
                    package.package_ref.clone(),
                )
                .is_some()
            {
                return Err(ToolingError::Capability(format!(
                    "behavior {} has more than one selected provider",
                    implementation.behavior_ref
                )));
            }
        }
    }

    let exact_route_available = packages
        .iter()
        .map(|package| {
            package
                .implementations
                .iter()
                .map(|implementation| {
                    operations
                        .get(&implementation.operation_id)
                        .is_some_and(|routes| {
                            routes.iter().any(|(owner, revision, action)| {
                                owner == &package.owner
                                    && revision == &package.package_revision
                                    && action.target == implementation.target
                                    && action.contract_revision == implementation.contract_revision
                                    && matches!(action.availability, Availability::Available)
                            })
                        })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut ready = BTreeSet::new();
    loop {
        let before = ready.len();
        for (package_index, package) in packages.iter().enumerate() {
            for (implementation_index, implementation) in package.implementations.iter().enumerate()
            {
                if exact_route_available[package_index][implementation_index]
                    && implementation
                        .requires
                        .iter()
                        .all(|value| ready.contains(value))
                {
                    ready.insert(implementation.behavior_ref.clone());
                }
            }
        }
        if ready.len() == before {
            break;
        }
    }

    Ok(packages
        .into_iter()
        .enumerate()
        .map(|(package_index, package)| {
            let implementations = package
                .implementations
                .into_iter()
                .enumerate()
                .map(|(implementation_index, implementation)| {
                    let requirements = implementation
                        .requires
                        .iter()
                        .map(|behavior_ref| {
                            let provider = providers.get(behavior_ref).cloned();
                            BehaviorRequirementPresentation {
                                behavior_ref: behavior_ref.clone(),
                                provider_package_ref: provider,
                                state: if ready.contains(behavior_ref) {
                                    BehaviorInjectionState::Ready
                                } else if providers.contains_key(behavior_ref) {
                                    BehaviorInjectionState::Blocked
                                } else {
                                    BehaviorInjectionState::Unavailable
                                },
                            }
                        })
                        .collect::<Vec<_>>();
                    let state = if !exact_route_available[package_index][implementation_index] {
                        BehaviorInjectionState::Unavailable
                    } else if ready.contains(&implementation.behavior_ref) {
                        BehaviorInjectionState::Ready
                    } else {
                        BehaviorInjectionState::Blocked
                    };
                    BehaviorImplementationPresentation {
                        behavior_ref: implementation.behavior_ref,
                        operation_id: implementation.operation_id,
                        target: implementation.target,
                        contract_revision: implementation.contract_revision,
                        input_type_ref: implementation.input_type_ref,
                        output_type_ref: implementation.output_type_ref,
                        effect: implementation.effect,
                        state,
                        requirements,
                    }
                })
                .collect::<Vec<_>>();
            let state = if implementations
                .iter()
                .all(|value| value.state == BehaviorInjectionState::Ready)
            {
                BehaviorInjectionState::Ready
            } else if implementations
                .iter()
                .any(|value| value.state == BehaviorInjectionState::Unavailable)
            {
                BehaviorInjectionState::Unavailable
            } else {
                BehaviorInjectionState::Blocked
            };
            BehaviorPackagePresentation {
                package_ref: package.package_ref,
                package_revision: package.package_revision,
                owner: package.owner,
                artifact: package.artifact,
                state,
                implementations,
            }
        })
        .collect())
}

pub(super) fn load_procedure_packages(
    base: &Path,
    manifest: &ProjectManifest,
) -> Result<Vec<procedure::ProcedurePackage>, ToolingError> {
    if manifest.procedure_packages.len() > 128 {
        return Err(ToolingError::Capability(
            "project selects more than 128 procedure packages".into(),
        ));
    }
    let mut selected_paths = BTreeSet::new();
    let mut package_refs = BTreeSet::new();
    let mut procedure_refs = BTreeSet::new();
    let mut packages = Vec::new();
    for selection in &manifest.procedure_packages {
        if !selection.manifest.ends_with(".json")
            || !selected_paths.insert(selection.manifest.clone())
        {
            return Err(ToolingError::Capability(
                "procedure package manifests must be unique project-relative JSON files".into(),
            ));
        }
        let path = resolve_project_file(
            base,
            &selection.manifest,
            "procedure package manifest",
            MAX_PROJECT_DOCUMENT_BYTES,
        )?;
        let bytes = fs::read(path)?;
        if Digest::document_digest(&bytes).to_hex() != selection.digest_sha256 {
            return Err(ToolingError::Capability(format!(
                "procedure package {} digest does not match the project selection",
                selection.manifest
            )));
        }
        let package: procedure::ProcedurePackage = serde_json::from_slice(&bytes)?;
        procedure::validate_package(&package, SCHEMA_VERSION).map_err(ToolingError::Capability)?;
        let path_identity = path_identity::logical_reference_between(
            Path::new(&selection.manifest),
            &[".sem-lang", "packages"],
            "procedure-package.json",
        )
        .map_err(ToolingError::Capability)?;
        if package.package_ref != path_identity {
            return Err(ToolingError::Capability(format!(
                "procedure package identity must equal its package path: {path_identity}"
            )));
        }
        if !package_refs.insert(package.package_ref.clone())
            || package
                .procedures
                .iter()
                .any(|value| !procedure_refs.insert(value.procedure_ref.clone()))
        {
            return Err(ToolingError::Capability(
                "procedure package or procedure identity is selected more than once".into(),
            ));
        }
        packages.push(package);
    }
    packages.sort_by(|left, right| left.package_ref.cmp(&right.package_ref));
    Ok(packages)
}

pub(super) fn load_semantic_modules(
    base: &Path,
    selections: &[semantic_package::SemanticPackageSource],
    procedure_packages: &[procedure::ProcedurePackage],
) -> Result<Vec<semantic_module::CompiledSemanticModule>, ToolingError> {
    let known_role_refs: BTreeSet<&str> = procedure_packages
        .iter()
        .flat_map(|package| package.roles.iter().map(|role| role.role_ref.as_str()))
        .collect();
    let mut selected_paths = BTreeSet::new();
    let mut modules = Vec::new();
    for selection in selections {
        if !selection.source.ends_with(".sem") || !selected_paths.insert(selection.source.clone()) {
            return Err(ToolingError::Capability(
                "semantic module sources must be unique project-relative .sem files".into(),
            ));
        }
        let path = resolve_project_file(
            base,
            &selection.source,
            "semantic module source",
            MAX_PROJECT_DOCUMENT_BYTES,
        )?;
        let bytes = fs::read(path)?;
        if Digest::document_digest(&bytes).to_hex() != selection.digest_sha256 {
            return Err(ToolingError::Capability(format!(
                "semantic module {} digest does not match the project selection",
                selection.source
            )));
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            ToolingError::Capability(format!("semantic module {} is not UTF-8", selection.source))
        })?;
        modules.push(
            semantic_module::compile(
                &selection.source_identity,
                &selection.source,
                source,
                SCHEMA_VERSION,
            )
            .map_err(ToolingError::Capability)?,
        );
    }
    modules.sort_by(|left, right| left.module_ref.cmp(&right.module_ref));
    semantic_module::validate_modules(&modules, SCHEMA_VERSION, &known_role_refs)
        .map_err(ToolingError::Capability)?;
    Ok(modules)
}
