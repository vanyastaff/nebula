//! Probe 4 — A static (non-`Refreshable`) credential cannot be passed
//! to a `where C: Refreshable`-bound dispatcher.
//!
//! Stub dispatcher mirrors the engine's
//! `RefreshDispatcher::for_credential::<C>()` bound. The real engine
//! dispatcher (`crates/engine/src/credential/resolver.rs::resolve_with_refresh`)
//! enforces the same `where C: Refreshable` shape; this fixture keeps
//! `nebula-engine` out of the `nebula-credential` dev-deps while
//! exercising the structural guarantee at the dispatch site.

use nebula_credential::{credentials::ApiKeyCredential, Refreshable};

/// Stand-in for the engine's `RefreshDispatcher::for_credential` —
/// `where C: Refreshable` is the canonical bound after §15.4.
struct RefreshDispatcher;

impl RefreshDispatcher {
    fn for_credential<C: Refreshable>() -> Self {
        Self
    }
}

fn main() {
    // E0277 — `ApiKeyCredential` does not implement `Refreshable`.
    let _ = RefreshDispatcher::for_credential::<ApiKeyCredential>();
}
