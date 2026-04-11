//! Real-world async option loader that fetches users from the public
//! JSONPlaceholder API and surfaces them as select options.
//!
//! Demonstrates attaching an [`OptionLoader`](nebula_parameter::loader::OptionLoader)
//! inline to a [`Parameter::select`] — no registry or trait impl required.
//!
//! Run with:
//! ```text
//! cargo run -p nebula-parameter --example jsonplaceholder_provider
//! ```

use nebula_parameter::{
    loader::{LoaderContext, LoaderError},
    loader_result::LoaderResult,
    option::SelectOption,
    parameter::Parameter,
    parameter_type::ParameterType,
};

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
    // Build a Parameter::select with an inline async loader that hits JSONPlaceholder.
    let param = Parameter::select("assigned_user")
        .label("Assigned User")
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

    // Extract the loader from the parameter's Select type.
    let loader = match &param.param_type {
        ParameterType::Select { loader, .. } => loader.as_ref().expect("loader is attached"),
        _ => unreachable!("parameter is a select"),
    };

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
