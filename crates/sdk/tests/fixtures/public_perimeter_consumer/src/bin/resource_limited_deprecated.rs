//! Negative: the closure family is deprecated. An integration that denies
//! deprecated items fails to wrap its client; the managed call facade
//! replaces it.
#![deny(deprecated)]

use nebula_sdk::integration::resource::{NoThrottle, ResourceContext};

fn wrap_client(ctx: &ResourceContext) {
    let _client = ctx.limits().wrap((), NoThrottle);
}

fn main() {
    let _ = wrap_client;
}
