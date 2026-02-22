use serde::{Deserialize, Serialize};

const TEMPORAL_KEYWORDS: &[&str] = &[
  "today", "now", "urgent", "recent", "latest", "currently", "breaking", "update", "timeline",
];
const PERSONAL_KEYWORDS: &[&str] = &[
  "i", "me", "you", "my", "your", "they", "we", "us", "team", "profile", "about me",
];
const TECHNICAL_KEYWORDS: &[&str] = &[
  "code", "api", "bug", "release", "deploy", "stack", "database", "schema", "debug", "error",
];
const EMOTIONAL_KEYWORDS: &[&str] = &[
  "feel", "love", "hate", "angry", "happy", "sad", "anxious", "excited", "frustrated", "relieved",
];

const QUESTION_KEYWORDS: &[&str] = &["how", "what", "why", "when", "where", "who", "can", "could", "would", "should"];
const TASK_KEYWORDS: &[&str] = &["please", "need to", "should", "must", "schedule", "deploy", "plan", "complete"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageType {
  Casual,
  Question,
  Task,
  Emotional,
}

impl MessageType {
  pub fn as_str(&self) -> &'static str {
    match self {
      MessageType::Casual => "casual",
      MessageType::Question => "question",
      MessageType::Task => "task",
      MessageType::Emotional => "emotional",
    }
  }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionState {
  ColdStart,
  Warm,
  ContextShift,
  EmotionalShift,
  TaskMode,
}

impl SessionState {
  pub fn as_str(&self) -> &'static str {
    match self {
      SessionState::ColdStart => "COLD_START",
      SessionState::Warm => "WARM",
      SessionState::ContextShift => "CONTEXT_SHIFT",
      SessionState::EmotionalShift => "EMOTIONAL_SHIFT",
      SessionState::TaskMode => "TASK_MODE",
    }
  }
}

#[derive(Debug)]
pub struct ClassifierResult {
  pub temporal_weight: f64,
  pub personal_weight: f64,
  pub technical_weight: f64,
  pub emotional_weight: f64,
  pub message_type: MessageType,
  pub session_state: SessionState,
}

pub struct Classifier {
  last_state: SessionState,
  last_message_type: Option<MessageType>,
  message_count: usize,
}

impl Default for Classifier {
  fn default() -> Self {
    Self {
      last_state: SessionState::ColdStart,
      last_message_type: None,
      message_count: 0,
    }
  }
}

impl Classifier {
  pub fn classify(&mut self, message: &str) -> ClassifierResult {
    let normalized = message.trim().to_lowercase();
    let temporal_weight = Self::score(&normalized, TEMPORAL_KEYWORDS);
    let personal_weight = Self::score(&normalized, PERSONAL_KEYWORDS);
    let technical_weight = Self::score(&normalized, TECHNICAL_KEYWORDS);
    let emotional_weight = Self::score(&normalized, EMOTIONAL_KEYWORDS);

    let message_type =
      Self::detect_message_type(&normalized, emotional_weight, temporal_weight, technical_weight);

    let session_state = if self.message_count == 0 {
      SessionState::ColdStart
    } else {
      self.determine_state(
        message_type,
        temporal_weight,
        personal_weight,
        technical_weight,
        emotional_weight,
      )
    };

    self.last_message_type = Some(message_type);
    self.last_state = session_state;
    self.message_count += 1;

    ClassifierResult {
      temporal_weight,
      personal_weight,
      technical_weight,
      emotional_weight,
      message_type,
      session_state,
    }
  }

  pub fn current_state(&self) -> SessionState {
    if self.message_count == 0 {
      SessionState::ColdStart
    } else {
      self.last_state
    }
  }

  fn score(message: &str, keywords: &[&str]) -> f64 {
    if keywords.is_empty() {
      return 0.0;
    }
    let matches = keywords
      .iter()
      .filter(|keyword| message.contains(*keyword))
      .count();
    (matches as f64 / keywords.len() as f64).min(1.0)
  }

  fn detect_message_type(
    message: &str,
    emotional_weight: f64,
    temporal_weight: f64,
    technical_weight: f64,
  ) -> MessageType {
    if emotional_weight > 0.6 {
      return MessageType::Emotional;
    }

    if Self::is_question(message) {
      return MessageType::Question;
    }

    if Self::is_task(message, temporal_weight, technical_weight) {
      return MessageType::Task;
    }

    MessageType::Casual
  }

  fn is_question(message: &str) -> bool {
    if message.ends_with('?') {
      return true;
    }
    QUESTION_KEYWORDS.iter().any(|keyword| {
      message.starts_with(keyword) || message.contains(&format!(" {} ", keyword))
    })
  }

  fn is_task(message: &str, temporal_weight: f64, technical_weight: f64) -> bool {
    if TASK_KEYWORDS.iter().any(|keyword| message.contains(keyword)) {
      return true;
    }
    temporal_weight > 0.7 || technical_weight > 0.7
  }

  fn determine_state(
    &self,
    message_type: MessageType,
    temporal_weight: f64,
    personal_weight: f64,
    technical_weight: f64,
    emotional_weight: f64,
  ) -> SessionState {
    if message_type == MessageType::Task {
      return SessionState::TaskMode;
    }

    if emotional_weight > 0.6 {
      return SessionState::EmotionalShift;
    }

    let previous_type = self.last_message_type.unwrap_or(MessageType::Casual);
    if message_type != previous_type && message_type != MessageType::Casual {
      return SessionState::ContextShift;
    }

    if temporal_weight > 0.6 || personal_weight > 0.6 || technical_weight > 0.6 {
      return SessionState::ContextShift;
    }

    SessionState::Warm
  }
}
