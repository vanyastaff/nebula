//! Compile-fail probe.
//!
//! Schema is reachable only via the `Input`/`Output: HasSchema`
//! associated-type bound / `nebula_schema::schema_of`. FQS on the trait so
//! the only post-P3 failure is the removed method (not a missing impl, a
//! missing import, or an inherent-method shadow). `#[derive(Action)]`
//! provides a complete, current-shape impl, so the call site is the only
//! P3-sensitive symbol.

use nebula_action::Action;

#[derive(Action)]
#[action(key = "probe.p3", input = serde_json::Value, output = serde_json::Value)]
struct Probe;

fn main() {
    let _ = <Probe as Action>::input_schema();
}
