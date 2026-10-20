//! Bounded physical project input and service discovery.

use super::*;

pub(super) fn discover_service_documents(
    base: &Path,
    roots: &[String],
    runtime: bool,
) -> Result<Vec<ServiceDocument>, ToolingError> {
    let mut documents = Vec::new();
    for root in roots {
        let resolved = resolve_project_path(base, root, "service root")?;
        let metadata = fs::symlink_metadata(&resolved)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ToolingError::Capability(format!(
                "service root {} is not a physical directory",
                root
            )));
        }
        walk_service_directory(&resolved, &resolved, 0, runtime, &mut documents)?;
    }
    documents.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(documents)
}

pub(super) fn walk_service_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    runtime: bool,
    documents: &mut Vec<ServiceDocument>,
) -> Result<(), ToolingError> {
    if depth > MAX_SERVICE_DEPTH {
        return Err(ToolingError::Capability(
            "service hierarchy exceeds the supported depth".into(),
        ));
    }
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(ToolingError::Capability(
                "service hierarchies cannot contain symbolic links".into(),
            ));
        }
        if metadata.is_dir() {
            let segment = entry
                .file_name()
                .into_string()
                .map_err(|_| ToolingError::Capability("service path is not UTF-8".into()))?;
            if !valid_service_segment(&segment) {
                return Err(ToolingError::Capability(format!(
                    "service path segment {segment} is not part of the service language"
                )));
            }
            walk_service_directory(root, &path, depth + 1, runtime, documents)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(ToolingError::Capability(
                "service hierarchies may contain only directories and regular files".into(),
            ));
        }
        let file_name = entry
            .file_name()
            .into_string()
            .map_err(|_| ToolingError::Capability("service filename is not UTF-8".into()))?;
        let kind = match (runtime, file_name.as_str()) {
            (false, "capability.json") => Some(ServiceDocumentKind::Capability),
            (false, "authentication.json") => Some(ServiceDocumentKind::Authentication),
            (true, "state.json") => Some(ServiceDocumentKind::State),
            _ => None,
        };
        let Some(kind) = kind else {
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                return Err(ToolingError::Capability(format!(
                    "unknown service language document {file_name}"
                )));
            }
            continue;
        };
        if metadata.len() > MAX_SERVICE_DOCUMENT_BYTES || documents.len() >= MAX_SERVICE_DOCUMENTS {
            return Err(ToolingError::Capability(
                "service language document limits were exceeded".into(),
            ));
        }
        let parent = path
            .parent()
            .and_then(|value| value.strip_prefix(root).ok())
            .ok_or_else(|| ToolingError::Capability("service path escaped its root".into()))?;
        let id = path_identity::logical_reference(parent).map_err(ToolingError::Capability)?;
        if id.is_empty() || !valid_reference(&id) {
            return Err(ToolingError::Capability(
                "service document requires a path-derived identity".into(),
            ));
        }
        documents.push(ServiceDocument { id, kind, path });
    }
    Ok(())
}

pub(super) fn valid_service_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

pub(super) fn read_json_bounded<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<T, ToolingError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SERVICE_DOCUMENT_BYTES
    {
        return Err(ToolingError::Capability(
            "service language document is not a bounded regular file".into(),
        ));
    }
    read_json(path)
}

pub(super) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ToolingError> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

pub(crate) fn project_directory(project_file: &Path) -> Result<PathBuf, ToolingError> {
    let parent = project_file
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .map_or_else(std::env::current_dir, Ok)?;
    Ok(fs::canonicalize(parent)?)
}

pub(super) fn resolve_project_file(
    base: &Path,
    value: &str,
    owner: &str,
    max_bytes: u64,
) -> Result<PathBuf, ToolingError> {
    let resolved = resolve_project_path(base, value, owner)?;
    validate_physical_file(&resolved, owner, max_bytes)?;
    Ok(resolved)
}

pub(crate) fn resolve_project_path(
    base: &Path,
    value: &str,
    owner: &str,
) -> Result<PathBuf, ToolingError> {
    let canonical_base = fs::canonicalize(base)?;
    let relative = Path::new(value);
    if value.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ToolingError::Capability(format!(
            "{owner} must use a normalized project-relative path"
        )));
    }

    let mut resolved = canonical_base.clone();
    for component in relative.components() {
        let std::path::Component::Normal(segment) = component else {
            unreachable!("components were validated above")
        };
        resolved.push(segment);
        let metadata = fs::symlink_metadata(&resolved)?;
        if metadata.file_type().is_symlink() {
            return Err(ToolingError::Capability(format!(
                "{owner} path cannot contain symbolic links"
            )));
        }
    }
    let canonical = fs::canonicalize(&resolved)?;
    if !canonical.starts_with(&canonical_base) {
        return Err(ToolingError::Capability(format!(
            "{owner} escaped the project directory"
        )));
    }
    Ok(canonical)
}

pub(super) fn validate_physical_file(
    path: &Path,
    owner: &str,
    max_bytes: u64,
) -> Result<(), ToolingError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        Err(ToolingError::Capability(format!(
            "{owner} is not a bounded physical file"
        )))
    } else {
        Ok(())
    }
}
