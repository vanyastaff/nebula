//! Smallest action written against `nebula-sdk` alone.
//!
//! `simple_action!` declares a stateless action with draft metadata, and
//! `TestRuntime` runs it once in-process. Every Nebula name comes from the
//! SDK, so this file also compile-checks the integration-author path.
//!
//! `TestRuntime` is a single-action test harness, not a deployment runtime.
//! Registering an SDK-authored action into a deployment is not yet an SDK
//! workflow; see `crates/sdk/README.md`.
//!
//! ```shell
//! cargo run -p nebula-examples --example sdk_hello_action
//! ```

#![expect(
    clippy::print_stdout,
    reason = "example: printed output is the demonstration"
)]

use nebula_sdk::{prelude::*, simple_action};

simple_action! {
    name: HelloAction,
    key: "example.hello",
    input: Value,
    output: Value,
    async fn execute(&self, input, _ctx) {
        let name = input.get("name").and_then(Value::as_str).unwrap_or("world");
        Ok(ActionResult::success(json!({ "message": format!("Hello, {name}!") })))
    }
}

#[tokio::main]
async fn main() -> Result<(), ActionError> {
    let ctx = TestContextBuilder::new().with_input(json!({ "name": "Nebula" }));
    let report = TestRuntime::new(ctx).run_stateless(HelloAction).await?;

    println!("{} run output: {}", report.kind, report.output);
    assert_eq!(report.output, json!({ "message": "Hello, Nebula!" }));
    Ok(())
}
