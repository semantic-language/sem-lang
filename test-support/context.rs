//! Explicit fixture operations; never compiled into a published library.
#![allow(dead_code)]
use sem_lang_context::*;
pub trait ContextFixture {
    fn record_timezone_confirmation(&mut self, value: &str);
    fn bind_explicit_timezone(&mut self, value: &str, scope: ContextScope);
    fn clear_short_term(&mut self);
    fn retain_observation(
        &mut self,
        input: ObservationRequest,
    ) -> Result<ObservationReceipt, ContextError>;
}
fn person(engine: &ContextEngine) -> ContextSubject {
    if let Some(subject) = engine.subject() {
        return subject.clone();
    }
    engine
        .store
        .session_timezone
        .as_ref()
        .or(engine.store.active_timezone.as_ref())
        .map(|v| v.subject.clone())
        .unwrap_or(ContextSubject {
            id: "fixture:person".into(),
            class: "person".into(),
        })
}
pub fn formal(
    engine: &ContextEngine,
    subject: ContextSubject,
    scope: ContextScope,
    operation: ContextOperation,
) -> ContextRequest {
    ContextRequest {
        operation_id: format!(
            "fixture:context:{}",
            engine.commit_revision().unwrap().sequence + 1
        ),
        expected_commit_revision: engine.commit_revision().unwrap(),
        expected_memory_revision: engine.memory_revision().unwrap(),
        subject_ref: subject,
        scope,
        operation,
    }
}
impl ContextFixture for ContextEngine {
    fn record_timezone_confirmation(&mut self, value: &str) {
        let evidence_ref = format!(
            "fixture:evidence:{}",
            self.commit_revision().unwrap().sequence
        );
        let request = formal(
            self,
            person(self),
            ContextScope::PersistentAgentDefault,
            ContextOperation::ObserveTimezone {
                value: value.into(),
                evidence_ref,
            },
        );
        self.apply(&request).unwrap();
    }
    fn bind_explicit_timezone(&mut self, value: &str, scope: ContextScope) {
        let request = formal(
            self,
            person(self),
            scope,
            ContextOperation::BindTimezone {
                value: value.into(),
            },
        );
        self.apply(&request).unwrap();
    }
    fn retain_observation(
        &mut self,
        input: ObservationRequest,
    ) -> Result<ObservationReceipt, ContextError> {
        let request = formal(
            self,
            input.context.subject.clone(),
            input.context.context_scope,
            ContextOperation::Retain { observation: input },
        );
        let receipt = self.apply(&request)?;
        Ok(ObservationReceipt {
            reference: receipt.observation_ref.unwrap(),
            changed: receipt.changed,
        })
    }
    fn clear_short_term(&mut self) {
        let mut subjects = vec![];
        for record in &self.store.short_term {
            subjects.push((record.context.subject.clone(), record.context.context_scope));
        }
        for record in &self.store.observations {
            subjects.push((record.subject.clone(), record.scope));
        }
        for record in self.store.decided_candidates.values() {
            subjects.push((record.subject_ref.clone(), record.candidate.inferred_scope));
        }
        for record in [&self.store.session_timezone, &self.store.active_timezone]
            .into_iter()
            .flatten()
        {
            subjects.push((record.subject.clone(), record.scope));
        }
        for (subject, scope) in subjects {
            let request = formal(self, subject, scope, ContextOperation::ClearShortTerm);
            self.apply(&request).unwrap();
        }
    }
}
pub fn update<T>(
    path: impl AsRef<std::path::Path>,
    op: impl FnOnce(&mut ContextEngine) -> Result<T, ContextError>,
) -> Result<T, ContextError> {
    let path = path.as_ref();
    let mut engine = ContextEngine::new(ContextStore::load(path)?);
    let before = serde_json::to_vec(&engine.store).unwrap();
    let result = op(&mut engine)?;
    if before != serde_json::to_vec(&engine.store).unwrap() {
        engine.store.save(path)?;
    }
    Ok(result)
}
