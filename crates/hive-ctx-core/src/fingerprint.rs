use crate::classifier::SessionState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintEntry {
  pub key: String,
  pub value: String,
}

impl FingerprintEntry {
  fn new(key: String, value: String) -> Self {
    Self { key, value }
  }
}

#[derive(Debug)]
pub struct FingerprintResult {
  pub entries: Vec<FingerprintEntry>,
  pub delta_only: bool,
  pub compiled_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct FingerprintStore {
  last_profile: HashMap<String, String>,
  last_state: Option<SessionState>,
}

impl FingerprintStore {
  pub fn new() -> Self {
    Self {
      last_profile: HashMap::new(),
      last_state: None,
    }
  }

  pub fn compile(
    &mut self,
    profile: HashMap<String, String>,
    session_state: SessionState,
  ) -> FingerprintResult {
    let compiled_at = Utc::now();
    let full_compile = Self::needs_full_compile(session_state);
    let mut entries = Vec::new();

    if full_compile {
      let mut keys: Vec<_> = profile.keys().cloned().collect();
      keys.sort();
      for key in keys {
        if let Some(value) = profile.get(&key) {
          entries.push(FingerprintEntry::new(key.clone(), value.clone()));
        }
      }
    } else {
      let mut changed_keys = Vec::new();
      for key in profile.keys() {
        let changed = match self.last_profile.get(key) {
          Some(previous_value) => previous_value != profile.get(key).unwrap_or(&String::new()),
          None => true,
        };
        if changed {
          changed_keys.push(key.clone());
        }
      }
      changed_keys.sort();
      for key in changed_keys {
        if let Some(value) = profile.get(&key) {
          entries.push(FingerprintEntry::new(key.clone(), value.clone()));
        }
      }

      let mut removed_keys: Vec<_> = self
        .last_profile
        .keys()
        .filter(|key| !profile.contains_key(*key))
        .cloned()
        .collect();
      removed_keys.sort();
      for key in removed_keys {
        entries.push(FingerprintEntry::new(key.clone(), String::new()));
      }
    }

    self.last_profile = profile;
    self.last_state = Some(session_state);

    FingerprintResult {
      entries,
      delta_only: !full_compile,
      compiled_at,
    }
  }

  fn needs_full_compile(state: SessionState) -> bool {
    matches!(
      state,
      SessionState::ColdStart | SessionState::ContextShift | SessionState::TaskMode
    )
  }
}
