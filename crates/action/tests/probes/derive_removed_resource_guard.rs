//! Compile-fail probe: `#[resource]` on a `ResourceGuard<R>` field.
//!
//! Since 0.27.0 a `ResourceHandle<R>` is the only resource capability an
//! action can name: a lease bypasses the effect journal. The derive refuses
//! a `ResourceGuard<R>` slot in any wrapper with a migration hint.

use nebula_action::Action;
use nebula_core::sync::Lazy;
use nebula_resource::ResourceGuard;

#[derive(Action)]
#[action(
    key = "bad.removed_resource_guard",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct RequiredLease<R: nebula_resource::resource::Provider> {
    #[resource]
    db: ResourceGuard<R>,
}

#[derive(Action)]
#[action(
    key = "bad.removed_optional_lazy_resource_guard",
    input = serde_json::Value,
    output = serde_json::Value,
)]
struct OptionalLazyLease<R: nebula_resource::resource::Provider> {
    #[resource]
    db: Option<Lazy<nebula_resource::ResourceGuard<R>>>,
}

fn main() {}
