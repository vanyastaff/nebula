//! Real-world async option loader that fetches users from the public
//! JSONPlaceholder API and surfaces them as select options.
//!
//! Demonstrates attaching an [`nebula_parameter::loader::OptionLoader`] inline
//! directly to a [`nebula_parameter::field::Field::Select`] variant -- no
//! registry or trait impl required.
//!
//! Run with:
//! ```text
//! cargo run -p nebula-parameter --example jsonplaceholder_provider
//! ```

use nebula_parameter::field::Field;
use nebula_parameter::loader::{LoaderContext, LoaderError};
use nebula_parameter::loader_result::LoaderResult;
use nebula_parameter::option::SelectOption;

// -- API shape ----------------------------------------------------------------

#[derive(serde::Deserialize)]
struct User {
    id: u64,
    name: String,
    username: String,
    email: String,
}

// -- Example entry point ------------------------------------------------------

#[tokio::main]
async fn main() {
    // Build a Field::Select with an inline async loader that hits JSONPlaceholder.
    let field = Field::select("assigned_user")
        .with_label("Assigned User")
        .with_option_loader(|ctx: LoaderContext| async move {
            let users: Vec<User> = reqwest::Client::new()
                .get("https://jsonplaceholder.typicode.com/users")
                .send()
                .await
                .map_err(|e| LoaderError::with_source("HTTP request failed", e))?
                .json()
                .await
                .map_err(|e| LoaderError::with_source("JSON deserialization failed", e))?;

            let filter = ctx.filter.as_deref().unwrap_or("").to_lowercase();

            let options: Vec<SelectOption> = users
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

            Ok(LoaderResult::done(options))
        });

    let loader = field.option_loader().expect("loader is attached");

    // Full list.
    let ctx = LoaderContext {
        field_id: "assigned_user".to_owned(),
        values: serde_json::Value::Object(serde_json::Map::new()),
        filter: None,
        cursor: None,
        credential: None,
        metadata: None,
    };

    let result = loader.call(ctx).await.expect("loader should succeed");

    println!("=== All users ({} total) ===", result.items.len());
    for opt in &result.items {
        println!(
            "  id={:<3}  {}  <{}>",
            opt.value,
            opt.label,
            opt.description.as_deref().unwrap_or(""),
        );
    }

    // Filtered list -- only names/usernames containing "le".
    let ctx_filtered = LoaderContext {
        field_id: "assigned_user".to_owned(),
        values: serde_json::Value::Object(serde_json::Map::new()),
        filter: Some("le".to_owned()),
        cursor: None,
        credential: None,
        metadata: None,
    };

    let filtered = loader
        .call(ctx_filtered)
        .await
        .expect("loader should succeed");

    println!(
        "\n=== Filtered by \"le\" ({} results) ===",
        filtered.items.len()
    );
    for opt in &filtered.items {
        println!(
            "  id={:<3}  {}  <{}>",
            opt.value,
            opt.label,
            opt.description.as_deref().unwrap_or(""),
        );
    }
}
