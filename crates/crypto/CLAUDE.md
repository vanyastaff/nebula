# nebula-crypto — Claude Code orientation
> Agent quick-map for `crates/crypto/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Leaf crypto primitives — AES-256-GCM authenticated encryption (mandatory AAD) + Argon2id key derivation, with the `EncryptedData` envelope and `CryptoError` taxonomy.
**Layer:** Cross-cutting — depends only on `nebula-error` + the RustCrypto stack (root CLAUDE.md -> Layered Dependency Map); importable from any layer.

## Commands
- `cargo check -p nebula-crypto`
- `cargo nextest run -p nebula-crypto`  ·  doctests: `cargo test -p nebula-crypto --doc`

## Key files
- `src/lib.rs` — the entire crate (single file): `EncryptionKey`, `EncryptedData`, encrypt/decrypt fns, `CryptoError`, private `fresh_nonce`.

## Conventions & never-do
- **SEC-11:** no public no-AAD `encrypt`. Production callers MUST use `encrypt_with_aad` / `encrypt_with_key_id`; the AAD-free `encrypt_no_aad` is `#[cfg(test)]`-only — do not promote it to a non-test path.
- Plaintext outputs are wrapped in `Zeroizing<T>` and `EncryptionKey` is `ZeroizeOnDrop`. `EncryptedData` (nonce/ciphertext/tag) is public bytes by design — do NOT add a scrubbing `Drop`.
- `encrypt_with_key_id` rejects an empty `key_id` (`CryptoError::InvalidKeyId`) so rotation lookup can pick the decryption key — keep that invariant.
- `CryptoError` `code` strings keep the `CREDENTIAL:CRYPTO_*` prefix (stable across the credential stack) — do not rename them on a move.
- Keep this a leaf: no PKCE/OAuth-state helpers or `serde_base64` here — those stay in `nebula-credential` (travel with the OAuth protocol).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · [ADR-0088](../../docs/adr/0088-credential-subsystem-rewrite.md) (extracted from `nebula-credential`)
