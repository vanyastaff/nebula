//! Real-world `OptionProvider` that fetches users from the public
//! JSONPlaceholder API and surfaces them as select options.
//!
//! Run with:
//! ```text
//! cargo run -p nebula-parameter --example jsonplaceholder_provider
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use nebula_parameter::option::SelectOption;
use nebula_parameter::providers::{
    DynamicProviderEnvelope, DynamicResponseKind, OptionProvider, ProviderError, ProviderRegistry,
    ProviderRequest,
};
use nebula_parameter::values::ParameterValues;

// ── API shape ─────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct User {
    id: u64,
    name: String,
    username: String,
    email: String,
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Fetches users from JSONPlaceholder and converts them to [`SelectOption`]s.
///
/// Each option value is the numeric user `id`; the label is `"<name> (@<username>)"`.
/// The `description` field carries the user's e-mail address.
///
/// When the `filter` field in the request is set, only users whose name or
/// username contains that substring (case-insensitive) are returned.
struct JsonPlaceholderUserProvider {
    client: reqwest::Client,
}

impl JsonPlaceholderUserProvider {
    fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl OptionProvider for JsonPlaceholderUserProvider {
    async fn resolve(
        &self,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
        let users: Vec<User> = self
            .client
            .get("https://jsonplaceholder.typicode.com/users")
            .send()
            .await
            .map_err(|e| ProviderError::ResolveFailed {
                key: "jsonplaceholder.users".to_owned(),
                message: e.to_string(),
            })?
            .json()
            .await
            .map_err(|e| ProviderError::ResolveFailed {
                key: "jsonplaceholder.users".to_owned(),
                message: e.to_string(),
            })?;

        let filter = request.filter.as_deref().unwrap_or("").to_lowercase();

        let options = users
            .into_iter()
            .filter(|u| {
                filter.is_empty()
                    || u.name.to_lowercase().contains(&filter)
                    || u.username.to_lowercase().contains(&filter)
            })
            .map(|u| {
                let mut opt = SelectOption::new(
                    serde_json::json!(u.id),
                    format!("{} (@{})", u.name, u.username),
                );
                opt.description = Some(u.email);
                opt
            })
            .collect();

        Ok(DynamicProviderEnvelope::new(DynamicResponseKind::Options, options))
    }
}

// ── Example entry point ───────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Build and register the provider.
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider(
            "jsonplaceholder.users",
            Arc::new(JsonPlaceholderUserProvider::new()),
        )
        .expect("provider key is valid");

    // Full list.
    let request = ProviderRequest {
        field_id: "assigned_user".to_owned(),
        values: ParameterValues::new(),
        filter: None,
        cursor: None,
    };

    let envelope = registry
        .resolve_options("jsonplaceholder.users", &request)
        .await
        .expect("resolution should succeed");

    println!("=== All users ({} total) ===", envelope.items.len());
    for opt in &envelope.items {
        println!(
            "  id={:<3}  {}  <{}>",
            opt.value,
            opt.label,
            opt.description.as_deref().unwrap_or(""),
        );
    }

    // Filtered list.
    let filtered_request = ProviderRequest {
        field_id: "assigned_user".to_owned(),
        values: ParameterValues::new(),
        filter: Some("le".to_owned()),
        cursor: None,
    };

    let filtered = registry
        .resolve_options("jsonplaceholder.users", &filtered_request)
        .await
        .expect("filtered resolution should succeed");

    println!("\n=== Filtered by \"le\" ({} results) ===", filtered.items.len());
    for opt in &filtered.items {
        println!(
            "  id={:<3}  {}  <{}>",
            opt.value,
            opt.label,
            opt.description.as_deref().unwrap_or(""),
        );
    }
}
