# nebula-crypto — design

| Field | Value |
|-------|-------|
| **Status** | Stable — leaf cross-cutting primitive |
| **Layer** | Cross-cutting (leaf; depends only on `nebula-error`) |
| **Redesign role** | **Direct artifact of the credential rewrite** — extracted from `nebula-credential` (ADR-0088); the `Cipher` / `Kdf` ports (ADR-0092) are the inversion seam that lets the credential `EncryptionLayer` become generic over the cipher. |
| **Related** | ADR-0088 D7, ADR-0092, PRODUCT_CANON §12.5 |

---

## 1. Purpose & boundaries

`nebula-crypto` is the workspace's **authenticated-encryption + KDF primitive**.
It exists so the credential contract crate does not pull `aes-gcm` / `argon2` /
`subtle`, and so `nebula-storage`'s `EncryptionLayer` can take primitives directly.

**Owns:** AES-256-GCM encrypt/decrypt, Argon2id key derivation, the `EncryptedData`
envelope (`version`, `key_id`, nonce, ciphertext, tag), the `CryptoError` taxonomy,
and the `Cipher` / `Kdf` ports.

**Explicitly does NOT own:** key storage / `KeyProvider` (that is `nebula-storage`),
secret *types* (`SecretString` lives in the credential/secret layer), envelope
*wiring* into the store decorator stack (`EncryptionLayer` in `nebula-storage`),
PKCE / OAuth state helpers (those travel with the OAuth protocol in
`nebula-credential::secrets`, not generic crypto).

## 2. Public surface

| Item | Where |
|------|-------|
| `CryptoError` (`#[non_exhaustive]`, `derive(Classify)`) | `src/lib.rs:34` |
| `EncryptionKey` (256-bit, `Zeroize + ZeroizeOnDrop`) | `src/lib.rs:74` |
| `EncryptionKey::derive_from_password` (Argon2id 19 MiB / t=2 / p=1) | `src/lib.rs:92` |
| `EncryptedData` (serde envelope; `key_id` `#[serde(default)]`) | `src/lib.rs:124` |
| `decrypt` / `encrypt_with_aad` / `decrypt_with_aad` / `encrypt_with_key_id` | `src/lib.rs:228..340` |
| `trait Cipher: Send + Sync` (ADR-0092 port; **no no-AAD encrypt method by construction**) | `src/lib.rs:386` |
| `trait Kdf: Send + Sync` | `src/lib.rs:437` |
| `AesGcmCipher` / `Argon2Kdf` (default impls) | `src/lib.rs:452 / 486` |

Whole crate is one `src/lib.rs` (~500 LoC + ~15 unit tests).

## 3. Dependencies & dependents

- **Deps:** `aes-gcm`, `argon2`, `zeroize`, `rand`, `serde`, `thiserror`, and
  `nebula-error` (the only workspace dep).
- **Dependents:** `nebula-credential` (injects `Arc<dyn Cipher>` / `Arc<dyn Kdf>`),
  `nebula-storage` (`EncryptionLayer`).

## 4. Invariants & contracts

- **SEC-11 — AAD mandatory in production.** The `Cipher` trait exposes **no**
  no-AAD encrypt method; `encrypt_no_aad` is `#[cfg(test)]`-only. This is the
  load-bearing invariant: a permissive cipher cannot be constructed through the
  port.
- **Key zeroization.** `EncryptionKey` is `ZeroizeOnDrop`; `decrypt*` returns
  `Zeroizing<Vec<u8>>`.
- **Versioned envelope.** `EncryptedData::CURRENT_VERSION = 1`; unknown version →
  `CryptoError::UnsupportedVersion` (forward-compat for algorithm agility).
- **Non-empty `key_id` for keyed path.** `encrypt_with_key_id` rejects an empty
  `key_id` (`InvalidKeyId`) — the key-rotation discriminator.

## 5. Known tensions / debt (honest)

1. **SEC-11 asymmetry.** No-AAD `decrypt` is `pub` (needed for legacy/pre-AAD
   data) while `encrypt_no_aad` is test-only — a public read path that production
   has no sanctioned way to *produce*. Acceptable for migration; document the
   sunset.
2. **`key_id` discipline split.** `encrypt_with_aad` writes an empty `key_id`, yet
   `EncryptedData.key_id` docs claim "the encryption layer rejects empty key IDs
   at runtime" — the non-empty invariant lives only in `encrypt_with_key_id`. The
   guarantee is by-convention on the consumer, not by-construction here.
3. **Legacy error-code prefix.** `CryptoError` codes keep the `CREDENTIAL:CRYPTO_*`
   prefix in a now-generic crate — deliberate compat debt (renaming breaks the
   whole credential stack).
4. **Dual surface.** Free functions and `Cipher` trait methods are identical (the
   trait delegates) — two equal entry points to the same operations.

## 6. Forward design

- **Algorithm agility** is the reason the ports exist: ChaCha20-Poly1305 and
  HSM/KMS-backed `Cipher` impls are the intended growth path (owner-accepted one
  vtable dispatch on the warm path per ADR-0092). Keep the port no-AAD-free.
- **Open question:** should the keyed path (`key_id` non-empty) become the *only*
  public encrypt surface, demoting `encrypt_with_aad` to make the rotation
  discriminator by-construction rather than by-convention? Decide before a second
  durable backend ships.
