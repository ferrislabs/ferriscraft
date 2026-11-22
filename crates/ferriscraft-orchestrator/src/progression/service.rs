use ferriscraft_progression::{ProgressionRepository, ProgressionStoreError};

#[derive(Debug, thiserror::Error)]
pub enum ProgressionError {
    #[error(transparent)]
    Store(#[from] ProgressionStoreError),
}

#[derive(Clone)]
pub struct ProgressionService<R> {
    repo: R,
}

impl<R: ProgressionRepository> ProgressionService<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub fn repository(&self) -> &R {
        &self.repo
    }
}
