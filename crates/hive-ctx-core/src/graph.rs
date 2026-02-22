use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::{params, OptionalExtension, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

/// Categories of nodes recognized by the knowledge graph.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeCategory {
  Person,
  Place,
  Project,
  Concept,
  Emotion,
  State,
  Unknown,
}

impl NodeCategory {
  pub fn as_str(&self) -> &'static str {
    match self {
      NodeCategory::Person => "person",
      NodeCategory::Place => "place",
      NodeCategory::Project => "project",
      NodeCategory::Concept => "concept",
      NodeCategory::Emotion => "emotion",
      NodeCategory::State => "state",
      NodeCategory::Unknown => "unknown",
    }
  }

  pub fn from_str(value: &str) -> Self {
    match value.to_lowercase().as_str() {
      "person" => NodeCategory::Person,
      "place" => NodeCategory::Place,
      "project" => NodeCategory::Project,
      "concept" => NodeCategory::Concept,
      "emotion" => NodeCategory::Emotion,
      "state" => NodeCategory::State,
      _ => NodeCategory::Unknown,
    }
  }
}

#[derive(Debug, Clone)]
pub struct GraphNodeRecord {
  pub id: i64,
  pub label: String,
  pub category: NodeCategory,
  pub created_at: DateTime<Utc>,
  pub decay_score: f64,
  pub metadata: Option<String>,
}

impl GraphNodeRecord {
  fn from_row(row: &rusqlite::Row) -> Result<Self, GraphError> {
    let created_at: String = row.get("created_at")?;
    let created_at = DateTime::parse_from_rfc3339(&created_at)
      .map_err(GraphError::ParseDateTime)?
      .with_timezone(&Utc);

    Ok(Self {
      id: row.get("id")?,
      label: row.get("label")?,
      category: NodeCategory::from_str(row.get::<_, String>("category")?.as_str()),
      created_at,
      decay_score: row.get("decay_score")?,
      metadata: row.get("metadata")?,
    })
  }
}

#[derive(Clone)]
pub struct GraphDatabase {
  conn: Arc<parking_lot::Mutex<Connection>>,
}

impl GraphDatabase {
  pub fn open(path: &Path) -> Result<Self, GraphError> {
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::open(path)?;
    let db = Self {
      conn: Arc::new(parking_lot::Mutex::new(connection)),
    };
    db.ensure_schema()?;
    Ok(db)
  }

  fn ensure_schema(&self) -> Result<(), GraphError> {
    let conn = self.conn.lock();
    conn.execute_batch(
      "
      CREATE TABLE IF NOT EXISTS nodes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        label TEXT NOT NULL,
        category TEXT NOT NULL,
        created_at TEXT NOT NULL,
        decay_score REAL NOT NULL DEFAULT 1.0,
        metadata TEXT
      );
      CREATE UNIQUE INDEX IF NOT EXISTS nodes_label_category_idx ON nodes(label, category);
      CREATE TABLE IF NOT EXISTS edges (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        source_node INTEGER NOT NULL,
        target_node INTEGER NOT NULL,
        relationship_type TEXT NOT NULL,
        created_at TEXT NOT NULL,
        FOREIGN KEY(source_node) REFERENCES nodes(id),
        FOREIGN KEY(target_node) REFERENCES nodes(id)
      );
      CREATE INDEX IF NOT EXISTS edges_source_idx ON edges(source_node);
      CREATE INDEX IF NOT EXISTS edges_target_idx ON edges(target_node);
      ",
    )?;
    Ok(())
  }

  pub fn add_nodes_from_text(
    &self,
    text: &str,
    forced_category: Option<NodeCategory>,
  ) -> Result<Vec<GraphNodeRecord>, GraphError> {
    let options = Self::extract_entities(text);
    let mut results = Vec::new();

    for (label, category) in options.into_iter() {
      if let Some(node) = self.add_node(&label, forced_category.unwrap_or(category), None)? {
        results.push(node);
      }
    }

    if results.is_empty() {
      if let Some(node) = self.add_node(text.trim(), forced_category.unwrap_or(NodeCategory::Concept), None)? {
        results.push(node);
      }
    }

    Ok(results)
  }

  pub fn add_node(
    &self,
    label: &str,
    category: NodeCategory,
    metadata: Option<&str>,
  ) -> Result<Option<GraphNodeRecord>, GraphError> {
    let label = label.trim();
    if label.is_empty() {
      return Ok(None);
    }

    let now = Utc::now().to_rfc3339();
    let conn = self.conn.lock();
    conn.execute(
      "INSERT OR IGNORE INTO nodes (label, category, created_at, decay_score, metadata) VALUES (?1, ?2, ?3, 1.0, ?4)",
      params![label, category.as_str(), now, metadata],
    )?;

    let mut stmt = conn.prepare(
      "SELECT id, label, category, created_at, decay_score, metadata FROM nodes WHERE label = ?1 AND category = ?2",
    )?;
    let record = stmt.query_row(params![label, category.as_str()], |row| GraphNodeRecord::from_row(row))?;
    Ok(Some(record))
  }

  pub fn add_edge(
    &self,
    source_id: i64,
    target_id: i64,
    relationship_type: &str,
  ) -> Result<i64, GraphError> {
    let now = Utc::now().to_rfc3339();
    let conn = self.conn.lock();
    conn.execute(
      "INSERT INTO edges (source_node, target_node, relationship_type, created_at) VALUES (?1, ?2, ?3, ?4)",
      params![source_id, target_id, relationship_type.trim(), now],
    )?;
    Ok(conn.last_insert_rowid())
  }

  pub fn query(
    &self,
    pattern: Option<&str>,
    category: Option<NodeCategory>,
  ) -> Result<Vec<GraphNodeRecord>, GraphError> {
    let conn = self.conn.lock();
    let mut sql = "SELECT id, label, category, created_at, decay_score, metadata FROM nodes".to_string();
    let mut clauses = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(pat) = pattern {
      clauses.push("label LIKE ?");
      params_vec.push(Box::new(format!("%{}%", pat)));
    }

    if let Some(cat) = category {
      clauses.push("category = ?");
      params_vec.push(Box::new(cat.as_str().to_string()));
    }

    if !clauses.is_empty() {
      sql.push_str(" WHERE ");
      sql.push_str(&clauses.join(" AND "));
    }

    sql.push_str(" ORDER BY created_at DESC LIMIT 100");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_vec.iter().map(|p| &**p), |row| GraphNodeRecord::from_row(row))?;

    let mut result = Vec::new();
    for row in rows {
      result.push(row?);
    }

    Ok(result)
  }

  pub fn traverse(&self, start_id: i64, max_hops: usize) -> Result<Vec<GraphNodeRecord>, GraphError> {
    let mut visited = HashSet::new();
    let mut frontier = VecDeque::new();
    let mut collected = Vec::new();
    visited.insert(start_id);
    frontier.push_back((start_id, 0));

    let conn = self.conn.lock();
    while let Some((node_id, depth)) = frontier.pop_front() {
      if depth > max_hops {
        continue;
      }

      if let Some(node) = self.load_node(&conn, node_id)? {
        collected.push(node);
      }

      if depth == max_hops {
        continue;
      }

      for neighbor in self.neighbors(&conn, node_id)? {
        if visited.insert(neighbor) {
          frontier.push_back((neighbor, depth + 1));
        }
      }
    }

    Ok(collected)
  }

  pub fn decay_update(&self) -> Result<usize, GraphError> {
    let mut updated = 0;
    let conn = self.conn.lock();
    let mut stmt = conn.prepare(
      "SELECT id, created_at FROM nodes WHERE category IN ('emotion', 'state')",
    )?;

    let mut rows = stmt.query([])?;
    let now = Utc::now();
    while let Some(row) = rows.next()? {
      let id: i64 = row.get("id")?;
      let created_at: String = row.get("created_at")?;
      let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map_err(GraphError::ParseDateTime)?
        .with_timezone(&Utc);
      let age_hours = (now - created_at).num_minutes() as f64 / 60.0;
      let decay = (1.0 / (1.0 + age_hours / 24.0)).max(0.0);
      conn.execute("UPDATE nodes SET decay_score = ?1 WHERE id = ?2", params![decay, id])?;
      updated += 1;
    }

    Ok(updated)
  }

  fn load_node(&self, conn: &Connection, id: i64) -> Result<Option<GraphNodeRecord>, GraphError> {
    let mut stmt = conn.prepare(
      "SELECT id, label, category, created_at, decay_score, metadata FROM nodes WHERE id = ?1",
    )?;
    let maybe = stmt
      .query_row(params![id], |row| GraphNodeRecord::from_row(row))
      .optional()?;
    Ok(maybe)
  }

  fn neighbors(&self, conn: &Connection, node_id: i64) -> Result<Vec<i64>, GraphError> {
    let mut stmt = conn.prepare(
      "
      SELECT target_node FROM edges WHERE source_node = ?1
      UNION
      SELECT source_node FROM edges WHERE target_node = ?1
      ",
    )?;
    let rows = stmt.query_map(params![node_id], |row| row.get(0))?;

    let mut result = Vec::new();
    for row in rows {
      result.push(row?);
    }
    Ok(result)
  }

  fn extract_entities(text: &str) -> Vec<(String, NodeCategory)> {
    let regex = Regex::new(r"\b([A-Z][a-z]+(?:\s+[A-Z][a-z]+)*)\b").unwrap();
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let emotion_keywords = ["joy", "sadness", "anger", "trust", "fear", "surprise", "love"];
    let state_keywords = ["tired", "focused", "hungry", "stressed", "relaxed"];

    for mat in regex.find_iter(text) {
      let token = mat.as_str().trim();
      if token.len() < 3 || seen.contains(token) {
        continue;
      }
      seen.insert(token.to_string());
      let lower = token.to_lowercase();
      let category = if emotion_keywords.contains(&lower.as_str()) {
        NodeCategory::Emotion
      } else if state_keywords.contains(&lower.as_str()) {
        NodeCategory::State
      } else if lower.contains("project") || lower.contains("initiative") {
        NodeCategory::Project
      } else if lower.contains("concept") || lower.contains("idea") {
        NodeCategory::Concept
      } else if lower.contains("inc") || lower.contains("corp") || lower.contains("city") {
        NodeCategory::Place
      } else {
        NodeCategory::Person
      };
      results.push((token.to_string(), category));
    }

    results
  }
}

#[derive(Error, Debug)]
pub enum GraphError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("failed to parse timestamp: {0}")]
  ParseDateTime(#[from] chrono::ParseError),
  #[error("io error: {0}")]
  Io(#[from] std::io::Error),
}
