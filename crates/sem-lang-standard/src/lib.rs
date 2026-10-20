//! Product-owned standard meanings and multilingual surface bindings.
//!
//! The sources are bundled with the released crate and compiled once per
//! process. Projects select only their domain packages; they never receive an
//! editable copy of this standard set.

use sem_lang_core::Digest;
use sem_lang_vocab::{VocabularyError, VocabularyModule, VocabularyRegistry};
use std::sync::LazyLock;

pub const PACKAGE_REF: &str = "sem-lang/standard";
pub const VERSION: &str = "0.10.0";
pub const SEMANTIC_MODULE: &str = "sem-lang/standard/semantic";
pub const PIPELINE_MODULE: &str = "sem-lang/standard/pipeline";
pub const DOCUMENT_MODULE: &str = "sem-lang/standard/document";
pub const SURFACE_MODULE: &str = "sem-lang/standard/surface";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardLanguageSetDescriptor {
    pub package_ref: String,
    pub version: String,
    pub digest_sha256: String,
    pub module_ids: Vec<String>,
    pub locales: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StandardLanguageSet {
    pub descriptor: StandardLanguageSetDescriptor,
    pub modules: Vec<VocabularyModule>,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid bundled standard language set: {0}")]
pub struct StandardLanguageSetError(String);

static STANDARD: LazyLock<Result<StandardLanguageSet, StandardLanguageSetError>> =
    LazyLock::new(compile);

pub fn language_set() -> Result<&'static StandardLanguageSet, StandardLanguageSetError> {
    STANDARD.as_ref().map_err(Clone::clone)
}

pub fn registry() -> Result<VocabularyRegistry, StandardLanguageSetError> {
    let set = language_set()?;
    let mut registry = VocabularyRegistry::new();
    for module in &set.modules {
        registry
            .register(module.clone())
            .map_err(StandardLanguageSetError::from)?;
    }
    registry
        .validate_definitions()
        .map_err(StandardLanguageSetError::from)?;
    Ok(registry)
}

fn compile() -> Result<StandardLanguageSet, StandardLanguageSetError> {
    let sources = [
        include_str!("../definitions/semantic.sem"),
        include_str!("../definitions/pipeline.sem"),
        include_str!("../definitions/document.sem"),
        include_str!("../definitions/surface.sem"),
    ];
    let mut modules = Vec::with_capacity(sources.len());
    let mut registry = VocabularyRegistry::new();
    for source in sources {
        let module = VocabularyModule::from_sem(source).map_err(StandardLanguageSetError::from)?;
        registry
            .register(module.clone())
            .map_err(StandardLanguageSetError::from)?;
        modules.push(module);
    }
    registry
        .validate_definitions()
        .map_err(StandardLanguageSetError::from)?;
    let digest_sha256 = Digest::from_serializable(1, &(PACKAGE_REF, VERSION, &modules))
        .map_err(|error| StandardLanguageSetError(error.to_string()))?
        .to_hex();
    Ok(StandardLanguageSet {
        descriptor: StandardLanguageSetDescriptor {
            package_ref: PACKAGE_REF.into(),
            version: VERSION.into(),
            digest_sha256,
            module_ids: modules
                .iter()
                .map(|module| module.vocabulary_id.clone())
                .collect(),
            locales: vec!["en-US".into(), "ja-JP".into()],
        },
        modules,
    })
}

impl From<VocabularyError> for StandardLanguageSetError {
    fn from(value: VocabularyError) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sem_lang_core::TermId;

    #[test]
    fn bundled_set_is_stable_multilingual_and_definition_complete() {
        let set = language_set().unwrap();
        assert!(std::ptr::eq(set, language_set().unwrap()));
        assert_eq!(set.descriptor.locales, ["en-US", "ja-JP"]);
        let registry = registry().unwrap();
        let surface = registry.get(SURFACE_MODULE).unwrap();
        let request = TermId::from_declaration(SEMANTIC_MODULE, "Request");
        let forms = surface
            .lexical_bindings
            .iter()
            .filter(|binding| binding.target_term == request)
            .map(|binding| (binding.language_tag.as_str(), binding.surface_form.as_str()))
            .collect::<Vec<_>>();
        assert!(forms.contains(&("ja-JP", "依頼")));
        assert!(forms.contains(&("en-US", "Request")));

        let semantic = registry.get(SEMANTIC_MODULE).unwrap();
        let declared = semantic
            .terms
            .iter()
            .map(|term| term.label.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let bootstrap = sem_lang_linguistics::SEMANTIC_BASIS
            .iter()
            .chain(sem_lang_linguistics::STANDARD_SEMANTIC)
            .map(|term| term.canonical_label)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(declared, bootstrap);
    }

    #[test]
    fn domain_and_vendor_concepts_are_not_standard() {
        let registry = registry().unwrap();
        let labels = registry
            .modules()
            .flat_map(|module| module.terms.iter().map(|term| term.label.as_str()))
            .collect::<Vec<_>>();
        for excluded in [
            "Reservation",
            "Station",
            "PortableDocumentFormat",
            "PdfRender",
        ] {
            assert!(!labels.contains(&excluded));
        }
    }
}
