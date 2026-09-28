//! Compile-fail probe: `#[resource]` on `Lazy<ManagedRow<R>>`.
//!
//! A managed row facade checks nothing out when it is resolved, so there
//! is nothing to defer: the derive rejects the `Lazy` wrapper.

use nebula_action::Action;
use nebula_core::sync::Lazy;
use nebula_resource::call::ManagedRow;

#[derive(Action)]
#[action(
    key = "bad.lazy_managed_row",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct LazyRow<R: nebula_resource::resource::Provider> {
    #[resource]
    db: Lazy<ManagedRow<R>>,
}

fn main() {}
