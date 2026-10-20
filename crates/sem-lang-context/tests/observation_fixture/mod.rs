use sem_lang_context::{
    CompileContext, ContextScope, ContextSubject, ObservationRequest, SurfaceInput,
    SurfaceRetention,
};

pub fn request(reference: &str, text: &str) -> ObservationRequest {
    ObservationRequest {
        input: SurfaceInput::new(reference, text),
        context: CompileContext {
            subject: ContextSubject {
                id: "fixture:person".into(),
                class: "person".into(),
            },
            locale: "en".into(),
            at: Some(100),
            evidence: vec![],
            semantic_revision: "a".repeat(64),
            context_refs: vec![],
            context_scope: ContextScope::CurrentRequest,
            observation_evidence: vec![],
        },
        retention: SurfaceRetention::KeepAsEvidence,
    }
}
