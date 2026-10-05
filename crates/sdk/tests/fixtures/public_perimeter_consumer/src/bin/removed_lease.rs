//! Negative: the `Lease` facade is no longer exported (0.27.0). It holds
//! one checkout for its whole life and its units are not journaled; an
//! action submits units on a `ResourceHandle<R>`.

use nebula_sdk::integration::resource;

fn main() {
    let _: Option<resource::Lease<()>> = None;
}
