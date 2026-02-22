use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClassifierError {
  #[error("classifier not implemented")]
  NotImplemented,
}

