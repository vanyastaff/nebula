//! Catalog DTOs — action and plugin catalog response types.

use serde::{Deserialize, Serialize};

/// Summary entry in the action list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSummary {
    /// Action key (e.g. `"http.request"`)
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// Interface version as `"major.minor"` (e.g. `"1.0"`)
    pub version: String,
}

/// Response for `GET /actions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListActionsResponse {
    /// All registered actions
    pub actions: Vec<ActionSummary>,
}

/// Detailed action metadata response for `GET /actions/{key}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDetailResponse {
    /// Action key (e.g. `"http.request"`)
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// Short description
    pub description: String,
    /// Interface version as `"major.minor"`
    pub version: String,
    /// Isolation level name
    pub isolation_level: String,
}

/// Summary entry in the plugin list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSummary {
    /// Plugin key (e.g. `"slack"`)
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// Latest bundle semver version (e.g. `"1.2.0"`)
    pub version: String,
}

/// Response for `GET /plugins`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPluginsResponse {
    /// All registered plugins
    pub plugins: Vec<PluginSummary>,
}

/// Detailed plugin metadata response for `GET /plugins/{key}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDetailResponse {
    /// Plugin key (e.g. `"slack"`)
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// Short description
    pub description: String,
    /// Bundle semver version (e.g. `"1.2.0"`). One plugin, one version —
    /// multi-version runtime registry was removed in the plugin load-path
    /// stabilization (ADR-0027).
    pub version: String,
    /// Group hierarchy for UI categorization
    pub group: Vec<String>,
    /// Tags for filtering
    pub tags: Vec<String>,
    /// Optional icon URL (populated only when the manifest uses a URL-backed icon)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Optional author name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Optional SPDX license identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}
