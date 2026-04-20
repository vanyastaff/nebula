//! Minimal "hello world" stateless action.
//!
//! Demonstrates the lowest-boilerplate path for an integration author:
//! build an [`ActionMetadata`], wrap an `async` closure with `stateless_fn`,
//! and drive it through [`TestRuntime`] in-process.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p nebula-examples --bin hello_action
//! ```
//!
//! (`examples/` is a workspace member with `[[bin]]` targets, not a Cargo
//! `[[example]]` directory, so the standard `--example` shortcut doesn't
//! apply.)
//!
//! Expected output:
//!
//! ```text
//! kind:     stateless
//! output:   {"message":"Hello, Vanya!"}
//! duration: <wall-clock>
//! ```
//!
//! For a typed-input variant using `#[derive(HasSchema)]`, see the
//! `paginated_users` example. For a manual `StatelessAction` impl with
//! a custom struct, mirror the `AddAction` test in
//! `crates/action/src/stateless.rs`.

use nebula_sdk::{
    nebula_action::stateless_fn,
    prelude::{
        ActionError, ActionMetadata, TestContextBuilder, TestRuntime, Value, action_key, json,
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let metadata = ActionMetadata::new(
        action_key!("demo.hello"),
        "Hello",
        "Greets the caller by name (defaults to 'world').",
    );

    let action = stateless_fn::<_, Value, Value>(metadata, |input: Value| async move {
        let name = input.get("name").and_then(Value::as_str).unwrap_or("world");
        Ok::<Value, ActionError>(json!({ "message": format!("Hello, {name}!") }))
    });

    let ctx = TestContextBuilder::new().with_input(json!({ "name": "Vanya" }));
    let report = TestRuntime::new(ctx).run_stateless(action).await?;

    println!("kind:     {}", report.kind);
    println!("output:   {}", report.output);
    println!("duration: {:?}", report.duration);
    Ok(())
}
