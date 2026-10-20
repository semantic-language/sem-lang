//! Snapshot storage adapter. Zixcel alone owns CAS, identity, receipts and replay.
//! A proposed semantic image is published by its repository only after success.
use std::sync::Mutex;
use zixcel_revision::{Backend, CommitState, Failure};

#[derive(Debug, Default)]
pub struct SnapshotBackend {
    state: Mutex<CommitState>,
}

impl SnapshotBackend {
    pub fn new(state: CommitState) -> Result<Self, Failure> {
        state.validate()?;
        Ok(Self {
            state: Mutex::new(state),
        })
    }

    pub fn into_state(self) -> Result<CommitState, Failure> {
        self.state.into_inner().map_err(|_| Failure::Storage)
    }
}

impl Backend for &SnapshotBackend {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        let state = self.state.lock().map_err(|_| Failure::Storage)?;
        Ok(read(&state))
    }

    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let mut state = self.state.lock().map_err(|_| Failure::Storage)?;
        let mut proposed = state.clone();
        let result = write(&mut proposed)?;
        proposed.validate()?;
        *state = proposed;
        Ok(result)
    }
}

pub struct BorrowedBackend<'a, B: Backend>(pub &'a B);
impl<B: Backend> Backend for BorrowedBackend<'_, B> {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        self.0.read(read)
    }
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.0.write(write)
    }
}
