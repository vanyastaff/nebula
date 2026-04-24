//! Bonus probe — Pattern 2 rejects non-service credentials.
//!
//! Expected: E0277 — trait bound `…: BitbucketBearer` not satisfied,
//! leading to failure of BitbucketBearerPhantom blanket chain.
//!
//! Demonstrates ADR-0035 §1 compile-time rejection: a type satisfying
//! `Scheme = BearerScheme : AcceptsBearer` but NOT implementing
//! `BitbucketCredential` service marker is correctly rejected at
//! action declaration site.

use credential_proto::CredentialRef;
use credential_proto_builtin::{ApiKeyCredential, BitbucketBearerPhantom};

fn accept_bb<T: ?Sized + BitbucketBearerPhantom>() {}

fn main() {
    // ApiKeyCredential has Scheme = BearerScheme : AcceptsBearer.
    // But does NOT impl BitbucketCredential service marker.
    // So does NOT satisfy BitbucketBearer real trait.
    // So does NOT satisfy sealed_caps::BearerSealed.
    // So does NOT satisfy BitbucketBearerPhantom.
    accept_bb::<ApiKeyCredential>();
}
