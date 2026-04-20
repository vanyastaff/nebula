//! Minimal "hello world" stateless action.
//!
//! Demonstrates the lowest-boilerplate path for an integration author:
//! build an [`ActionMetadata`], wrap an `async` closure with `stateless_fn`,
//! and drive it through [`TestRuntime`] in-process.
//!
//! Run with:
//!
//! ```bash
//! cargo run --example hello_action
//! ```
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

use nebula_sdk::{nebula_action::stateless_fn, prelude::*};

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
