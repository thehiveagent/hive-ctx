mod classifier;
mod fingerprint;
mod graph;
mod memory;
mod pipeline;
mod retrieval;

use napi_derive::napi;

#[napi]
pub struct HiveCtxEngine {
  storage_path: String,
  budget_tokens: Option<u32>,
}

#[napi]
impl HiveCtxEngine {
  #[napi(constructor)]
  pub fn new(storage_path: String, budget_tokens: Option<u32>) -> Self {
    Self {
      storage_path,
      budget_tokens,
    }
  }

  #[napi(getter)]
  pub fn storage_path(&self) -> String {
    self.storage_path.clone()
  }

  #[napi(getter)]
  pub fn budget_tokens(&self) -> Option<u32> {
    self.budget_tokens
  }
}

