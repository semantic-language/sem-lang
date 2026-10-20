//! Transient value-copy work, charged before allocation. Not semantic state.
use super::{EvaluationBound, ProcessingError};
use sem_lang_core::Value;

const MAX_BYTES: usize = 1_048_576;

pub(super) struct Materialization {
    bytes: usize,
    limit: usize,
}
impl Default for Materialization {
    fn default() -> Self {
        Self {
            bytes: 0,
            limit: MAX_BYTES,
        }
    }
}

impl std::io::Write for Materialization {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > self.limit {
            return Err(std::io::Error::other(
                "materialization byte budget exceeded",
            ));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Materialization {
    fn reserve(&mut self, value: &impl serde::Serialize) -> Result<(), ProcessingError> {
        serde_json::to_writer(&mut *self, value).map_err(|_| {
            if self.bytes > self.limit {
                ProcessingError::EvaluationLimitExceeded {
                    bound: EvaluationBound::MaterializationBytes,
                    limit: self.limit,
                    observed: self.bytes,
                }
            } else {
                ProcessingError::ContractViolation
            }
        })
    }
    pub(super) fn copy(&mut self, value: &Value) -> Result<Value, ProcessingError> {
        self.copy_serialized(value)
    }
    pub(super) fn copy_serialized<T: serde::Serialize + Clone>(
        &mut self,
        value: &T,
    ) -> Result<T, ProcessingError> {
        self.reserve(value)?;
        Ok(value.clone())
    }
    pub(super) fn encode(
        value: &impl serde::Serialize,
        limit: usize,
    ) -> Result<Vec<u8>, ProcessingError> {
        Self {
            bytes: 0,
            limit: limit.min(MAX_BYTES),
        }
        .reserve(value)?;
        serde_json::to_vec(value).map_err(|_| ProcessingError::ContractViolation)
    }
    pub(super) fn ordering_key(&mut self, value: &Value) -> Result<Vec<u8>, ProcessingError> {
        self.reserve(value)?;
        serde_json::to_vec(value).map_err(|_| ProcessingError::ContractViolation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_limit_precedes_copy_and_counts_all_materializations() {
        for total in [MAX_BYTES - 1, MAX_BYTES, MAX_BYTES + 1] {
            let mut budget = Materialization::default();
            let prefix = Value::Text("x".repeat(16384));
            let size = serde_json::to_vec(&prefix).unwrap().len();
            for _ in 0..total / size {
                budget.copy(&prefix).unwrap();
            }
            // JSON external tagging contributes 11 bytes to a Text value.
            let tail = Value::Text("y".repeat(total % size - 11));
            let result = budget.copy(&tail);
            if total <= MAX_BYTES {
                assert_eq!(result.unwrap(), tail);
            } else {
                assert_eq!(
                    result,
                    Err(ProcessingError::EvaluationLimitExceeded {
                        bound: EvaluationBound::MaterializationBytes,
                        limit: MAX_BYTES,
                        observed: total,
                    })
                );
            }
            assert_eq!(budget.bytes, total);
        }
        let mut budget = Materialization::default();
        assert_eq!(
            budget.copy(&Value::Float(f64::NAN)),
            Err(ProcessingError::ContractViolation)
        );
        let value = Value::Sequence(vec![Value::Text("same value".into())]);
        let key = budget.ordering_key(&value).unwrap();
        assert_eq!(key, serde_json::to_vec(&value).unwrap());
        assert_eq!(budget.copy(&value).unwrap(), value);
    }
}
