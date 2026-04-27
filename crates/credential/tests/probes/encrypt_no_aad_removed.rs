//! Probe — SEC-11 hardening: bare `encrypt(key, plaintext)` removed
//! from `nebula-credential`'s public surface.
//!
//! Both access paths must fail to compile:
//!   1. Module path: `nebula_credential::secrets::crypto::encrypt` — the
//!      function was renamed to `encrypt_no_aad` AND tightened to
//!      `pub(crate)`. External lookup hits «cannot find function».
//!   2. Prelude import: `nebula_credential::encrypt` — the lib root
//!      re-export was removed. External lookup hits unresolved import.
//!
//! The probe captures the first failure mode (unresolved name in the
//! crypto module path). The prelude re-export removal is captured as a
//! second compile error in the same fixture.

use nebula_credential::secrets::EncryptionKey;

fn must_not_compile() {
    let key = EncryptionKey::from_bytes([0x42; 32]);
    let plaintext = b"plaintext";
    // E0425: cannot find function `encrypt` in module `crypto`.
    let _ = nebula_credential::secrets::crypto::encrypt(&key, plaintext);
    // E0432: unresolved import `nebula_credential::encrypt` — bare
    // `encrypt` was removed from the prelude re-export when the function
    // was scoped down. (Captured below at use-site to keep it inside the
    // probe body.)
    use nebula_credential::encrypt;
    let _ = encrypt(&key, plaintext);
}

fn main() {
    let _ = must_not_compile;
}
