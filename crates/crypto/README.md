# nebula-crypto

Cross-cutting cryptographic primitives for Nebula: **AES-256-GCM** authenticated
encryption (with mandatory AAD) and **Argon2id** key derivation, plus the
`EncryptedData` envelope and `CryptoError` taxonomy.

Extracted from `nebula-credential` per
[ADR-0088](../../docs/adr/0088-credential-subsystem-rewrite.md) so that:

- the credential contract crate no longer compiles `aes-gcm` / `argon2`, and
- `nebula-storage`'s `EncryptionLayer` consumes the primitives directly rather
  than reaching across into the credential crate.

This is a **leaf cross-cutting crate** — it depends only on `nebula-error` (for
the `Classify` error taxonomy) plus the RustCrypto stack. It is importable from
any layer.

## What lives here

| Item | Purpose |
|------|---------|
| `EncryptionKey` | 256-bit key, zeroize-on-drop; `derive_from_password` (Argon2id) / `from_bytes` |
| `EncryptedData` | versioned envelope: `key_id`, 96-bit nonce, ciphertext, 128-bit tag |
| `encrypt_with_aad` / `decrypt_with_aad` | AEAD with mandatory Additional Authenticated Data (record-swap defence) |
| `encrypt_with_key_id` | like `encrypt_with_aad`, records the key identity for rotation |
| `decrypt` | decrypt an envelope (rejects AAD-bound data when no AAD is supplied) |
| `CryptoError` | `Classify`-tagged crypto failure taxonomy |

## What does NOT live here

PKCE / OAuth-state helpers (`generate_pkce_verifier`, `generate_code_challenge`,
`generate_random_state`) and the `serde_base64` field helper stay in
`nebula-credential` — they travel with the OAuth protocol, not generic byte
crypto.

**SEC-11:** there is no public no-AAD `encrypt`. The AAD-free path
(`encrypt_no_aad`) is `#[cfg(test)]`-only and therefore uncallable from any
non-test build; production callers must use `encrypt_with_aad` /
`encrypt_with_key_id`.
