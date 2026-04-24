//! §3.3 NEGATIVE-CASE PROOF.
//!
//! `BitbucketAppPassword`'s Scheme is `BasicScheme`, which does NOT
//! implement `AcceptsBearer`. Therefore the blanket impl
//! `impl<T> BitbucketBearer for T where T: BitbucketCredential, T::Scheme: AcceptsBearer`
//! must NOT apply, and this assertion must fail to compile.
//!
//! How to verify:
//!     cd spike/credential-proto-builtin && cargo build --tests \
//!         --bin app_password_is_not_bearer 2>&1
//!
//! Expected: error[E0277]: the trait bound `BasicScheme: AcceptsBearer` is not
//! satisfied (or similar — the precise diagnostic depends on the resolver
//! chain, but the failure must mention BasicScheme + AcceptsBearer).
//!
//! If this file COMPILES, §3.3 has SEMANTIC FAILURE: the blanket impl is
//! too permissive and AppPassword silently satisfies BitbucketBearer.
//! That outcome triggers Fallback A per Strategy §3.7.

use credential_proto_builtin::{BitbucketAppPassword, BitbucketBearer};

const fn _assert_bearer<T: BitbucketBearer>() {}

const _: () = {
    _assert_bearer::<BitbucketAppPassword>();
};

fn main() {}
