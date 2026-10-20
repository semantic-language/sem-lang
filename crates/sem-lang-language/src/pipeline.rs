//! Language-level shape of a declarative semantic pipeline.
//!
//! Surface languages bind their own words to these stable operator terms. The
//! compiler therefore never recognizes Japanese or English keywords directly.
//! External packages implement behavior behind a resolved semantic term; they
//! cannot add control-flow syntax to this core.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineOperator {
    Source,
    Select,
    Transform,
    Expand,
    Aggregate,
    Effect,
    Result,
}

impl PipelineOperator {
    /// Resolve the universal pipeline role from a canonical term name.
    ///
    /// `TermId` includes the selected package path and therefore cannot be a
    /// language-global constant. The caller first resolves the exact term in
    /// its selected language package, then supplies that term's canonical
    /// name here.
    pub fn from_canonical_name(name: &str) -> Option<Self> {
        [
            ("PipelineSource", Self::Source),
            ("PipelineSelect", Self::Select),
            ("PipelineTransform", Self::Transform),
            ("PipelineExpand", Self::Expand),
            ("PipelineAggregate", Self::Aggregate),
            ("PipelineEffect", Self::Effect),
            ("PipelineResult", Self::Result),
        ]
        .into_iter()
        .find_map(|(candidate, operator)| (name == candidate).then_some(operator))
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::Source,
            Self::Select,
            Self::Transform,
            Self::Expand,
            Self::Aggregate,
            Self::Effect,
            Self::Result,
        ]
        .into_iter()
        .find(|operator| operator.name() == name)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Select => "select",
            Self::Transform => "transform",
            Self::Expand => "expand",
            Self::Aggregate => "aggregate",
            Self::Effect => "effect",
            Self::Result => "result",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PipelineShapeError {
    #[error("a pipeline must start with one source")]
    MissingSource,
    #[error("a pipeline may contain only one source")]
    DuplicateSource,
    #[error("a pipeline must end with one result")]
    MissingResult,
    #[error("a pipeline may contain only one result")]
    DuplicateResult,
    #[error("no transformation may follow an effect")]
    TransformationAfterEffect,
    #[error("no pipeline step may follow its result")]
    StepAfterResult,
}

/// Validate only universal data-flow ordering. Input/output type compatibility
/// and effects remain owned by the injected behavior contracts.
pub fn validate_pipeline_shape(
    operators: impl IntoIterator<Item = PipelineOperator>,
) -> Result<(), PipelineShapeError> {
    let operators = operators.into_iter().collect::<Vec<_>>();
    if operators.first() != Some(&PipelineOperator::Source) {
        return Err(PipelineShapeError::MissingSource);
    }
    if operators
        .iter()
        .filter(|value| **value == PipelineOperator::Source)
        .count()
        != 1
    {
        return Err(PipelineShapeError::DuplicateSource);
    }
    if operators.last() != Some(&PipelineOperator::Result) {
        return Err(PipelineShapeError::MissingResult);
    }
    if operators
        .iter()
        .filter(|value| **value == PipelineOperator::Result)
        .count()
        != 1
    {
        return Err(PipelineShapeError::DuplicateResult);
    }
    let mut effect_seen = false;
    let mut result_seen = false;
    for operator in operators {
        if result_seen {
            return Err(PipelineShapeError::StepAfterResult);
        }
        match operator {
            PipelineOperator::Effect => effect_seen = true,
            PipelineOperator::Result => result_seen = true,
            PipelineOperator::Source => {}
            _ if effect_seen => return Err(PipelineShapeError::TransformationAfterEffect),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_query_and_effect_pipelines() {
        assert_eq!(
            validate_pipeline_shape([
                PipelineOperator::Source,
                PipelineOperator::Transform,
                PipelineOperator::Aggregate,
                PipelineOperator::Result,
            ]),
            Ok(())
        );
        assert_eq!(
            validate_pipeline_shape([
                PipelineOperator::Source,
                PipelineOperator::Select,
                PipelineOperator::Effect,
                PipelineOperator::Result,
            ]),
            Ok(())
        );
    }

    #[test]
    fn rejects_imperative_or_ambiguous_shapes() {
        assert_eq!(
            validate_pipeline_shape([PipelineOperator::Transform, PipelineOperator::Result]),
            Err(PipelineShapeError::MissingSource)
        );
        assert_eq!(
            validate_pipeline_shape([
                PipelineOperator::Source,
                PipelineOperator::Effect,
                PipelineOperator::Transform,
                PipelineOperator::Result,
            ]),
            Err(PipelineShapeError::TransformationAfterEffect)
        );
    }
}
