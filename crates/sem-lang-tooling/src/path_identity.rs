//! Conversion between host filesystem paths and portable logical references.
//!
//! Filesystem traversal always uses `Path` components, so platform separators
//! never become part of the identity algorithm.  `/` is used only as the
//! canonical wire encoding of an already validated component sequence.

use std::path::{Component, Path};

/// Encode an already-relative physical path as a portable wire path.
///
/// Separators are interpreted only by the host `Path` implementation. A
/// backslash embedded in a Unix filename is rejected instead of being
/// reinterpreted as a Windows separator, which keeps identities stable when a
/// project is moved between operating systems.
pub(crate) fn logical_path(path: &Path) -> Result<String, String> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| "path contains non-UTF-8 data".to_owned())?;
                if value.is_empty() || value.contains(['/', '\\']) {
                    return Err("path contains a non-portable component".into());
                }
                Ok(value.to_owned())
            }
            _ => Err("path must be relative and normalized".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err("path is empty".into());
    }
    Ok(components.join("/"))
}

pub(crate) fn logical_reference(path: &Path) -> Result<String, String> {
    let value = logical_path(path)?;
    if !zixcel_interaction::valid_reference(&value) {
        return Err("path does not form a valid logical reference".into());
    }
    Ok(value)
}

pub(crate) fn logical_reference_between(
    path: &Path,
    required_prefix: &[&str],
    required_file_name: &str,
) -> Result<String, String> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "path identity contains non-UTF-8 data".to_owned()),
            _ => Err("path identity must be relative and normalized".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.len() <= required_prefix.len() + 1
        || components
            .iter()
            .take(required_prefix.len())
            .map(String::as_str)
            .ne(required_prefix.iter().copied())
        || components.last().map(String::as_str) != Some(required_file_name)
    {
        return Err(format!(
            "path must have the canonical `{}/{required_file_name}` layout",
            required_prefix.join("/")
        ));
    }
    let identity = components[required_prefix.len()..components.len() - 1].join("/");
    if !zixcel_interaction::valid_reference(&identity) {
        return Err("path-derived identity is invalid".into());
    }
    Ok(identity)
}

pub(crate) fn source_identity(package_ref: &str, source: &Path) -> Result<String, String> {
    let mut components = vec![package_ref.to_owned()];
    for component in source.components() {
        let Component::Normal(value) = component else {
            return Err("package source path must be relative and normalized".into());
        };
        let value = value
            .to_str()
            .ok_or_else(|| "package source path contains non-UTF-8 data".to_owned())?;
        components.push(value.to_owned());
    }
    let last = components
        .last_mut()
        .ok_or_else(|| "package source path is empty".to_owned())?;
    *last = last
        .strip_suffix(".sem")
        .ok_or_else(|| "package source path must end in .sem".to_owned())?
        .to_owned();
    let identity = components.join("/");
    if !zixcel_interaction::valid_reference(&identity) {
        return Err("package source path does not form a valid identity".into());
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn identities_follow_components_not_platform_separator_text() {
        let manifest = PathBuf::from("packages")
            .join("example")
            .join("work")
            .join("package.sem");
        assert_eq!(
            logical_reference_between(&manifest, &["packages"], "package.sem").unwrap(),
            "example/work"
        );
        assert_eq!(
            source_identity("example/work", &PathBuf::from("language").join("core.sem")).unwrap(),
            "example/work/language/core"
        );
    }

    #[test]
    fn absolute_and_parent_paths_never_become_identities() {
        assert!(
            logical_reference_between(
                Path::new("../packages/x/package.sem"),
                &["packages"],
                "package.sem"
            )
            .is_err()
        );
        assert!(source_identity("example/work", Path::new("../core.sem")).is_err());
    }

    #[test]
    fn wire_paths_do_not_guess_foreign_platform_separators() {
        assert_eq!(
            logical_path(&PathBuf::from("work").join("review.sem")).unwrap(),
            "work/review.sem"
        );
        #[cfg(unix)]
        assert!(logical_path(Path::new(r"work\review.sem")).is_err());
    }
}
