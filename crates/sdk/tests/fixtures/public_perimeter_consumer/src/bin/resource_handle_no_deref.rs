//! A resource handle never derefs to an instance: it holds none, and
//! provider calls go through an attempt's own checkout.

use nebula_sdk::integration::resource::{Provider, ResourceHandle};

fn skip_the_facade<R: Provider>(row: &ResourceHandle<R>) -> &R::Instance {
    &**row
}

fn main() {}
