//! §3.3 NEGATIVE-CASE PROOF — phantom-trait variant.
//!
//! After the iteration-1 phantom-trait adjustment, the §3.3 semantic guarantee
//! must survive: `BitbucketAppPassword` (Scheme = BasicScheme) must NOT
//! satisfy `BitbucketBearerPhantom`, because:
//!
//!   impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}
//!
//! and `BitbucketBearer` requires `T::Scheme: AcceptsBearer`, which BasicScheme
//! does not implement.
//!
//! How to verify:
//!     cargo build --example compile_fail_app_password_via_phantom 2>&1
//!
//! Expected: error[E0277] mentioning BasicScheme + AcceptsBearer.
//!
//! If this file COMPILES, the phantom-trait laundering BROKE §3.3 — and the
//! workaround would have to be replaced with H2/H3 or tag types.

use credential_proto_builtin::{BitbucketAppPassword, BitbucketBearerPhantom};

const fn _assert_phantom_bearer<T: BitbucketBearerPhantom>() {}

const _: () = {
    _assert_phantom_bearer::<BitbucketAppPassword>();
};

fn main() {}
