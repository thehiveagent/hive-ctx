use crate::{
  classifier::{Classifier, ClassifierResult},
  fingerprint::{FingerprintEntry, FingerprintStore},
  retrieval::{RetrievalEngine, RetrievalError, RetrievalResult, RetrievalWeights},
};
use std::{cmp::Ordering, collections::HashMap, fmt, sync::Arc};
use thiserror::Error;

const DEFAULT_TOKEN_BUDGET: usize = 300;

#[derive(Debug)]
pub struct PipelineResult {
  pub system_prompt: String,
  pub token_count: usize,
  pub layers: PipelineLayers,
}

#[derive(Debug)]
pub struct PipelineLayers {
  pub episodes: usize,
  pub graph_nodes: usize,
  pub fingerprint_entries: usize,
  pub fingerprint_mode: String,
  pub included_layers: Vec<String>,
}

impl fmt::Display for PipelineLayers {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "episodes={}, graph={}, fingerprint={} (mode={})",
      self.episodes, self.graph_nodes, self.fingerprint_entries, self.fingerprint_mode
    )
  }
}

#[derive(Debug, Error)]
pub enum PipelineError {
  #[error(transparent)]
  Retrieval(#[from] RetrievalError),
}

pub struct PipelineEngine {
  classifier: Arc<parking_lot::Mutex<Classifier>>,
  fingerprint: Arc<parking_lot::Mutex<FingerprintStore>>,
  retrieval: Arc<RetrievalEngine>,
}

impl PipelineEngine {
  pub fn new(
    classifier: Arc<parking_lot::Mutex<Classifier>>,
    fingerprint: Arc<parking_lot::Mutex<FingerprintStore>>,
    retrieval: Arc<RetrievalEngine>,
  ) -> Self {
    Self {
      classifier,
      fingerprint,
      retrieval,
    }
  }

  pub fn build(
    &self,
    message: &str,
    profile: HashMap<String, String>,
    token_budget: Option<usize>,
  ) -> Result<PipelineResult, PipelineError> {
    let mut classifier = self.classifier.lock();
    let classification = classifier.classify(message);
    let weights = RetrievalWeights {
      temporal: classification.temporal_weight,
      personal: classification.personal_weight,
      technical: classification.technical_weight,
      emotional: classification.emotional_weight,
    };
    drop(classifier);

    let mut retrieved = self.retrieval.search(message, weights, 24)?;
    retrieved.sort_by(|a, b| match b.score.partial_cmp(&a.score) {
      Some(ord) => ord,
      None => Ordering::Equal,
    });

    let mut episodes: Vec<RetrievalResult> =
      retrieved.iter().cloned().filter(|result| result.tier.is_some()).collect();
    let mut graph_nodes: Vec<RetrievalResult> =
      retrieved.into_iter().filter(|result| result.tier.is_none()).collect();

    let mut fingerprint = self.fingerprint.lock();
    let fingerprint_result = fingerprint.compile(profile, classification.session_state);
    let fingerprint_mode = if fingerprint_result.delta_only {
      "delta"
    } else {
      "full"
    }
    .to_string();
    let mut fingerprint_entries = fingerprint_result.entries;

    let mut episodes_tokens = episodes.iter().map(|entry| entry.tokens).sum::<usize>();
    let mut graph_tokens = graph_nodes.iter().map(|entry| entry.tokens).sum::<usize>();
    let mut fingerprint_tokens = fingerprint_entries
      .iter()
      .map(|entry| estimate_fingerprint_tokens(entry))
      .sum::<usize>();

    let mut total_tokens = episodes_tokens + graph_tokens + fingerprint_tokens;
    let budget = token_budget.unwrap_or(DEFAULT_TOKEN_BUDGET);

    while total_tokens > budget && !episodes.is_empty() {
      let removed = episodes.pop().expect("entry exists");
      episodes_tokens = episodes_tokens.saturating_sub(removed.tokens);
      total_tokens = total_tokens.saturating_sub(removed.tokens);
    }

    while total_tokens > budget && !graph_nodes.is_empty() {
      let removed = graph_nodes.pop().expect("entry exists");
      graph_tokens = graph_tokens.saturating_sub(removed.tokens);
      total_tokens = total_tokens.saturating_sub(removed.tokens);
    }

    while total_tokens > budget && !fingerprint_entries.is_empty() {
      let removed = fingerprint_entries.pop().expect("entry exists");
      let removed_tokens = estimate_fingerprint_tokens(&removed);
      fingerprint_tokens = fingerprint_tokens.saturating_sub(removed_tokens);
      total_tokens = total_tokens.saturating_sub(removed_tokens);
    }

    let included_layers = {
      let mut layers = Vec::new();
      if !episodes.is_empty() {
        layers.push("episodes".to_string());
      }
      if !graph_nodes.is_empty() {
        layers.push("graph".to_string());
      }
      if !fingerprint_entries.is_empty() {
        layers.push("fingerprint".to_string());
      }
      layers
    };

    let layers_info = PipelineLayers {
      episodes: episodes.len(),
      graph_nodes: graph_nodes.len(),
      fingerprint_entries: fingerprint_entries.len(),
      fingerprint_mode,
      included_layers,
    };

    let system_prompt = build_system_prompt(
      &classification,
      &episodes,
      &graph_nodes,
      &fingerprint_entries,
      &layers_info.fingerprint_mode,
    );

    Ok(PipelineResult {
      system_prompt,
      token_count: total_tokens,
      layers: layers_info,
    })
  }
}

fn estimate_fingerprint_tokens(entry: &FingerprintEntry) -> usize {
  let key_tokens = entry.key.split_whitespace().count();
  let value_tokens = entry.value.split_whitespace().count();
  (key_tokens + value_tokens).max(1)
}

fn build_system_prompt(
  classification: &ClassifierResult,
  episodes: &[RetrievalResult],
  graph_nodes: &[RetrievalResult],
  fingerprint_entries: &[FingerprintEntry],
  fingerprint_mode: &str,
) -> String {
  let mut sections = Vec::new();
  sections.push(format!(
    "Message classified as {} (state {}). Weights — temporal {:.2}, personal {:.2}, technical {:.2}, emotional {:.2}.",
    classification.message_type.as_str(),
    classification.session_state.as_str(),
    classification.temporal_weight,
    classification.personal_weight,
    classification.technical_weight,
    classification.emotional_weight,
  ));

  if let Some(summary) = summarize_entries("Episodes", episodes) {
    sections.push(summary);
  }
  if let Some(summary) = summarize_entries("Graph context", graph_nodes) {
    sections.push(summary);
  }
  if !fingerprint_entries.is_empty() {
    let samples: Vec<_> = fingerprint_entries
      .iter()
      .take(4)
      .map(|entry| format!("{}={}", entry.key, entry.value))
      .collect();
    sections.push(format!(
      "Fingerprint (mode={}): {}",
      fingerprint_mode,
      samples.join("; ")
    ));
  }

  sections.join("\n\n")
}

fn summarize_entries(label: &str, entries: &[RetrievalResult]) -> Option<String> {
  if entries.is_empty() {
    return None;
  }
  let snippets: Vec<_> = entries
    .iter()
    .take(3)
    .map(|entry| entry.text.trim())
    .filter(|text| !text.is_empty())
    .collect();
  if snippets.is_empty() {
    return None;
  }
  Some(format!("{}: {}", label, snippets.join(" | ")))
}
