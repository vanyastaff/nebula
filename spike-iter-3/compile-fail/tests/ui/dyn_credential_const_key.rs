//! Bonus probe — `dyn Credential` is blocked by `const KEY`.
//!
//! Expected: E0038 — Credential is not dyn compatible because of associated const KEY.
//!
//! This is the diagnostic that governs question (a). `dyn Credential` CANNOT
//! be constructed directly; consumers must go through:
//!   1. Phantom-shim `dyn XPhantom` (no Credential supertrait, no const KEY).
//!   2. Concrete generic type parameter `C: Credential`.
//!   3. Narrower object-safe trait (AnyCredential-style — no assoc const).

use credential_proto::Credential;
use credential_proto_builtin::{ApiKeyState, BearerScheme, ApiKeyCredential};

fn main() {
    let _: Box<dyn Credential<Input = (), Scheme = BearerScheme, State = ApiKeyState>> =
        Box::new(ApiKeyCredential);
}
