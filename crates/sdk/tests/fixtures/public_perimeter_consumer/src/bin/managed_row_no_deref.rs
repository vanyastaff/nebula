//! A managed row never derefs to an instance: it holds none, and provider
//! calls go through an attempt's own checkout.

use nebula_sdk::integration::resource::{ManagedRow, Provider};

fn skip_the_facade<R: Provider>(row: &ManagedRow<R>) -> &R::Instance {
    &**row
}

fn main() {}
