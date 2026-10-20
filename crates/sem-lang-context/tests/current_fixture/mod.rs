//! Test fixture selection uses explicit subject and exact stored refs, not a
//! production fallback or a temporal parser.
use sem_lang_context::*;

pub fn values(engine: &ContextEngine, subject: &ContextSubject) -> Vec<String> {
    let mut refs = engine
        .store
        .persistent_defaults
        .iter()
        .filter(|entry| &entry.subject == subject)
        .map(|entry| ContextReference::Default(entry.id.clone()))
        .collect::<Vec<_>>();
    refs.extend(
        engine
            .store
            .decided_candidates
            .iter()
            .filter(|(_, entry)| &entry.subject_ref == subject)
            .map(|(reference, _)| ContextReference::Decision(reference.clone())),
    );
    if engine.store.active_timezone.is_some() {
        refs.push(ContextReference::Active);
    }
    if engine.store.session_timezone.is_some() {
        refs.push(ContextReference::Session);
    }
    engine
        .read_current(&refs, subject, ContextScope::CurrentRequest, None)
        .unwrap()
        .select(ContextKind::TimeZone, &[])
        .values
        .into_iter()
        .map(str::to_owned)
        .collect()
}
