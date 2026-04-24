//! ADR-0035 §3 coherence-hole closure — EXTERNAL FORGE PROOF.
//!
//! This example lives in the *examples/* directory but is compiled as an
//! external consumer of `credential_proto_builtin` (the lib crate). It
//!  attempts to forge `BitbucketBearerPhantom` membership on a local
//! type that does NOT satisfy `BitbucketBearer` (or even `Credential`).
//!
//! Under ADR-0035 §3 sealing, this MUST FAIL because:
//!   1. `BitbucketBearerPhantom: sealed_caps::BearerSealed + …`
//!   2. `sealed_caps` is a crate-private module of credential-proto-builtin.
//!   3. External impls of `sealed_caps::BearerSealed` are impossible.
//!   4. Therefore `impl BitbucketBearerPhantom for LocalType` cannot satisfy
//!      the sealed supertrait bound.
//!
//! How to verify:
//!     cargo build --example compile_fail_external_forge 2>&1
//!
//! Expected failure: either E0277 "BearerSealed not satisfied" or an
//! error about the sealed path being private. Both are acceptable — both
//! prove the forge is rejected at compile time.
//!
//! This is the Alternative A coherence hole that ADR-0035 §Alternatives
//! considered warned about; the two-trait sealed form closes it.

use credential_proto_builtin::BitbucketBearerPhantom;

/// External plugin's hostile/naive local type.
struct RogueCredential;

impl BitbucketBearerPhantom for RogueCredential {}

fn main() {}
