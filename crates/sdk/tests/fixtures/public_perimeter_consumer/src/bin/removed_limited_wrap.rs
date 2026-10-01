//! Negative: the `Limited` closure family is removed (MIGRATION P10). A
//! limiter no longer wraps a client; each provider call goes through the
//! managed call facade (`ResourceHandle::submit` + `OperationCx::call`).

use nebula_sdk::integration::resource::{ResourceContext, ResourceLimiter};

fn wrap_client(ctx: &ResourceContext) {
    let limits = ctx.limits();
    let _client = ResourceLimiter::wrap(&limits, ());
}

fn main() {
    let _ = wrap_client;
}
