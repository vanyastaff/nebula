//! Catalog DTOs — action and plugin catalog response types.

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Summary entry in the action list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListActionsResponse {
    /// All registered actions
    pub actions: Vec<ActionSummary>,
}

/// Detailed action metadata response for `GET /actions/{key}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Input parameter schema of an action, for `GET /actions/{key}/parameters`.
///
/// Editors render a node's parameter form from it. Kept apart from
/// [`ActionDetailResponse`] so the schema is fetched only when a form needs it.
///
/// `parameters` embeds `nebula-schema`'s own serialization of the admitted
/// schema, so a change to that format is a change to this response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActionParametersResponse {
    /// Action key (e.g. `"core.json_transform"`)
    pub key: String,
    /// Admitted input schema in the `nebula-schema` wire format. A record
    /// schema is `{"fields": [...]}`; other roots carry a `kind` (`"union"`,
    /// `"any"`, `"scalar"`), and `policy_version` and `root_rules` may appear.
    pub parameters: serde_json::Value,
    /// Execution kind in snake case: `stateless`, `stateful`, `stream`, `agent`, `interactive`,
    /// `control`, `trigger` or `resource`. Only `stateless`, `stateful`, `control` and `agent`
    /// actions can be workflow nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl ActionParametersResponse {
    /// Whether the action can be a node of a workflow graph, as the workflow compiler admits it.
    /// An unknown kind is not refused here; publication judges it.
    #[must_use]
    pub fn is_graph_node(&self) -> bool {
        self.kind
            .as_deref()
            .is_none_or(|kind| matches!(kind, "stateless" | "stateful" | "control" | "agent"))
    }
}

/// Summary entry in the plugin list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ListPluginsResponse {
    /// All registered plugins
    pub plugins: Vec<PluginSummary>,
}

/// Detailed plugin metadata response for `GET /plugins/{key}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PluginDetailResponse {
    /// Plugin key (e.g. `"slack"`)
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// Short description
    pub description: String,
    /// Bundle semver version (e.g. `"1.2.0"`). One plugin, one version —
    /// multi-version runtime registry was removed in the plugin load-path
    /// stabilization.
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
