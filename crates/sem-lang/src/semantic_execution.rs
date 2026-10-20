//! Interpretation to explicit typed execution input. No physical owner access,
//! dispatch, permission, retention, model choice or adoption.
use crate::{
    CompileContext, Engine, SurfaceInput,
    current::{ProcessingError, ProcessingOutcome},
};
pub use sem_lang_execution::semantic::*;
use sem_lang_memory::bindings::DefinitionRef;
use sem_lang_repository::Repository;

#[derive(Debug, thiserror::Error, serde::Serialize)]
pub enum InputError {
    #[error("semantic operation is absent from exact restored definitions")]
    OperationNotDefined,
    #[error("semantic input does not resolve to the requested exact operation")]
    OperationMismatch,
    #[error(transparent)]
    Processing(#[from] ProcessingError),
    #[error(transparent)]
    Contract(#[from] ContractError),
}

impl Engine {
    /// Source text selects meaning only through the existing compiler. An exact
    /// operation and its explicitly declared input roles are checked separately;
    /// a matching surface label never selects an implementation.
    pub fn execution_input<R: Repository>(
        &self,
        repo: &R,
        operation: &DefinitionRef,
        contract: &SemanticOperationContract,
        arguments: &[SemanticArgument],
        input: &SurfaceInput,
        context: &CompileContext,
    ) -> Result<(ProcessingOutcome, Option<TypedRequest>), InputError> {
        let restoration = self
            .restoration_input()
            .ok_or(InputError::OperationNotDefined)?;
        if !restoration
            .definitions
            .iter()
            .any(|r| r.namespace == operation.namespace && r.revision == operation.revision)
        {
            return Err(InputError::OperationNotDefined);
        }
        let definitions = sem_lang_memory::definitions::read_revision(
            repo,
            &operation.namespace,
            &operation.revision,
        )
        .map_err(|_| InputError::OperationNotDefined)?;
        if !definitions
            .modules
            .iter()
            .any(|m| m.terms.iter().any(|t| t.id == operation.term))
        {
            return Err(InputError::OperationNotDefined);
        }
        let outcome = self.compile_surface(input, context)?;
        let ProcessingOutcome::Resolved(resolution) = &outcome else {
            return Ok((outcome, None));
        };
        if resolution.concept != Some(operation.term) {
            return Err(InputError::OperationMismatch);
        }
        for role in contract.input_roles.values() {
            if !restoration
                .definitions
                .iter()
                .any(|r| r.namespace == role.namespace && r.revision == role.revision)
            {
                return Err(ContractError::Role.into());
            }
        }
        let request = bind_semantic_arguments(repo, contract, arguments)?;
        Ok((outcome, Some(request)))
    }
}
