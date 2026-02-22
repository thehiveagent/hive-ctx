use crate::{
  graph::{GraphDatabase, GraphNodeRecord, GraphError, NodeCategory},
  memory::{MemoryRecord, MemorySnapshot, MemoryStore, MemoryTier, MemoryError},
};
use chrono::{DateTime, Utc};
use std::{collections::HashSet, fmt, sync::Arc};
use thiserror::Error;

const EMOTIONAL_TERMS: &[&str] = &[
  "feel", "love", "hate", "angry", "happy", "sad", "anxious", "excited", "frustrated", "relieved",
  "stressed", "panicked", "calm", "breathe", "overwhelmed",
];

#[derive(Clone, Copy)]
pub struct RetrievalWeights {
  pub temporal: f64,
  pub personal: f64,
  pub technical: f64,
  pub emotional: f64,
}

#[derive(Debug, Error)]
pub enum RetrievalError {
  #[error("graph error: {0}")]
  Graph(#[from] GraphError),
  #[error("memory error: {0}")]
  Memory(#[from] MemoryError),
}

#[derive(Clone)]
#[derive(Debug)]
pub enum RetrievalSource {
  Graph(i64),
  Memory(MemoryTier, i64),
}

impl fmt::Display for RetrievalSource {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      RetrievalSource::Graph(node_id) => write!(f, "graph:{}", node_id),
      RetrievalSource::Memory(tier, entry_id) => write!(f, "memory:{:?}:{}", tier, entry_id),
    }
  }
}

#[derive(Debug, Clone)]
pub struct RetrievalResult {
  pub source: RetrievalSource,
  pub text: String,
  pub created_at: DateTime<Utc>,
  pub score: f64,
  pub tokens: usize,
  pub category: Option<NodeCategory>,
  pub tier: Option<MemoryTier>,
}

#[derive(Debug, Clone)]
pub struct RetrievalRankCandidate {
  pub text: String,
  pub created_at: DateTime<Utc>,
  pub category: Option<NodeCategory>,
  pub node_id: Option<i64>,
  pub tier: Option<MemoryTier>,
}

pub struct RetrievalEngine {
  graph: Arc<GraphDatabase>,
  memory: Arc<MemoryStore>,
}

impl RetrievalEngine {
  pub fn new(graph: Arc<GraphDatabase>, memory: Arc<MemoryStore>) -> Self {
    Self { graph, memory }
  }

  pub fn search(
    &self,
    message: &str,
    weights: RetrievalWeights,
    limit: usize,
  ) -> Result<Vec<RetrievalResult>, RetrievalError> {
    let limit = limit.max(1);
    let mut entries = Vec::new();
    let query_tokens = tokenize(message);

    let nodes = self.graph.query(None, None, limit * 3)?;
    let mut max_degree = 0usize;
    for node in nodes {
      let degree = self.graph.node_degree(node.id)?;
      max_degree = max_degree.max(degree);
      entries.push(self.build_from_graph(node, degree));
    }

    let snapshot = self.memory.retrieve(limit * 3)?;
    entries.extend(self.build_from_memory(snapshot));

    let scored = self.score_entries(entries, &query_tokens, weights, max_degree);
    let mut sorted: Vec<_> = scored.into_iter().collect();
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    sorted.truncate(limit);
    Ok(sorted)
  }

  pub fn rank(
    &self,
    message: &str,
    weights: RetrievalWeights,
    candidates: Vec<RetrievalRankCandidate>,
    limit: usize,
  ) -> Result<Vec<RetrievalResult>, RetrievalError> {
    let query_tokens = tokenize(message);
    let mut entries = Vec::new();
    let mut max_degree = 0usize;

    for candidate in candidates {
      let degree = if let Some(id) = candidate.node_id {
        let value = self.graph.node_degree(id)?;
        max_degree = max_degree.max(value);
        Some(value)
      } else {
        None
      };

      entries.push(RawEntry {
        source: match (candidate.tier, candidate.node_id) {
          (Some(tier), id) => RetrievalSource::Memory(tier, id.unwrap_or(0)),
          (None, Some(id)) => RetrievalSource::Graph(id),
          _ => RetrievalSource::Memory(MemoryTier::Tier2, 0),
        },
        text: candidate.text,
        created_at: candidate.created_at,
        category: candidate.category,
        degree,
      });
    }

    let scored = self.score_entries(entries, &query_tokens, weights, max_degree);
    let mut sorted: Vec<_> = scored.into_iter().collect();
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    sorted.truncate(limit.max(1));
    Ok(sorted)
  }

  fn build_from_graph(&self, node: GraphNodeRecord, degree: usize) -> RawEntry {
    RawEntry {
      source: RetrievalSource::Graph(node.id),
      text: node.label,
      created_at: node.created_at,
      category: Some(node.category),
      degree: Some(degree),
    }
  }

  fn build_from_memory(&self, snapshot: MemorySnapshot) -> Vec<RawEntry> {
    let mut entries = Vec::new();
    for record in snapshot.tier1.into_iter() {
      entries.push(self.memory_entry(record, MemoryTier::Tier1));
    }
    for record in snapshot.tier2.into_iter() {
      entries.push(self.memory_entry(record, MemoryTier::Tier2));
    }
    for record in snapshot.tier3.into_iter() {
      entries.push(self.memory_entry(record, MemoryTier::Tier3));
    }
    entries
  }

  fn memory_entry(&self, record: MemoryRecord, tier: MemoryTier) -> RawEntry {
    RawEntry {
      source: RetrievalSource::Memory(tier, record.id),
      text: record.text,
      created_at: record.created_at,
      category: None,
      degree: None,
    }
  }

  fn score_entries(
    &self,
    entries: Vec<RawEntry>,
    query_tokens: &HashSet<String>,
    weights: RetrievalWeights,
    max_degree: usize,
  ) -> Vec<RetrievalResult> {
    let graph_weight = (weights.personal + weights.technical) / 2.0;
    let semantic_weight = graph_weight;
    let denom = (weights.temporal + graph_weight + semantic_weight + weights.emotional).max(1e-6);
    let max_degree = max_degree.max(1);

    entries
      .into_iter()
      .map(|entry| {
        let recency_score = recency(entry.created_at);
        let centrality = entry.degree.map(|degree| degree as f64 / max_degree as f64).unwrap_or(0.0);
        let similarity = semantic_similarity(query_tokens, &entry.text);
        let emotional = emotional_relevance(&entry);
        let score = (recency_score * weights.temporal
          + centrality * graph_weight
          + similarity * semantic_weight
          + emotional * weights.emotional)
          / denom;

        let text = entry.text;
        let tokens = count_tokens(&text);
        let source = entry.source;
        let tier = match &source {
          RetrievalSource::Memory(tier, _) => Some(*tier),
          _ => None,
        };

        RetrievalResult {
          source,
          text,
          created_at: entry.created_at,
          score,
          tokens,
          category: entry.category,
          tier,
        }
      })
      .collect()
  }
}

fn semantic_similarity(query_tokens: &HashSet<String>, text: &str) -> f64 {
  if query_tokens.is_empty() {
    return 0.0;
  }

  let doc_tokens = tokenize(text);
  if doc_tokens.is_empty() {
    return 0.0;
  }

  let intersection = query_tokens.intersection(&doc_tokens).count();
  let union = query_tokens.union(&doc_tokens).count();
  if union == 0 {
    return 0.0;
  }
  intersection as f64 / union as f64
}

fn emotional_relevance(entry: &RawEntry) -> f64 {
  if matches!(
    entry.category,
    Some(NodeCategory::Emotion) | Some(NodeCategory::State)
  ) {
    return 1.0;
  }

  let tokens = tokenize(&entry.text);
  if tokens.is_empty() {
    return 0.0;
  }

  let matches = tokens
    .iter()
    .filter(|token| EMOTIONAL_TERMS.contains(&token.as_str()))
    .count();
  (matches as f64 / 5.0).min(1.0)
}

fn recency(created_at: DateTime<Utc>) -> f64 {
  let age_minutes = (Utc::now() - created_at).num_minutes() as f64;
  let age_hours = age_minutes / 60.0;
  (1.0 / (1.0 + age_hours / 24.0)).clamp(0.0, 1.0)
}

fn tokenize(text: &str) -> HashSet<String> {
  text.split(|c: char| !c.is_alphanumeric())
    .filter(|token| !token.is_empty())
    .map(|token| token.to_lowercase())
    .collect()
}

fn count_tokens(text: &str) -> usize {
  text.split_whitespace().count()
}

struct RawEntry {
  source: RetrievalSource,
  text: String,
  created_at: DateTime<Utc>,
  category: Option<NodeCategory>,
  degree: Option<usize>,
}
