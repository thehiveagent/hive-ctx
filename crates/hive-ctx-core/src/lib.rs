mod classifier;
mod fingerprint;
mod graph;
mod memory;
mod pipeline;
mod retrieval;

use classifier::{Classifier, ClassifierResult};
use fingerprint::{FingerprintEntry, FingerprintResult, FingerprintStore};
use graph::{GraphDatabase, GraphNodeRecord, NodeCategory};
use memory::{
  MemoryCompressionResult, MemoryCrystallizationResult, MemoryRecord, MemorySnapshot, MemoryStats,
  MemoryStore, MemoryTier,
};
use chrono::{DateTime, Utc};
use napi::{Error as NapiError, Result as NapiResult};
use napi_derive::napi;
use parking_lot::Mutex;
use retrieval::{RetrievalEngine, RetrievalError, RetrievalRankCandidate, RetrievalResult, RetrievalWeights};
use std::{collections::HashMap, path::Path, sync::Arc};

#[napi(object)]
pub struct GraphNodeDto {
  pub id: i64,
  pub label: String,
  pub category: String,
  pub created_at: String,
  pub decay_score: f64,
  pub metadata: Option<String>,
}

#[napi(object)]
pub struct RetrievalWeightsDto {
  pub temporal_weight: f64,
  pub personal_weight: f64,
  pub technical_weight: f64,
  pub emotional_weight: f64,
}

#[napi(object)]
pub struct RetrievalResultDto {
  pub source: String,
  pub text: String,
  pub score: f64,
  pub tokens: u32,
  pub created_at: String,
  pub category: Option<String>,
  pub tier: Option<String>,
}

#[napi(object)]
pub struct RetrievalCandidateDto {
  pub text: String,
  pub created_at: Option<String>,
  pub category: Option<String>,
  pub node_id: Option<i64>,
  pub tier: Option<String>,
}

#[napi(object)]
pub struct ClassifierResultDto {
  pub temporal_weight: f64,
  pub personal_weight: f64,
  pub technical_weight: f64,
  pub emotional_weight: f64,
  pub message_type: String,
  pub session_state: String,
}

impl From<ClassifierResult> for ClassifierResultDto {
  fn from(result: ClassifierResult) -> Self {
    Self {
      temporal_weight: result.temporal_weight,
      personal_weight: result.personal_weight,
      technical_weight: result.technical_weight,
      emotional_weight: result.emotional_weight,
      message_type: result.message_type.as_str().to_string(),
      session_state: result.session_state.as_str().to_string(),
    }
  }
}

#[napi(object)]
pub struct FingerprintEntryDto {
  pub key: String,
  pub value: String,
}

impl From<FingerprintEntry> for FingerprintEntryDto {
  fn from(entry: FingerprintEntry) -> Self {
    Self {
      key: entry.key,
      value: entry.value,
    }
  }
}

#[napi(object)]
pub struct FingerprintResultDto {
  pub entries: Vec<FingerprintEntryDto>,
  pub delta_only: bool,
  pub compiled_at: String,
}

impl From<FingerprintResult> for FingerprintResultDto {
  fn from(result: FingerprintResult) -> Self {
    Self {
      entries: result.entries.into_iter().map(FingerprintEntryDto::from).collect(),
      delta_only: result.delta_only,
      compiled_at: result.compiled_at.to_rfc3339(),
    }
  }
}

impl From<RetrievalResult> for RetrievalResultDto {
  fn from(result: RetrievalResult) -> Self {
    Self {
      source: result.source.to_string(),
      text: result.text,
      score: result.score,
      tokens: result.tokens as u32,
      created_at: result.created_at.to_rfc3339(),
      category: result.category.map(|cat| cat.as_str().to_string()),
      tier: result.tier.map(|tier| tier.as_str().to_string()),
    }
  }
}

fn map_retrieval_error(err: RetrievalError) -> NapiError {
  NapiError::from_reason(err.to_string())
}

fn retrieval_weights_from_dto(dto: RetrievalWeightsDto) -> RetrievalWeights {
  RetrievalWeights {
    temporal: dto.temporal_weight,
    personal: dto.personal_weight,
    technical: dto.technical_weight,
    emotional: dto.emotional_weight,
  }
}

fn parse_candidate(dto: RetrievalCandidateDto) -> NapiResult<RetrievalRankCandidate> {
  let created_at = if let Some(value) = dto.created_at {
    DateTime::parse_from_rfc3339(&value)
      .map_err(|err| NapiError::from_reason(err.to_string()))?
      .with_timezone(&Utc)
  } else {
    Utc::now()
  };

  let category = dto.category.map(|category| NodeCategory::from_str(&category));
  let tier = dto
    .tier
    .and_then(|tier| MemoryTier::from_str(&tier));

  Ok(RetrievalRankCandidate {
    text: dto.text,
    created_at,
    category,
    node_id: dto.node_id,
    tier,
  })
}

#[napi(object)]
pub struct MemoryRecordDto {
  pub id: i64,
  pub tier: String,
  pub created_at: String,
  pub expires_at: Option<String>,
  pub text: String,
}

impl From<MemoryRecord> for MemoryRecordDto {
  fn from(record: MemoryRecord) -> Self {
    Self {
      id: record.id,
      tier: record.tier.as_str().to_string(),
      created_at: record.created_at.to_rfc3339(),
      expires_at: record.expires_at.map(|dt| dt.to_rfc3339()),
      text: record.text,
    }
  }
}

#[napi(object)]
pub struct MemorySnapshotDto {
  pub tier1: Vec<MemoryRecordDto>,
  pub tier2: Vec<MemoryRecordDto>,
  pub tier3: Vec<MemoryRecordDto>,
}

#[napi(object)]
pub struct MemoryCompressionDto {
  pub compressed: u32,
  pub skipped: u32,
}

#[napi(object)]
pub struct MemoryCrystallizationDto {
  pub processed_summaries: u32,
  pub facts_created: u32,
}

#[napi(object)]
pub struct MemoryStatsDto {
  pub tier1_count: u32,
  pub tier2_count: u32,
  pub tier3_count: u32,
  pub last_compress: Option<String>,
  pub last_crystallize: Option<String>,
}

impl From<GraphNodeRecord> for GraphNodeDto {
  fn from(record: GraphNodeRecord) -> Self {
    Self {
      id: record.id,
      label: record.label,
      category: record.category.as_str().to_string(),
      created_at: record.created_at.to_rfc3339(),
      decay_score: record.decay_score,
      metadata: record.metadata,
    }
  }
}

#[napi]
pub struct HiveCtxEngine {
  storage_path: String,
  budget_tokens: Option<u32>,
  graph: Arc<GraphDatabase>,
  memory: Arc<MemoryStore>,
  classifier: Arc<Mutex<Classifier>>,
  fingerprint: Arc<Mutex<FingerprintStore>>,
  retrieval: Arc<RetrievalEngine>,
}

fn map_graph_error(err: graph::GraphError) -> NapiError {
  NapiError::from_reason(err.to_string())
}

fn map_memory_error(err: memory::MemoryError) -> NapiError {
  NapiError::from_reason(err.to_string())
}

fn parse_category(input: Option<String>) -> Option<NodeCategory> {
  input.map(|value| NodeCategory::from_str(&value))
}

fn snapshot_to_dto(snapshot: MemorySnapshot) -> MemorySnapshotDto {
  MemorySnapshotDto {
    tier1: snapshot.tier1.into_iter().map(MemoryRecordDto::from).collect(),
    tier2: snapshot.tier2.into_iter().map(MemoryRecordDto::from).collect(),
    tier3: snapshot.tier3.into_iter().map(MemoryRecordDto::from).collect(),
  }
}

fn compression_to_dto(result: MemoryCompressionResult) -> MemoryCompressionDto {
  MemoryCompressionDto {
    compressed: result.compressed as u32,
    skipped: result.skipped as u32,
  }
}

fn crystallization_to_dto(result: MemoryCrystallizationResult) -> MemoryCrystallizationDto {
  MemoryCrystallizationDto {
    processed_summaries: result.processed_summaries as u32,
    facts_created: result.facts_created as u32,
  }
}

fn stats_to_dto(stats: MemoryStats) -> MemoryStatsDto {
  MemoryStatsDto {
    tier1_count: stats.tier1_count as u32,
    tier2_count: stats.tier2_count as u32,
    tier3_count: stats.tier3_count as u32,
    last_compress: stats.last_compress.map(|dt| dt.to_rfc3339()),
    last_crystallize: stats.last_crystallize.map(|dt| dt.to_rfc3339()),
  }
}

#[napi]
impl HiveCtxEngine {
  fn try_new(storage_path: String, budget_tokens: Option<u32>) -> NapiResult<Self> {
    let storage_dir = Path::new(&storage_path);
    std::fs::create_dir_all(storage_dir).map_err(|err| {
      NapiError::from_reason(format!("failed to create storage directory: {}", err))
    })?;

    let graph_path = storage_dir.join("hive_graph.sqlite");
    let graph = Arc::new(GraphDatabase::open(&graph_path).map_err(map_graph_error)?);
    let memory_path = storage_dir.join("hive_memory.sqlite");
    let memory = Arc::new(
      MemoryStore::open(&memory_path, Arc::clone(&graph)).map_err(map_memory_error)?,
    );
    let classifier = Arc::new(Mutex::new(Classifier::default()));
    let fingerprint = Arc::new(Mutex::new(FingerprintStore::new()));
    let retrieval_engine = Arc::new(RetrievalEngine::new(Arc::clone(&graph), Arc::clone(&memory)));

    Ok(Self {
      storage_path,
      budget_tokens,
      graph,
      memory,
      classifier,
      fingerprint,
      retrieval: retrieval_engine,
    })
  }

  #[napi(constructor)]
  pub fn new(storage_path: String, budget_tokens: Option<u32>) -> Self {
    Self::try_new(storage_path, budget_tokens)
      .expect("HiveCtxEngine construction failed")
  }


  #[napi(getter)]
  pub fn storage_path(&self) -> String {
    self.storage_path.clone()
  }

  #[napi(getter)]
  pub fn budget_tokens(&self) -> Option<u32> {
    self.budget_tokens
  }

  #[napi]
  pub fn graph_add_node(
    &self,
    text: String,
    category: Option<String>,
  ) -> NapiResult<Vec<GraphNodeDto>> {
    let forced_category = parse_category(category);
    let nodes = self
      .graph
      .add_nodes_from_text(&text, forced_category)
      .map_err(map_graph_error)?;
    Ok(nodes.into_iter().map(GraphNodeDto::from).collect())
  }

  #[napi]
  pub fn graph_add_edge(
    &self,
    source_node_id: i64,
    target_node_id: i64,
    relationship_type: String,
  ) -> NapiResult<i64> {
    let edge_id = self
      .graph
      .add_edge(source_node_id, target_node_id, relationship_type.trim())
      .map_err(map_graph_error)?;
    Ok(edge_id)
  }

  #[napi]
  pub fn graph_query(
    &self,
    pattern: Option<String>,
    category: Option<String>,
    limit: Option<u32>,
  ) -> NapiResult<Vec<GraphNodeDto>> {
    let nodes = self
      .graph
      .query(
        pattern.as_deref(),
        parse_category(category),
        limit.unwrap_or(100) as usize,
      )
      .map_err(map_graph_error)?;
    Ok(nodes.into_iter().map(GraphNodeDto::from).collect())
  }

  #[napi]
  pub fn graph_traverse(&self, start_node_id: i64, max_hops: u32) -> NapiResult<Vec<GraphNodeDto>> {
    let nodes = self
      .graph
      .traverse(start_node_id, max_hops as usize)
      .map_err(map_graph_error)?;
    Ok(nodes.into_iter().map(GraphNodeDto::from).collect())
  }

  #[napi]
  pub fn graph_decay_update(&self) -> NapiResult<u32> {
    let updated = self.graph.decay_update().map_err(map_graph_error)?;
    Ok(updated as u32)
  }

  #[napi]
  pub fn memory_store(&self, text: String) -> NapiResult<i64> {
    let id = self.memory.store(&text).map_err(map_memory_error)?;
    Ok(id)
  }

  #[napi]
  pub fn memory_retrieve(&self, limit: Option<u32>) -> NapiResult<MemorySnapshotDto> {
    let snapshot = self
      .memory
      .retrieve(limit.unwrap_or(10) as usize)
      .map_err(map_memory_error)?;
    Ok(snapshot_to_dto(snapshot))
  }

  #[napi]
  pub fn memory_compress(&self) -> NapiResult<MemoryCompressionDto> {
    let result = self.memory.compress().map_err(map_memory_error)?;
    Ok(compression_to_dto(result))
  }

  #[napi]
  pub fn memory_crystallize(&self) -> NapiResult<MemoryCrystallizationDto> {
    let result = self.memory.crystallize().map_err(map_memory_error)?;
    Ok(crystallization_to_dto(result))
  }

  #[napi]
  pub fn memory_stats(&self) -> NapiResult<MemoryStatsDto> {
    let stats = self.memory.stats().map_err(map_memory_error)?;
    Ok(stats_to_dto(stats))
  }

  #[napi]
  pub fn retrieval_search(
    &self,
    text: String,
    weights: RetrievalWeightsDto,
    limit: Option<u32>,
  ) -> NapiResult<Vec<RetrievalResultDto>> {
    let limit = limit.unwrap_or(12) as usize;
    let result = self
      .retrieval
      .search(&text, retrieval_weights_from_dto(weights), limit)
      .map_err(map_retrieval_error)?;
    Ok(result.into_iter().map(RetrievalResultDto::from).collect())
  }

  #[napi]
  pub fn retrieval_rank(
    &self,
    text: String,
    weights: RetrievalWeightsDto,
    candidates: Vec<RetrievalCandidateDto>,
    limit: Option<u32>,
  ) -> NapiResult<Vec<RetrievalResultDto>> {
    let parsed = candidates
      .into_iter()
      .map(parse_candidate)
      .collect::<Result<Vec<_>, _>>()?;
    let limit = limit.unwrap_or(12) as usize;
    let result = self
      .retrieval
      .rank(&text, retrieval_weights_from_dto(weights), parsed, limit)
      .map_err(map_retrieval_error)?;
    Ok(result.into_iter().map(RetrievalResultDto::from).collect())
  }

  #[napi]
  pub fn classify_message(&self, text: String) -> NapiResult<ClassifierResultDto> {
    let mut classifier = self.classifier.lock();
    let result = classifier.classify(&text);
    Ok(ClassifierResultDto::from(result))
  }

  #[napi]
  pub fn fingerprint_compile(
    &self,
    profile: HashMap<String, String>,
  ) -> NapiResult<FingerprintResultDto> {
    let state = self.classifier.lock().current_state();
    let mut fingerprint = self.fingerprint.lock();
    let result = fingerprint.compile(profile, state);
    Ok(FingerprintResultDto::from(result))
  }
}
