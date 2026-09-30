//! A held lease facade never derefs to the instance: provider calls go
//! through a granted attempt.

use nebula_sdk::integration::resource::{Lease, PinSlots, Provider};

fn skip_the_facade<R: Provider + PinSlots>(lease: &Lease<R>) -> &R::Instance {
    &**lease
}

fn main() {}
