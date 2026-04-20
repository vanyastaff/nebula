//! Regression test for `simple_action!` macro expansion.
//!
//! The macro previously expanded to `impl ProcessAction`, a trait that no
//! longer exists in `nebula-action`. The fix points it at `StatelessAction`
//! (canon §3.5 trait family) — this test compiles a macro use-site and
//! drives it through `TestRuntime` so the expansion stays correct.

use nebula_sdk::{prelude::*, simple_action};

simple_action! {
    name: EchoAction,
    key: "test.echo",
    input: Value,
    output: Value,
    async fn execute(&self, input, _ctx) {
        Ok(ActionResult::success(input))
    }
}

#[tokio::test]
async fn simple_action_macro_expands_to_runnable_stateless_action() {
    let ctx = TestContextBuilder::new().with_input(json!({"hello": "world"}));
    let report = TestRuntime::new(ctx)
        .run_stateless(EchoAction)
        .await
        .expect("EchoAction should run");

    assert_eq!(report.kind, "stateless");
    assert_eq!(report.output, json!({"hello": "world"}));
    assert_eq!(report.iterations, 1);
}
