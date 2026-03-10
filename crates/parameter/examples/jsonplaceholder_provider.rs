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
use nebula_parameter::loader::LoaderCtx;
use nebula_parameter::option::SelectOption;
use nebula_parameter::runtime::ParameterValues;

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
        .with_option_loader(|ctx: LoaderCtx| async move {
            let users: Vec<User> = reqwest::Client::new()
                .get("https://jsonplaceholder.typicode.com/users")
                .send()
                .await
                .expect("HTTP request failed")
                .json()
                .await
                .expect("JSON deserialization failed");

            let filter = ctx.filter.as_deref().unwrap_or("").to_lowercase();

            users
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
                .collect()
        });

    let loader = field.option_loader().expect("loader is attached");

    // Full list.
    let ctx = LoaderCtx {
        field_id: "assigned_user".to_owned(),
        values: ParameterValues::new(),
        filter: None,
        cursor: None,
        credential: None,
    };

    let options = loader.call(ctx).await;

    println!("=== All users ({} total) ===", options.len());
    for opt in &options {
        println!(
            "  id={:<3}  {}  <{}>",
            opt.value,
            opt.label,
            opt.description.as_deref().unwrap_or(""),
        );
    }

    // Filtered list -- only names/usernames containing "le".
    let ctx_filtered = LoaderCtx {
        field_id: "assigned_user".to_owned(),
        values: ParameterValues::new(),
        filter: Some("le".to_owned()),
        cursor: None,
        credential: None,
    };

    let filtered = loader.call(ctx_filtered).await;

    println!("\n=== Filtered by \"le\" ({} results) ===", filtered.len());
    for opt in &filtered {
        println!(
            "  id={:<3}  {}  <{}>",
            opt.value,
            opt.label,
            opt.description.as_deref().unwrap_or(""),
        );
    }
}
