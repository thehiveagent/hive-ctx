use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
  pub id: Uuid,
  pub created_at: DateTime<Utc>,
  pub text: String,
}

