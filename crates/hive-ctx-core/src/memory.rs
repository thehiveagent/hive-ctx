use crate::graph::{GraphDatabase, NodeCategory};
use blake3::Hasher;
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use rusqlite::{params, OptionalExtension, Connection};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MemoryTier {
  Tier1,
  Tier2,
  Tier3,
}

impl MemoryTier {
  pub fn as_str(&self) -> &'static str {
    match self {
      MemoryTier::Tier1 => "tier1",
      MemoryTier::Tier2 => "tier2",
      MemoryTier::Tier3 => "tier3",
    }
  }
}

#[derive(Debug, Clone)]
pub struct MemoryRecord {
  pub id: i64,
  pub tier: MemoryTier,
  pub created_at: DateTime<Utc>,
  pub expires_at: Option<DateTime<Utc>>,
  pub text: String,
}

#[derive(Debug)]
pub struct MemorySnapshot {
  pub tier1: Vec<MemoryRecord>,
  pub tier2: Vec<MemoryRecord>,
  pub tier3: Vec<MemoryRecord>,
}

#[derive(Debug)]
pub struct MemoryCompressionResult {
  pub compressed: usize,
  pub skipped: usize,
}

#[derive(Debug)]
pub struct MemoryCrystallizationResult {
  pub processed_summaries: usize,
  pub facts_created: usize,
}

#[derive(Debug)]
pub struct MemoryStats {
  pub tier1_count: usize,
  pub tier2_count: usize,
  pub tier3_count: usize,
  pub last_compress: Option<DateTime<Utc>>,
  pub last_crystallize: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct MemoryStore {
  conn: Arc<Mutex<Connection>>,
  graph: Arc<GraphDatabase>,
}

impl MemoryStore {
  pub fn open(path: &Path, graph: Arc<GraphDatabase>) -> Result<Self, MemoryError> {
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::open(path)?;
    let store = Self {
      conn: Arc::new(Mutex::new(connection)),
      graph,
    };
    store.ensure_schema()?;
    Ok(store)
  }

  pub fn store(&self, text: &str) -> Result<i64, MemoryError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
      return Err(MemoryError::EmptyText);
    }

    let created_at = Utc::now();
    let expires_at = created_at + Duration::hours(24);
    let mut hasher = Hasher::new();
    hasher.update(trimmed.as_bytes());
    let content_hash = hasher.finalize().to_hex().to_string();

    let conn = self.conn.lock();
    conn.execute(
      "INSERT INTO tier1_entries (created_at, text, content_hash, expires_at) VALUES (?1, ?2, ?3, ?4)",
      params![
        created_at.to_rfc3339(),
        trimmed,
        content_hash,
        expires_at.to_rfc3339()
      ],
    )?;

    let tier1_id = conn.last_insert_rowid();
    Ok(tier1_id)
  }

  pub fn retrieve(&self, limit: usize) -> Result<MemorySnapshot, MemoryError> {
    self.clean_expired()?;
    let conn = self.conn.lock();
    let tier1 = self.query_tier1(&conn, limit)?;
    let tier2 = self.query_tier2(&conn, limit)?;
    let tier3 = self.query_tier3(&conn, limit)?;
    Ok(MemorySnapshot { tier1, tier2, tier3 })
  }

  pub fn compress(&self) -> Result<MemoryCompressionResult, MemoryError> {
    self.clean_expired()?;
    let conn = self.conn.lock();
    let mut stmt = conn.prepare(
      "SELECT id, text, content_hash FROM tier1_entries ORDER BY created_at ASC",
    )?;

    let mut rows = stmt.query([])?;
    let mut processed_ids = Vec::new();
    let mut compressed = 0;
    let mut skipped = 0;

    while let Some(row) = rows.next()? {
      let id: i64 = row.get("id")?;
      let text: String = row.get("text")?;
      let source_hash: String = row.get("content_hash")?;

      processed_ids.push(id);

      if self.summary_exists(&conn, &source_hash)? {
        skipped += 1;
        continue;
      }

      let summary = Self::summarize_text(&text);
      if summary.is_empty() {
        continue;
      }

      let mut hasher = Hasher::new();
      hasher.update(summary.as_bytes());
      let summary_hash = hasher.finalize().to_hex().to_string();
      let created_at = Utc::now();
      let expires_at = created_at + Duration::days(30);

      conn.execute(
        "INSERT INTO tier2_summaries (created_at, summary, summary_hash, expires_at, source_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
          created_at.to_rfc3339(),
          summary,
          summary_hash,
          expires_at.to_rfc3339(),
          source_hash
        ],
      )?;

      compressed += 1;
    }

    for id in processed_ids {
      conn.execute("DELETE FROM tier1_entries WHERE id = ?1", params![id])?;
    }

    self.set_meta("last_compress", &Utc::now().to_rfc3339())?;
    Ok(MemoryCompressionResult { compressed, skipped })
  }

  pub fn crystallize(&self) -> Result<MemoryCrystallizationResult, MemoryError> {
    self.clean_expired()?;
    let threshold = Utc::now() - Duration::days(30);
    let conn = self.conn.lock();
    let mut stmt = conn.prepare(
      "SELECT id, summary FROM tier2_summaries WHERE created_at <= ?1 ORDER BY created_at ASC",
    )?;
    let mut rows = stmt.query(params![threshold.to_rfc3339()])?;

    let mut processed_summaries = 0;
    let mut facts_created = 0;
    let mut processed_ids = Vec::new();

    while let Some(row) = rows.next()? {
      let summary_id: i64 = row.get("id")?;
      let summary: String = row.get("summary")?;
      let facts = Self::extract_facts(&summary);
      for fact in facts {
        let fact_text = fact.trim();
        if fact_text.is_empty() {
          continue;
        }

        conn.execute(
          "INSERT INTO tier3_crystallized (created_at, fact_text, summary_id) VALUES (?1, ?2, ?3)",
          params![Utc::now().to_rfc3339(), fact_text, summary_id],
        )?;

        self.graph
          .add_nodes_from_text(fact_text, Some(NodeCategory::Concept))?;

        facts_created += 1;
      }

      processed_summaries += 1;
      processed_ids.push(summary_id);
    }

    for id in processed_ids {
      conn.execute("DELETE FROM tier2_summaries WHERE id = ?1", params![id])?;
    }

    self.set_meta("last_crystallize", &Utc::now().to_rfc3339())?;
    Ok(MemoryCrystallizationResult {
      processed_summaries,
      facts_created,
    })
  }

  pub fn stats(&self) -> Result<MemoryStats, MemoryError> {
    let conn = self.conn.lock();
    let tier1_count: usize = conn.query_row("SELECT COUNT(*) FROM tier1_entries", [], |row| row.get(0))?;
    let tier2_count: usize = conn.query_row("SELECT COUNT(*) FROM tier2_summaries", [], |row| row.get(0))?;
    let tier3_count: usize = conn.query_row("SELECT COUNT(*) FROM tier3_crystallized", [], |row| row.get(0))?;
    let last_compress = self.get_meta_datetime(&conn, "last_compress")?;
    let last_crystallize = self.get_meta_datetime(&conn, "last_crystallize")?;
    Ok(MemoryStats {
      tier1_count,
      tier2_count,
      tier3_count,
      last_compress,
      last_crystallize,
    })
  }

  fn ensure_schema(&self) -> Result<(), MemoryError> {
    let conn = self.conn.lock();
    conn.execute_batch(
      "
      CREATE TABLE IF NOT EXISTS tier1_entries (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at TEXT NOT NULL,
        text TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        expires_at TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS tier1_created_idx ON tier1_entries(created_at);
      CREATE TABLE IF NOT EXISTS tier2_summaries (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at TEXT NOT NULL,
        summary TEXT NOT NULL,
        summary_hash TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        source_hash TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS tier2_created_idx ON tier2_summaries(created_at);
      CREATE UNIQUE INDEX IF NOT EXISTS tier2_source_hash_idx ON tier2_summaries(source_hash);
      CREATE TABLE IF NOT EXISTS tier3_crystallized (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at TEXT NOT NULL,
        fact_text TEXT NOT NULL,
        summary_id INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS tier3_created_idx ON tier3_crystallized(created_at);
      CREATE TABLE IF NOT EXISTS memory_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
      );
      ",
    )?;
    Ok(())
  }

  fn clean_expired(&self) -> Result<(), MemoryError> {
    let now = Utc::now().to_rfc3339();
    let conn = self.conn.lock();
    conn.execute("DELETE FROM tier1_entries WHERE expires_at <= ?1", params![now])?;
    conn.execute("DELETE FROM tier2_summaries WHERE expires_at <= ?1", params![now])?;
    Ok(())
  }

  fn query_tier1(&self, conn: &Connection, limit: usize) -> Result<Vec<MemoryRecord>, MemoryError> {
    let mut stmt = conn.prepare(
      "SELECT id, created_at, text, expires_at FROM tier1_entries ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
      let created_at: DateTime<Utc> = DateTime::parse_from_rfc3339(row.get::<_, String>("created_at")?.as_str())?
        .with_timezone(&Utc);
      let expires_at = DateTime::parse_from_rfc3339(row.get::<_, String>("expires_at")?.as_str())?.with_timezone(&Utc);

      Ok(MemoryRecord {
        id: row.get("id")?,
        tier: MemoryTier::Tier1,
        created_at,
        expires_at: Some(expires_at),
        text: row.get("text")?,
      })
    })?;

    rows.collect::<Result<Vec<_>, _>>()?
  }

  fn query_tier2(&self, conn: &Connection, limit: usize) -> Result<Vec<MemoryRecord>, MemoryError> {
    let mut stmt = conn.prepare(
      "SELECT id, created_at, summary, expires_at FROM tier2_summaries ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
      let created_at: DateTime<Utc> = DateTime::parse_from_rfc3339(row.get::<_, String>("created_at")?.as_str())?
        .with_timezone(&Utc);
      let expires_at = DateTime::parse_from_rfc3339(row.get::<_, String>("expires_at")?.as_str())?.with_timezone(&Utc);

      Ok(MemoryRecord {
        id: row.get("id")?,
        tier: MemoryTier::Tier2,
        created_at,
        expires_at: Some(expires_at),
        text: row.get("summary")?,
      })
    })?;

    rows.collect::<Result<Vec<_>, _>>()?
  }

  fn query_tier3(&self, conn: &Connection, limit: usize) -> Result<Vec<MemoryRecord>, MemoryError> {
    let mut stmt = conn.prepare(
      "SELECT id, created_at, fact_text FROM tier3_crystallized ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
      let created_at: DateTime<Utc> = DateTime::parse_from_rfc3339(row.get::<_, String>("created_at")?.as_str())?
        .with_timezone(&Utc);

      Ok(MemoryRecord {
        id: row.get("id")?,
        tier: MemoryTier::Tier3,
        created_at,
        expires_at: None,
        text: row.get("fact_text")?,
      })
    })?;

    rows.collect::<Result<Vec<_>, _>>()?
  }

  fn summary_exists(&self, conn: &Connection, source_hash: &str) -> Result<bool, MemoryError> {
    let mut stmt = conn.prepare("SELECT 1 FROM tier2_summaries WHERE source_hash = ?1 LIMIT 1")?;
    let exists = stmt.query_row(params![source_hash], |_| Ok(())).optional()?;
    Ok(exists.is_some())
  }

  fn set_meta(&self, key: &str, value: &str) -> Result<(), MemoryError> {
    let conn = self.conn.lock();
    conn.execute(
      "INSERT INTO memory_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      params![key, value],
    )?;
    Ok(())
  }

  fn get_meta_datetime(&self, conn: &Connection, key: &str) -> Result<Option<DateTime<Utc>>, MemoryError> {
    let value: Option<String> = conn
      .query_row("SELECT value FROM memory_meta WHERE key = ?1", params![key], |row| row.get(0))
      .optional()?;
    match value {
      Some(value) => {
        let dt = DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc);
        Ok(Some(dt))
      }
      None => Ok(None),
    }
  }

  fn summarize_text(text: &str) -> String {
    let sentences = Self::extract_sentences(text);
    if sentences.is_empty() {
      return text.trim().to_string();
    }
    sentences.into_iter().take(3).collect::<Vec<_>>().join(". ")
  }

  fn extract_facts(summary: &str) -> Vec<String> {
    let sentences = Self::extract_sentences(summary);
    sentences.into_iter().map(|s| {
      if s.ends_with('.') || s.ends_with('!') || s.ends_with('?') {
        s
      } else {
        format!("{}.", s)
      }
    }).collect()
  }

  fn extract_sentences(text: &str) -> Vec<String> {
    text.split_terminator(|c| matches!(c, '.' | '!' | '?'))
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(String::from)
      .collect()
  }
}

#[derive(Error, Debug)]
pub enum MemoryError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("datetime parse error: {0}")]
  Chrono(#[from] chrono::ParseError),
  #[error("graph error: {0}")]
  Graph(#[from] crate::graph::GraphError),
  #[error("io error: {0}")]
  Io(#[from] std::io::Error),
  #[error("text is empty")]
  EmptyText,
}
