use parking_lot::Mutex;
use rusqlite::Connection;

pub struct PipelineState {
  _db: Mutex<Option<Connection>>,
}

impl Default for PipelineState {
  fn default() -> Self {
    Self {
      _db: Mutex::new(None),
    }
  }
}

