//! Negative: `ResourceGuard` is no longer exported (0.27.0). A raw lease
//! bypasses the effect journal; an action holds a `ResourceHandle<R>`.

use nebula_sdk::integration::resource;

fn main() {
    let _: Option<resource::ResourceGuard<()>> = None;
}
