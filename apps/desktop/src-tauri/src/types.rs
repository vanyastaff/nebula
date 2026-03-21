use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub status: AuthStatus,
    pub provider: Option<String>,
    pub access_token: String,
    pub user: Option<UserProfile>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    SignedOut,
    Authorizing,
    SignedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub mode: ConnectionMode,
    pub local_base_url: String,
    pub remote_base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    Local,
    Remote,
}

// ── Credential Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub metadata: CredentialMetadata,
    #[specta(type = String)]
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialMetadata {
    pub created_at: String,
    pub last_accessed: Option<String>,
    pub last_modified: String,
    pub version: u32,
    pub expires_at: Option<String>,
    #[specta(type = Option<u32>)]
    pub ttl_seconds: Option<u64>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateCredentialRequest {
    pub name: String,
    pub kind: String,
    #[specta(type = String)]
    pub state: serde_json::Value,
    pub tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCredentialRequest {
    pub name: Option<String>,
    #[specta(type = String)]
    pub state: Option<serde_json::Value>,
    pub tags: Option<HashMap<String, String>>,
}

// ── Auth User ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub provider: String,
}

// ── Workflow Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NodePort {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub port_type: String, // "input" or "output"
    pub data_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NodeData {
    pub action_type: String,
    pub label: String,
    #[specta(type = String)]
    pub parameters: serde_json::Value,
    pub inputs: Vec<NodePort>,
    pub outputs: Vec<NodePort>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub validation_errors: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String, // "action", "trigger", "condition", "loop"
    pub position: Position,
    pub data: NodeData,
    pub selected: Option<bool>,
    pub dragging: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub source_port: String,
    pub target: String,
    pub target_port: String,
    pub label: Option<String>,
    pub animated: Option<bool>,
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMetadata {
    pub created_at: String,
    pub last_modified: String,
    pub last_deployed: Option<String>,
    pub last_executed: Option<String>,
    pub version: u32,
    pub tags: HashMap<String, String>,
    pub author: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub status: String, // "draft", "active", "paused", "error", "archived"
    pub trigger_mode: String, // "manual", "scheduled", "event"
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub metadata: WorkflowMetadata,
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRequest {
    pub name: String,
    pub description: Option<String>,
    pub trigger_mode: String,
    pub tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkflowRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub trigger_mode: Option<String>,
    pub nodes: Option<Vec<WorkflowNode>>,
    pub edges: Option<Vec<WorkflowEdge>>,
    pub tags: Option<HashMap<String, String>>,
    pub server_url: Option<String>,
}
