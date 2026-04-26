//! Probe 1 — §15.4 amendment: `CredentialState` requires `ZeroizeOnDrop`.
//!
//! `BadState` does not derive `Zeroize`/`ZeroizeOnDrop`; the supertrait
//! bound on `CredentialState` rejects the impl with E0277.

use nebula_credential::CredentialState;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct BadState {
    pub token: String, // no ZeroizeOnDrop
}

impl CredentialState for BadState {
    const KIND: &'static str = "bad_state";
    const VERSION: u32 = 1;
}

fn main() {}
