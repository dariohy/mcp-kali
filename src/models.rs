use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Interrupted,
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::TimedOut | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub tool: String,
    /// Private execution specification. Stored separately with restricted file
    /// permissions and never returned by the API or webhook.
    #[serde(skip_serializing, default)]
    pub argv: Vec<String>,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub timeout_seconds: u64,
    pub return_code: Option<i32>,
    pub error: Option<String>,
    pub webhook_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitJob {
    pub tool: Option<String>,
    pub argv: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub webhook_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ToolRequest {
    #[serde(flatten)]
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct OutputPage {
    pub job_id: Uuid,
    pub stream: String,
    pub offset: u64,
    pub next_offset: u64,
    pub truncated: bool,
    pub data: String,
}

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub service: &'static str,
    pub queued: usize,
    pub running: usize,
    pub max_concurrency: usize,
}
