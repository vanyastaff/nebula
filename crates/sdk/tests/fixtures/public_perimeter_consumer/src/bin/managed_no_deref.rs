//! A managed call facade never derefs to the instance: provider calls go
//! through a granted attempt.

use nebula_sdk::integration::resource::{Managed, PinSlots, Provider};

fn skip_the_facade<R: Provider + PinSlots>(managed: &Managed<R>) -> &R::Instance {
    &**managed
}

fn main() {}
