//! Probe 1 — CredentialState impl without ZeroizeOnDrop fails.
//!
//! Expected: E0277 — `ZeroizeOnDrop` not satisfied.

use credential_proto::CredentialState;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
struct BadState {
    secret: String,
}

// MISSING: #[derive(Zeroize, ZeroizeOnDrop)] / manual impl.
impl CredentialState for BadState {}

fn main() {}
