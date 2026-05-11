//! `SessionGraph` struct. See spec section 2.4.
//!
//! This is the canonical schema for the JSON object stored in
//! `session_graphs.graph_json` and injected into the system prompt at session
//! start.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGraph {
    pub sg_version: String,
    pub session_id: String,
    pub project_hash: String,
    pub created_at: String,
    pub last_updated: String,
    pub token_count: u32,

    pub project: ProjectInfo,
    pub state: WorkState,
    pub decisions: Vec<Decision>,
    pub conventions: Conventions,
    pub files: FilesInfo,
    pub errors: Vec<ErrorEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: Option<String>,
    pub stack: Vec<String>,
    pub entry_points: Vec<String>,
    pub package_manager: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkState {
    pub current_task: Option<String>,
    pub progress: Option<String>,
    pub next_steps: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub topic: String,
    pub decision: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conventions {
    pub naming: Option<String>,
    pub structure: Option<String>,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesInfo {
    pub active: Vec<String>,
    pub read: Vec<String>,
    pub created: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEntry {
    pub file: Option<String>,
    pub description: String,
    pub resolution: Option<String>,
}
