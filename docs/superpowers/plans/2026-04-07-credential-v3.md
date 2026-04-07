# nebula-credential v3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign nebula-credential with universal auth patterns, open AuthScheme trait, security-first encryption (key rotation, AAD hard cutover, Zeroizing buffers), and struct-based derive macros with field mapping.

**Architecture:** AuthScheme becomes an open trait with `AuthPattern` classification enum. 14 protocol-specific scheme types collapse to 12 universal patterns. EncryptionLayer gains multi-key support for key rotation. Derive macros enable 5-line credential definitions with `#[credential(into = "field")]` mapping.

**Tech Stack:** Rust 1.94, AES-256-GCM (aes-gcm), zeroize, serde, proc-macro2/syn/quote, nebula-parameter `#[derive(Parameters)]`

**Spec:** `docs/superpowers/specs/2026-04-06-credential-v3-design.md`

---

## Scope Note

This plan covers 6 phases. Each phase produces compiling, tested code and can be committed independently. Phases 1-2 are the most breaking (core trait + scheme renames). Phases 3-6 can proceed in any order after Phase 2.

**Blast radius:** AuthScheme trait change in nebula-core affects 29 files across 4 crates (core, credential, resource, action, sdk). Scheme renames affect nebula-sdk prelude.

**Spec deviation:** The spec lists `OAuth2Token.refresh_token` in the scheme struct. This contradicts the current (correct) design where refresh internals stay in `OAuth2State`, not the consumer-facing scheme. This plan keeps the current separation — refresh_token stays in State only.

---

## File Structure

### Phase 1: Foundation (nebula-core)
- Create: `crates/core/src/auth_pattern.rs`
- Modify: `crates/core/src/auth.rs`
- Modify: `crates/core/src/lib.rs`

### Phase 2: Universal Scheme Types (nebula-credential)
- Create: `crates/credential/src/scheme/secret_token.rs`
- Create: `crates/credential/src/scheme/identity_password.rs`
- Create: `crates/credential/src/scheme/key_pair.rs`
- Create: `crates/credential/src/scheme/signing_key.rs`
- Create: `crates/credential/src/scheme/federated_assertion.rs`
- Create: `crates/credential/src/scheme/challenge_secret.rs`
- Create: `crates/credential/src/scheme/otp_seed.rs`
- Create: `crates/credential/src/scheme/connection_uri.rs`
- Create: `crates/credential/src/scheme/instance_binding.rs`
- Create: `crates/credential/src/scheme/shared_key.rs`
- Delete: `crates/credential/src/scheme/bearer.rs`
- Delete: `crates/credential/src/scheme/basic.rs`
- Delete: `crates/credential/src/scheme/api_key.rs`
- Delete: `crates/credential/src/scheme/header.rs`
- Delete: `crates/credential/src/scheme/hmac.rs`
- Delete: `crates/credential/src/scheme/saml.rs`
- Delete: `crates/credential/src/scheme/kerberos.rs`
- Delete: `crates/credential/src/scheme/database.rs`
- Delete: `crates/credential/src/scheme/aws.rs`
- Delete: `crates/credential/src/scheme/ssh.rs`
- Delete: `crates/credential/src/scheme/ldap.rs`
- Modify: `crates/credential/src/scheme/certificate.rs` (rename type + update fields)
- Modify: `crates/credential/src/scheme/oauth2.rs` (update fields per spec)
- Modify: `crates/credential/src/scheme/coercion.rs` (rewrite for new types)
- Modify: `crates/credential/src/scheme/mod.rs` (new modules + exports)
- Modify: `crates/credential/src/credentials/mod.rs` (update identity_state! calls)
- Modify: `crates/credential/src/credentials/api_key.rs` (use SecretToken)
- Modify: `crates/credential/src/credentials/basic_auth.rs` (use IdentityPassword)
- Delete: `crates/credential/src/credentials/database.rs` (plugin concern per spec)
- Delete: `crates/credential/src/credentials/header_auth.rs` (covered by SecretToken)
- Modify: `crates/credential/src/credentials/oauth2.rs` (update OAuth2Token usage)
- Modify: `crates/credential/src/static_protocol.rs` (update examples)
- Modify: `crates/credential/src/lib.rs` (update exports)
- Modify: `crates/credential/src/snapshot.rs` (update type refs)
- Modify: `crates/credential/src/description.rs` (add pattern field)
- Modify: `crates/credential/src/registry.rs` (update type refs)
- Modify: `crates/credential/src/resolver.rs` (update type refs)
- Modify: `crates/credential/src/handle.rs` (update type refs)
- Modify: `crates/credential/src/context.rs` (update type refs)
- Modify: `crates/credential/tests/units/scheme_roundtrip_tests.rs`
- Modify: `crates/sdk/src/prelude.rs` (update re-exports)
- Modify: `crates/resource/src/resource.rs` (update if needed)
- Modify: `crates/action/src/context.rs` (update if needed)

### Phase 3: Security Hardening
- Modify: `crates/credential/src/crypto.rs` (key_id, Zeroizing, multi-key)
- Modify: `crates/credential/src/layer/encryption.rs` (multi-key, remove AAD fallback)

### Phase 4: AuthScheme Derive Macro
- Create: `crates/credential/macros/src/auth_scheme.rs`
- Modify: `crates/credential/macros/src/lib.rs`
- Modify: `crates/credential/src/state.rs` (keep identity_state! as fallback, deprecate)
- Modify: `crates/credential/src/credentials/mod.rs` (remove identity_state! calls)

### Phase 5: Credential Derive v3
- Modify: `crates/credential/macros/src/credential.rs` (struct-based with field mapping)
- Modify: `crates/credential/macros/src/lib.rs`
- Modify: `crates/credential/src/credentials/api_key.rs` (use new derive)
- Modify: `crates/credential/src/credentials/basic_auth.rs` (use new derive)

### Phase 6: API Surface Updates
- Modify: `crates/credential/src/credential.rs` (test() returns Option<TestResult>)
- Modify: `crates/credential/src/credentials/oauth2.rs` (update test() signature)
- Modify: `crates/credential/src/resolve.rs` (TestResult changes if needed)

---

## Phase 1: Foundation — AuthPattern + AuthScheme Trait

### Task 1: Add AuthPattern enum to nebula-core

**Files:**
- Create: `crates/core/src/auth_pattern.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Write test for AuthPattern**

Add to the bottom of `crates/core/src/auth_pattern.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_are_distinct() {
        let variants = [
            AuthPattern::SecretToken,
            AuthPattern::IdentityPassword,
            AuthPattern::OAuth2,
            AuthPattern::KeyPair,
            AuthPattern::Certificate,
            AuthPattern::RequestSigning,
            AuthPattern::FederatedIdentity,
            AuthPattern::ChallengeResponse,
            AuthPattern::OneTimePasscode,
            AuthPattern::ConnectionUri,
            AuthPattern::InstanceIdentity,
            AuthPattern::SharedSecret,
            AuthPattern::Custom,
        ];
        // All variants hash to different values
        let set: std::collections::HashSet<_> = variants.iter().collect();
        assert_eq!(set.len(), 13);
    }

    #[test]
    fn serde_round_trips() {
        let pattern = AuthPattern::OAuth2;
        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: AuthPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(pattern, deserialized);
    }

    #[test]
    fn debug_output_is_readable() {
        assert_eq!(format!("{:?}", AuthPattern::SecretToken), "SecretToken");
    }
}
```

- [ ] **Step 2: Write AuthPattern enum**

Create `crates/core/src/auth_pattern.rs`:

```rust
//! Classification of authentication patterns.
//!
//! [`AuthPattern`] groups auth schemes into universal categories for UI,
//! logging, and tooling. Each [`AuthScheme`](super::AuthScheme) implementation
//! declares its pattern via [`AuthScheme::pattern()`](super::AuthScheme::pattern).

use serde::{Deserialize, Serialize};

/// Classification of authentication patterns.
///
/// 12 built-in patterns cover the vast majority of auth mechanisms.
/// [`Custom`](AuthPattern::Custom) handles everything else.
///
/// # Examples
///
/// ```
/// use nebula_core::AuthPattern;
///
/// let pattern = AuthPattern::OAuth2;
/// assert_eq!(format!("{pattern:?}"), "OAuth2");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AuthPattern {
    /// Opaque secret string (API key, bearer token, session token).
    SecretToken,
    /// Identity + password pair (user/email/account + password).
    IdentityPassword,
    /// OAuth2/OIDC token set.
    OAuth2,
    /// Asymmetric key pair (SSH, PGP, crypto wallets).
    KeyPair,
    /// X.509 certificate + private key (mTLS, TLS client auth).
    Certificate,
    /// Request signing credentials (HMAC, SigV4, webhook signatures).
    RequestSigning,
    /// Third-party identity assertion (SAML, JWT, Kerberos ticket).
    FederatedIdentity,
    /// Challenge-response protocol credentials (Digest, NTLM, SCRAM).
    ChallengeResponse,
    /// TOTP/HOTP seed or OTP delivery config.
    OneTimePasscode,
    /// Compound connection URI (postgres://..., redis://...).
    ConnectionUri,
    /// Cloud/infrastructure instance identity (IMDS, managed identity).
    InstanceIdentity,
    /// Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).
    SharedSecret,
    /// Plugin-defined pattern not covered by built-in categories.
    Custom,
}
```

- [ ] **Step 3: Export AuthPattern from nebula-core**

In `crates/core/src/lib.rs`, add:

```rust
mod auth_pattern;
pub use auth_pattern::AuthPattern;
```

- [ ] **Step 4: Run tests**

Run: `cargo check -p nebula-core && cargo nextest run -p nebula-core`
Expected: All tests pass, including new AuthPattern tests.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/auth_pattern.rs crates/core/src/lib.rs
git commit -m "feat(core): add AuthPattern classification enum

12 universal auth patterns + Custom for plugin-defined schemes.
Used by AuthScheme::pattern() for UI/logging/tooling classification.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 2: Change AuthScheme trait to use pattern()

**Files:**
- Modify: `crates/core/src/auth.rs`

- [ ] **Step 1: Write test for new AuthScheme trait**

Add to `crates/core/src/auth.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPattern;

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    struct TestToken {
        value: String,
    }

    impl AuthScheme for TestToken {
        fn pattern() -> AuthPattern {
            AuthPattern::SecretToken
        }
    }

    #[test]
    fn custom_scheme_reports_correct_pattern() {
        assert_eq!(TestToken::pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn unit_scheme_is_none_pattern() {
        // () represents "no auth" — uses Custom since there's no None variant
        assert_eq!(<() as AuthScheme>::pattern(), AuthPattern::Custom);
    }
}
```

- [ ] **Step 2: Update AuthScheme trait**

Replace the trait in `crates/core/src/auth.rs`:

```rust
use crate::AuthPattern;

/// Consumer-facing authentication material.
///
/// Resources declare `type Auth: AuthScheme` to specify what auth
/// material they need. Credentials produce it via `Credential::project()`.
///
/// # Security contract
///
/// `Serialize + DeserializeOwned` bounds exist for the State = Scheme
/// identity path (static credentials stored directly). Serialization
/// to plaintext JSON happens **exclusively** inside `EncryptionLayer`.
/// Never serialize `AuthScheme` types in logging, debugging, or telemetry.
///
/// # Implementors
///
/// Built-in schemes are defined in `nebula-credential::scheme`.
/// The `()` type implements `AuthScheme` for resources that require
/// no authentication.
///
/// # Examples
///
/// ```
/// use nebula_core::{AuthScheme, AuthPattern};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct BearerToken {
///     token: String,
/// }
///
/// impl AuthScheme for BearerToken {
///     fn pattern() -> AuthPattern {
///         AuthPattern::SecretToken
///     }
/// }
/// ```
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Classification for UI, logging, and tooling.
    fn pattern() -> AuthPattern;

    /// When this auth material expires, if applicable.
    ///
    /// Used by the framework to schedule auto-refresh. Returns `None`
    /// for schemes that do not expire (the default).
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        None
    }
}

/// No authentication required.
impl AuthScheme for () {
    fn pattern() -> AuthPattern {
        AuthPattern::Custom
    }
}
```

- [ ] **Step 3: Run check to see what breaks**

Run: `cargo check -p nebula-core`
Expected: PASS (core itself compiles)

Run: `cargo check --workspace 2>&1 | head -50`
Expected: FAIL — downstream crates still use `const KIND`. This is expected; we fix them in Phase 2.

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/auth.rs
git commit -m "feat(core)!: change AuthScheme from const KIND to fn pattern()

BREAKING: AuthScheme trait now uses fn pattern() -> AuthPattern
instead of const KIND: &'static str. All implementors must update.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Phase 2: Universal Scheme Types

### Task 3: Create SecretToken (replaces BearerToken + ApiKeyAuth)

**Files:**
- Create: `crates/credential/src/scheme/secret_token.rs`
- Test: inline

- [ ] **Step 1: Write tests for SecretToken**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::AuthPattern;

    #[test]
    fn pattern_is_secret_token() {
        assert_eq!(SecretToken::pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn debug_redacts_token() {
        let token = SecretToken::new(SecretString::new("super-secret"));
        let debug = format!("{token:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn expose_returns_inner_value() {
        let token = SecretToken::new(SecretString::new("abc123"));
        token.token().expose_secret(|v| assert_eq!(v, "abc123"));
    }

    #[test]
    fn serde_round_trips() {
        let token = SecretToken::new(SecretString::new("test"));
        let json = serde_json::to_string(&token).unwrap();
        let restored: SecretToken = serde_json::from_str(&json).unwrap();
        restored.token().expose_secret(|v| assert_eq!(v, "test"));
    }
}
```

- [ ] **Step 2: Write SecretToken**

Create `crates/credential/src/scheme/secret_token.rs`:

```rust
//! Opaque secret string authentication (API key, bearer token, session token).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Opaque secret string for authentication.
///
/// Covers: API keys, bearer tokens, session tokens, service account keys.
/// Transport injection (HTTP header, query param) is NOT this type's concern —
/// that belongs to the resource/action layer.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::SecretToken;
/// use nebula_core::SecretString;
///
/// let token = SecretToken::new(SecretString::new("sk-abc123"));
/// token.token().expose_secret(|t| assert_eq!(t, "sk-abc123"));
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct SecretToken {
    #[serde(with = "nebula_core::serde_secret")]
    token: SecretString,
}

impl SecretToken {
    /// Creates a new secret token.
    #[must_use]
    pub fn new(token: SecretString) -> Self {
        Self { token }
    }

    /// Returns the token value.
    pub fn token(&self) -> &SecretString {
        &self.token
    }
}

impl AuthScheme for SecretToken {
    fn pattern() -> AuthPattern {
        AuthPattern::SecretToken
    }
}

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretToken")
            .field("token", &"[REDACTED]")
            .finish()
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p nebula-credential`
Expected: May fail if other scheme files still reference old AuthScheme::KIND. That's OK — we fix those files next.

- [ ] **Step 4: Commit**

```bash
git add crates/credential/src/scheme/secret_token.rs
git commit -m "feat(credential): add SecretToken universal scheme type

Replaces BearerToken and ApiKeyAuth with a single opaque secret
type. Transport injection is resource/action concern, not scheme's.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 4: Create IdentityPassword (replaces BasicAuth)

**Files:**
- Create: `crates/credential/src/scheme/identity_password.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::AuthPattern;

    #[test]
    fn pattern_is_identity_password() {
        assert_eq!(IdentityPassword::pattern(), AuthPattern::IdentityPassword);
    }

    #[test]
    fn debug_redacts_password() {
        let cred = IdentityPassword::new("admin", SecretString::new("pass123"));
        let debug = format!("{cred:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("admin"));
        assert!(!debug.contains("pass123"));
    }

    #[test]
    fn accessors_return_values() {
        let cred = IdentityPassword::new("user@example.com", SecretString::new("pw"));
        assert_eq!(cred.identity(), "user@example.com");
        cred.password().expose_secret(|v| assert_eq!(v, "pw"));
    }

    #[test]
    fn serde_round_trips() {
        let cred = IdentityPassword::new("user", SecretString::new("pass"));
        let json = serde_json::to_string(&cred).unwrap();
        let restored: IdentityPassword = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.identity(), "user");
    }
}
```

- [ ] **Step 2: Write IdentityPassword**

Create `crates/credential/src/scheme/identity_password.rs`:

```rust
//! Identity + password pair authentication.

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Identity + password pair.
///
/// Covers: username/password, email/password, account/password.
/// The `identity` field is intentionally generic — not "username" — because
/// it could be an email, account number, or any identifier.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::IdentityPassword;
/// use nebula_core::SecretString;
///
/// let cred = IdentityPassword::new("admin", SecretString::new("s3cret"));
/// assert_eq!(cred.identity(), "admin");
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct IdentityPassword {
    identity: String,
    #[serde(with = "nebula_core::serde_secret")]
    password: SecretString,
}

impl IdentityPassword {
    /// Creates a new identity + password credential.
    #[must_use]
    pub fn new(identity: impl Into<String>, password: SecretString) -> Self {
        Self {
            identity: identity.into(),
            password,
        }
    }

    /// Returns the identity (username, email, account).
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the password.
    pub fn password(&self) -> &SecretString {
        &self.password
    }
}

impl AuthScheme for IdentityPassword {
    fn pattern() -> AuthPattern {
        AuthPattern::IdentityPassword
    }
}

impl std::fmt::Debug for IdentityPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityPassword")
            .field("identity", &self.identity)
            .field("password", &"[REDACTED]")
            .finish()
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/credential/src/scheme/identity_password.rs
git commit -m "feat(credential): add IdentityPassword universal scheme type

Replaces BasicAuth. Generic identity field covers username, email,
account number — not protocol-specific.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 5: Create remaining 8 universal scheme types

**Files:**
- Create: `crates/credential/src/scheme/key_pair.rs`
- Create: `crates/credential/src/scheme/signing_key.rs`
- Create: `crates/credential/src/scheme/federated_assertion.rs`
- Create: `crates/credential/src/scheme/challenge_secret.rs`
- Create: `crates/credential/src/scheme/otp_seed.rs`
- Create: `crates/credential/src/scheme/connection_uri.rs`
- Create: `crates/credential/src/scheme/instance_binding.rs`
- Create: `crates/credential/src/scheme/shared_key.rs`

Each follows the same pattern as SecretToken/IdentityPassword. All secret fields use `SecretString` + `serde_secret`. All have redacting `Debug`. All implement `AuthScheme` with their pattern.

- [ ] **Step 1: Create KeyPair**

Create `crates/credential/src/scheme/key_pair.rs`:

```rust
//! Asymmetric key pair authentication (SSH, PGP, crypto wallets).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Asymmetric key pair.
///
/// Covers: SSH keys, PGP keys, crypto wallet keys.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::KeyPair;
/// use nebula_core::SecretString;
///
/// let kp = KeyPair::new("ssh-ed25519 AAAA...", SecretString::new("-----BEGIN..."));
/// assert!(kp.public_key().starts_with("ssh-"));
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct KeyPair {
    public_key: String,
    #[serde(with = "nebula_core::serde_secret")]
    private_key: SecretString,
    #[serde(with = "nebula_core::serde_secret_option")]
    passphrase: Option<SecretString>,
    algorithm: Option<String>,
}

impl KeyPair {
    /// Creates a new key pair.
    #[must_use]
    pub fn new(public_key: impl Into<String>, private_key: SecretString) -> Self {
        Self {
            public_key: public_key.into(),
            private_key,
            passphrase: None,
            algorithm: None,
        }
    }

    /// Sets the passphrase for the private key.
    #[must_use]
    pub fn with_passphrase(mut self, passphrase: SecretString) -> Self {
        self.passphrase = Some(passphrase);
        self
    }

    /// Sets the algorithm (e.g., "ed25519", "rsa-4096").
    #[must_use]
    pub fn with_algorithm(mut self, algorithm: impl Into<String>) -> Self {
        self.algorithm = Some(algorithm.into());
        self
    }

    /// Returns the public key.
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// Returns the private key.
    pub fn private_key(&self) -> &SecretString {
        &self.private_key
    }

    /// Returns the passphrase, if set.
    pub fn passphrase(&self) -> Option<&SecretString> {
        self.passphrase.as_ref()
    }

    /// Returns the algorithm, if set.
    pub fn algorithm(&self) -> Option<&str> {
        self.algorithm.as_deref()
    }
}

impl AuthScheme for KeyPair {
    fn pattern() -> AuthPattern {
        AuthPattern::KeyPair
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPair")
            .field("public_key", &self.public_key)
            .field("private_key", &"[REDACTED]")
            .field("passphrase", &self.passphrase.as_ref().map(|_| "[REDACTED]"))
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_key_pair() {
        assert_eq!(KeyPair::pattern(), AuthPattern::KeyPair);
    }

    #[test]
    fn debug_redacts_secrets() {
        let kp = KeyPair::new("pub", SecretString::new("priv"))
            .with_passphrase(SecretString::new("pass"));
        let debug = format!("{kp:?}");
        assert!(!debug.contains("priv"));
        assert!(!debug.contains("pass"));
    }
}
```

- [ ] **Step 2: Create SigningKey**

Create `crates/credential/src/scheme/signing_key.rs`:

```rust
//! Request signing credentials (HMAC, SigV4, webhook signatures).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Request signing key.
///
/// Covers: HMAC-SHA256, AWS SigV4, webhook signature verification.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::SigningKey;
/// use nebula_core::SecretString;
///
/// let key = SigningKey::new(SecretString::new("whsec_..."), "hmac-sha256");
/// assert_eq!(key.algorithm(), "hmac-sha256");
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct SigningKey {
    #[serde(with = "nebula_core::serde_secret")]
    key: SecretString,
    algorithm: String,
}

impl SigningKey {
    /// Creates a new signing key.
    #[must_use]
    pub fn new(key: SecretString, algorithm: impl Into<String>) -> Self {
        Self {
            key,
            algorithm: algorithm.into(),
        }
    }

    /// Returns the signing key.
    pub fn key(&self) -> &SecretString {
        &self.key
    }

    /// Returns the algorithm identifier.
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }
}

impl AuthScheme for SigningKey {
    fn pattern() -> AuthPattern {
        AuthPattern::RequestSigning
    }
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey")
            .field("key", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_request_signing() {
        assert_eq!(SigningKey::pattern(), AuthPattern::RequestSigning);
    }

    #[test]
    fn debug_redacts_key() {
        let key = SigningKey::new(SecretString::new("secret"), "sha256");
        let debug = format!("{key:?}");
        assert!(!debug.contains("secret"));
    }
}
```

- [ ] **Step 3: Create FederatedAssertion**

Create `crates/credential/src/scheme/federated_assertion.rs`:

```rust
//! Third-party identity assertion (SAML, JWT, Kerberos ticket).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Third-party identity assertion.
///
/// Covers: SAML assertions, JWT tokens, Kerberos tickets.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::FederatedAssertion;
/// use nebula_core::SecretString;
///
/// let assertion = FederatedAssertion::new(
///     SecretString::new("eyJhbGci..."),
///     "https://idp.example.com",
/// );
/// assert_eq!(assertion.issuer(), "https://idp.example.com");
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct FederatedAssertion {
    #[serde(with = "nebula_core::serde_secret")]
    assertion: SecretString,
    issuer: String,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl FederatedAssertion {
    /// Creates a new federated assertion.
    #[must_use]
    pub fn new(assertion: SecretString, issuer: impl Into<String>) -> Self {
        Self {
            assertion,
            issuer: issuer.into(),
            expires_at: None,
        }
    }

    /// Sets the expiration time.
    #[must_use]
    pub fn with_expires_at(mut self, at: chrono::DateTime<chrono::Utc>) -> Self {
        self.expires_at = Some(at);
        self
    }

    /// Returns the assertion value.
    pub fn assertion(&self) -> &SecretString {
        &self.assertion
    }

    /// Returns the issuer.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
}

impl AuthScheme for FederatedAssertion {
    fn pattern() -> AuthPattern {
        AuthPattern::FederatedIdentity
    }

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }
}

impl std::fmt::Debug for FederatedAssertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FederatedAssertion")
            .field("assertion", &"[REDACTED]")
            .field("issuer", &self.issuer)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_federated_identity() {
        assert_eq!(FederatedAssertion::pattern(), AuthPattern::FederatedIdentity);
    }
}
```

- [ ] **Step 4: Create ChallengeSecret**

Create `crates/credential/src/scheme/challenge_secret.rs`:

```rust
//! Challenge-response protocol credentials (Digest, NTLM, SCRAM).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Challenge-response protocol credentials.
///
/// Covers: HTTP Digest, NTLM, SCRAM-SHA-256.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChallengeSecret {
    identity: String,
    #[serde(with = "nebula_core::serde_secret")]
    secret: SecretString,
    protocol: String,
}

impl ChallengeSecret {
    /// Creates a new challenge-response credential.
    #[must_use]
    pub fn new(
        identity: impl Into<String>,
        secret: SecretString,
        protocol: impl Into<String>,
    ) -> Self {
        Self {
            identity: identity.into(),
            secret,
            protocol: protocol.into(),
        }
    }

    /// Returns the identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the secret.
    pub fn secret(&self) -> &SecretString {
        &self.secret
    }

    /// Returns the protocol identifier.
    pub fn protocol(&self) -> &str {
        &self.protocol
    }
}

impl AuthScheme for ChallengeSecret {
    fn pattern() -> AuthPattern {
        AuthPattern::ChallengeResponse
    }
}

impl std::fmt::Debug for ChallengeSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChallengeSecret")
            .field("identity", &self.identity)
            .field("secret", &"[REDACTED]")
            .field("protocol", &self.protocol)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_challenge_response() {
        assert_eq!(ChallengeSecret::pattern(), AuthPattern::ChallengeResponse);
    }
}
```

- [ ] **Step 5: Create OtpSeed**

Create `crates/credential/src/scheme/otp_seed.rs`:

```rust
//! TOTP/HOTP seed for one-time passcode generation.

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// TOTP/HOTP seed.
///
/// Covers: Google Authenticator, Authy, any RFC 6238/4226 compliant OTP.
#[derive(Clone, Serialize, Deserialize)]
pub struct OtpSeed {
    #[serde(with = "nebula_core::serde_secret")]
    seed: SecretString,
    algorithm: String,
    digits: u8,
    period: Option<u32>,
}

impl OtpSeed {
    /// Creates a new TOTP seed with default 6 digits.
    #[must_use]
    pub fn totp(seed: SecretString, algorithm: impl Into<String>) -> Self {
        Self {
            seed,
            algorithm: algorithm.into(),
            digits: 6,
            period: Some(30),
        }
    }

    /// Creates a new HOTP seed with default 6 digits.
    #[must_use]
    pub fn hotp(seed: SecretString, algorithm: impl Into<String>) -> Self {
        Self {
            seed,
            algorithm: algorithm.into(),
            digits: 6,
            period: None,
        }
    }

    /// Sets the number of digits.
    #[must_use]
    pub fn with_digits(mut self, digits: u8) -> Self {
        self.digits = digits;
        self
    }

    /// Sets the TOTP period in seconds.
    #[must_use]
    pub fn with_period(mut self, period: u32) -> Self {
        self.period = Some(period);
        self
    }

    /// Returns the seed secret.
    pub fn seed(&self) -> &SecretString {
        &self.seed
    }

    /// Returns the algorithm.
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    /// Returns the number of digits.
    pub fn digits(&self) -> u8 {
        self.digits
    }

    /// Returns the TOTP period in seconds, if applicable.
    pub fn period(&self) -> Option<u32> {
        self.period
    }
}

impl AuthScheme for OtpSeed {
    fn pattern() -> AuthPattern {
        AuthPattern::OneTimePasscode
    }
}

impl std::fmt::Debug for OtpSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtpSeed")
            .field("seed", &"[REDACTED]")
            .field("algorithm", &self.algorithm)
            .field("digits", &self.digits)
            .field("period", &self.period)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_one_time_passcode() {
        assert_eq!(OtpSeed::pattern(), AuthPattern::OneTimePasscode);
    }

    #[test]
    fn totp_defaults() {
        let seed = OtpSeed::totp(SecretString::new("JBSWY3DPEHPK3PXP"), "totp-sha1");
        assert_eq!(seed.digits(), 6);
        assert_eq!(seed.period(), Some(30));
    }
}
```

- [ ] **Step 6: Create ConnectionUri**

Create `crates/credential/src/scheme/connection_uri.rs`:

```rust
//! Compound connection URI (postgres://..., redis://..., mongodb://...).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Compound connection URI containing embedded credentials.
///
/// Covers: database connection strings, Redis URLs, message broker URIs.
/// The URI is treated as an opaque secret — parsing is the consumer's concern.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionUri {
    #[serde(with = "nebula_core::serde_secret")]
    uri: SecretString,
}

impl ConnectionUri {
    /// Creates a new connection URI.
    #[must_use]
    pub fn new(uri: SecretString) -> Self {
        Self { uri }
    }

    /// Returns the URI.
    pub fn uri(&self) -> &SecretString {
        &self.uri
    }
}

impl AuthScheme for ConnectionUri {
    fn pattern() -> AuthPattern {
        AuthPattern::ConnectionUri
    }
}

impl std::fmt::Debug for ConnectionUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionUri")
            .field("uri", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_connection_uri() {
        assert_eq!(ConnectionUri::pattern(), AuthPattern::ConnectionUri);
    }
}
```

- [ ] **Step 7: Create InstanceBinding**

Create `crates/credential/src/scheme/instance_binding.rs`:

```rust
//! Cloud/infrastructure instance identity (IMDS, managed identity).

use nebula_core::{AuthPattern, AuthScheme};
use serde::{Deserialize, Serialize};

/// Cloud instance identity binding.
///
/// Covers: AWS IMDS, GCP metadata server, Azure managed identity.
/// No secret material — identity is inferred from the runtime environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceBinding {
    provider: String,
    role_or_account: String,
    region: Option<String>,
}

impl InstanceBinding {
    /// Creates a new instance binding.
    #[must_use]
    pub fn new(provider: impl Into<String>, role_or_account: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            role_or_account: role_or_account.into(),
            region: None,
        }
    }

    /// Sets the region.
    #[must_use]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Returns the cloud provider.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Returns the role or account identifier.
    pub fn role_or_account(&self) -> &str {
        &self.role_or_account
    }

    /// Returns the region, if set.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }
}

impl AuthScheme for InstanceBinding {
    fn pattern() -> AuthPattern {
        AuthPattern::InstanceIdentity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_instance_identity() {
        assert_eq!(InstanceBinding::pattern(), AuthPattern::InstanceIdentity);
    }
}
```

- [ ] **Step 8: Create SharedKey**

Create `crates/credential/src/scheme/shared_key.rs`:

```rust
//! Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Pre-shared symmetric key.
///
/// Covers: TLS-PSK, WireGuard keys, IoT device secrets.
#[derive(Clone, Serialize, Deserialize)]
pub struct SharedKey {
    #[serde(with = "nebula_core::serde_secret")]
    key: SecretString,
    identity: Option<String>,
}

impl SharedKey {
    /// Creates a new shared key.
    #[must_use]
    pub fn new(key: SecretString) -> Self {
        Self {
            key,
            identity: None,
        }
    }

    /// Sets the PSK identity hint.
    #[must_use]
    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    /// Returns the key.
    pub fn key(&self) -> &SecretString {
        &self.key
    }

    /// Returns the identity hint, if set.
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }
}

impl AuthScheme for SharedKey {
    fn pattern() -> AuthPattern {
        AuthPattern::SharedSecret
    }
}

impl std::fmt::Debug for SharedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedKey")
            .field("key", &"[REDACTED]")
            .field("identity", &self.identity)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_shared_secret() {
        assert_eq!(SharedKey::pattern(), AuthPattern::SharedSecret);
    }
}
```

- [ ] **Step 9: Commit all new scheme types**

```bash
git add crates/credential/src/scheme/key_pair.rs \
        crates/credential/src/scheme/signing_key.rs \
        crates/credential/src/scheme/federated_assertion.rs \
        crates/credential/src/scheme/challenge_secret.rs \
        crates/credential/src/scheme/otp_seed.rs \
        crates/credential/src/scheme/connection_uri.rs \
        crates/credential/src/scheme/instance_binding.rs \
        crates/credential/src/scheme/shared_key.rs
git commit -m "feat(credential): add 8 universal scheme types

KeyPair, SigningKey, FederatedAssertion, ChallengeSecret,
OtpSeed, ConnectionUri, InstanceBinding, SharedKey.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 6: Update Certificate and OAuth2Token schemes

**Files:**
- Modify: `crates/credential/src/scheme/certificate.rs`
- Modify: `crates/credential/src/scheme/oauth2.rs`

- [ ] **Step 1: Update CertificateAuth → Certificate**

Rename `CertificateAuth` to `Certificate` in `crates/credential/src/scheme/certificate.rs`. Update field names per spec (`cert_pem` → `cert_chain`, add passphrase). Update `AuthScheme` impl to use `fn pattern()`:

```rust
//! X.509 certificate + private key (mTLS, TLS client auth).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// X.509 certificate + private key for mutual TLS.
///
/// Covers: mTLS client auth, TLS client certificates, service mesh identity.
#[derive(Clone, Serialize, Deserialize)]
pub struct Certificate {
    /// PEM-encoded certificate chain.
    cert_chain: String,
    #[serde(with = "nebula_core::serde_secret")]
    private_key: SecretString,
    #[serde(with = "nebula_core::serde_secret_option")]
    passphrase: Option<SecretString>,
}

impl Certificate {
    /// Creates a new certificate with chain and private key.
    #[must_use]
    pub fn new(cert_chain: impl Into<String>, private_key: SecretString) -> Self {
        Self {
            cert_chain: cert_chain.into(),
            private_key,
            passphrase: None,
        }
    }

    /// Sets the private key passphrase.
    #[must_use]
    pub fn with_passphrase(mut self, passphrase: SecretString) -> Self {
        self.passphrase = Some(passphrase);
        self
    }

    /// Returns the PEM-encoded certificate chain.
    pub fn cert_chain(&self) -> &str {
        &self.cert_chain
    }

    /// Returns the private key.
    pub fn private_key(&self) -> &SecretString {
        &self.private_key
    }

    /// Returns the passphrase, if set.
    pub fn passphrase(&self) -> Option<&SecretString> {
        self.passphrase.as_ref()
    }
}

impl AuthScheme for Certificate {
    fn pattern() -> AuthPattern {
        AuthPattern::Certificate
    }
}

impl std::fmt::Debug for Certificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Certificate")
            .field("cert_chain", &format!("[{} bytes]", self.cert_chain.len()))
            .field("private_key", &"[REDACTED]")
            .field("passphrase", &self.passphrase.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_certificate() {
        assert_eq!(Certificate::pattern(), AuthPattern::Certificate);
    }

    #[test]
    fn debug_redacts_secrets() {
        let cert = Certificate::new("-----BEGIN CERT-----", SecretString::new("-----BEGIN KEY-----"))
            .with_passphrase(SecretString::new("pass"));
        let debug = format!("{cert:?}");
        assert!(!debug.contains("-----BEGIN KEY-----"));
        assert!(!debug.contains("pass"));
    }
}
```

- [ ] **Step 2: Update OAuth2Token**

Update `crates/credential/src/scheme/oauth2.rs` to use `fn pattern()` instead of `const KIND`:

Replace:
```rust
impl AuthScheme for OAuth2Token {
    const KIND: &'static str = "oauth2";

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }
}
```

With:
```rust
impl AuthScheme for OAuth2Token {
    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.expires_at
    }
}
```

Add `use nebula_core::AuthPattern;` to the imports.

- [ ] **Step 3: Commit**

```bash
git add crates/credential/src/scheme/certificate.rs crates/credential/src/scheme/oauth2.rs
git commit -m "refactor(credential)!: rename CertificateAuth to Certificate, update OAuth2Token

BREAKING: CertificateAuth renamed to Certificate with updated fields.
Both schemes now use fn pattern() instead of const KIND.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 7: Delete old scheme types and wire up module

**Files:**
- Delete: `crates/credential/src/scheme/bearer.rs`
- Delete: `crates/credential/src/scheme/basic.rs`
- Delete: `crates/credential/src/scheme/api_key.rs`
- Delete: `crates/credential/src/scheme/header.rs`
- Delete: `crates/credential/src/scheme/hmac.rs`
- Delete: `crates/credential/src/scheme/saml.rs`
- Delete: `crates/credential/src/scheme/kerberos.rs`
- Delete: `crates/credential/src/scheme/database.rs`
- Delete: `crates/credential/src/scheme/aws.rs`
- Delete: `crates/credential/src/scheme/ssh.rs`
- Delete: `crates/credential/src/scheme/ldap.rs`
- Modify: `crates/credential/src/scheme/coercion.rs`
- Modify: `crates/credential/src/scheme/mod.rs`

- [ ] **Step 1: Delete old scheme files**

```bash
rm crates/credential/src/scheme/bearer.rs \
   crates/credential/src/scheme/basic.rs \
   crates/credential/src/scheme/api_key.rs \
   crates/credential/src/scheme/header.rs \
   crates/credential/src/scheme/hmac.rs \
   crates/credential/src/scheme/saml.rs \
   crates/credential/src/scheme/kerberos.rs \
   crates/credential/src/scheme/database.rs \
   crates/credential/src/scheme/aws.rs \
   crates/credential/src/scheme/ssh.rs \
   crates/credential/src/scheme/ldap.rs
```

- [ ] **Step 2: Rewrite scheme/mod.rs**

Replace `crates/credential/src/scheme/mod.rs` with:

```rust
//! Universal authentication scheme types.
//!
//! 12 built-in types cover common auth patterns. Plugins add protocol-specific
//! types via the open [`AuthScheme`](nebula_core::AuthScheme) trait.

mod certificate;
mod challenge_secret;
mod coercion;
mod connection_uri;
mod federated_assertion;
mod identity_password;
mod instance_binding;
mod key_pair;
mod oauth2;
mod otp_seed;
mod secret_token;
mod shared_key;
mod signing_key;

pub use certificate::Certificate;
pub use challenge_secret::ChallengeSecret;
pub use connection_uri::ConnectionUri;
pub use federated_assertion::FederatedAssertion;
pub use identity_password::IdentityPassword;
pub use instance_binding::InstanceBinding;
pub use key_pair::KeyPair;
pub use oauth2::OAuth2Token;
pub use otp_seed::OtpSeed;
pub use secret_token::SecretToken;
pub use shared_key::SharedKey;
pub use signing_key::SigningKey;
```

- [ ] **Step 3: Rewrite coercion.rs for new types**

Replace `crates/credential/src/scheme/coercion.rs` with:

```rust
//! Scheme coercion -- [`From`]/[`TryFrom`] conversions between scheme types.
//!
//! # Supported conversions
//!
//! | From | To | Condition |
//! |------|----|-----------|
//! | [`OAuth2Token`] | [`SecretToken`] | Always (extracts access_token) |

use super::{OAuth2Token, SecretToken};

impl From<OAuth2Token> for SecretToken {
    fn from(oauth: OAuth2Token) -> Self {
        SecretToken::new(oauth.access_token().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::SecretString;

    #[test]
    fn oauth2_to_secret_token() {
        let oauth = OAuth2Token::new(SecretString::new("access-token-123"));
        let token: SecretToken = oauth.into();
        token.token().expose_secret(|v| assert_eq!(v, "access-token-123"));
    }
}
```

- [ ] **Step 4: Commit**

```bash
git add -A crates/credential/src/scheme/
git commit -m "refactor(credential)!: replace 14 protocol-specific schemes with 12 universal types

BREAKING: Removed BearerToken, BasicAuth, ApiKeyAuth, HeaderAuth,
DatabaseAuth, HmacSecret, SamlAuth, KerberosAuth, AwsAuth, SshAuth, LdapAuth.
Replaced with: SecretToken, IdentityPassword, Certificate, KeyPair,
SigningKey, FederatedAssertion, ChallengeSecret, OtpSeed, ConnectionUri,
InstanceBinding, SharedKey. OAuth2Token retained with updated trait impl.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 8: Update built-in credentials and all references

**Files:**
- Modify: `crates/credential/src/credentials/api_key.rs`
- Modify: `crates/credential/src/credentials/basic_auth.rs`
- Delete: `crates/credential/src/credentials/database.rs`
- Delete: `crates/credential/src/credentials/header_auth.rs`
- Modify: `crates/credential/src/credentials/mod.rs`
- Modify: `crates/credential/src/credentials/oauth2.rs`
- Modify: `crates/credential/src/credentials/oauth2_flow.rs`
- Modify: `crates/credential/src/lib.rs`
- Modify: `crates/credential/src/static_protocol.rs`
- Modify: `crates/credential/src/snapshot.rs`
- Modify: `crates/credential/src/registry.rs`
- Modify: `crates/credential/src/resolver.rs`
- Modify: `crates/credential/src/handle.rs`
- Modify: `crates/credential/src/context.rs`
- Modify: `crates/credential/src/description.rs`
- Modify: `crates/credential/tests/units/scheme_roundtrip_tests.rs`
- Modify: `crates/sdk/src/prelude.rs`
- Modify: `crates/resource/src/resource.rs`
- Modify: `crates/action/src/context.rs`

This is the biggest task — updating all references from old scheme names to new ones. The changes are mechanical but widespread.

- [ ] **Step 1: Update credentials/mod.rs**

Replace `crates/credential/src/credentials/mod.rs` with:

```rust
//! Built-in credential type implementations.
//!
//! Each type implements [`Credential`](crate::credential::Credential) using
//! the v2 unified trait. Static credentials (API key, basic auth) use
//! [`identity_state!`](crate::identity_state) so that `State = Scheme`.

pub mod api_key;
pub mod basic_auth;
pub mod oauth2;
/// OAuth2 provider configuration (grant type, auth style, endpoints).
pub mod oauth2_config;
pub mod oauth2_flow;

pub use api_key::ApiKeyCredential;
pub use basic_auth::BasicAuthCredential;
pub use oauth2::{OAuth2Credential, OAuth2Pending, OAuth2State};

// ── identity_state! invocations ─────────────────────────────────────────
//
// For static credentials, State = Scheme. These macro calls implement
// `CredentialState` for each scheme type so they can be stored directly.

use crate::identity_state;
use crate::scheme::{IdentityPassword, SecretToken};

identity_state!(SecretToken, "secret_token", 1);
identity_state!(IdentityPassword, "identity_password", 1);
```

- [ ] **Step 2: Update api_key.rs to use SecretToken**

In `crates/credential/src/credentials/api_key.rs`, replace all references to `BearerToken` with `SecretToken`. Update `StaticProtocol::build()` to construct `SecretToken::new(...)` instead of `BearerToken::new(...)`. Update `use crate::scheme::BearerToken` → `use crate::scheme::SecretToken`. Update the `Credential` impl's associated types: `type Scheme = SecretToken; type State = SecretToken;`.

- [ ] **Step 3: Update basic_auth.rs to use IdentityPassword**

In `crates/credential/src/credentials/basic_auth.rs`, replace all references to `BasicAuth` with `IdentityPassword`. Update `StaticProtocol::build()` to construct `IdentityPassword::new(username, password)`. Update associated types: `type Scheme = IdentityPassword; type State = IdentityPassword;`.

- [ ] **Step 4: Delete database.rs and header_auth.rs**

```bash
rm crates/credential/src/credentials/database.rs \
   crates/credential/src/credentials/header_auth.rs
```

- [ ] **Step 5: Update OAuth2 credential**

In `crates/credential/src/credentials/oauth2.rs`, update any references to scheme types. The `OAuth2Token` type name is unchanged, so minimal changes needed — just ensure `AuthScheme` usage is updated if `KIND` was referenced.

- [ ] **Step 6: Update static_protocol.rs examples**

In `crates/credential/src/static_protocol.rs`, replace all doc-comment references from `BearerToken` → `SecretToken`, `DatabaseAuth` → remove example.

- [ ] **Step 7: Update lib.rs exports**

In `crates/credential/src/lib.rs`, update all re-exports:
- Remove: `BearerToken`, `BasicAuth`, `DatabaseAuth`, `HeaderAuth`, `ApiKeyAuth`, `HmacSecret`, `SamlAuth`, `KerberosAuth`, `AwsAuth`, `SshAuth`, `LdapAuth`, `SslMode`, `SshAuthMethod`, `LdapBindMethod`, `LdapTlsMode`
- Remove: `DatabaseCredential`, `HeaderAuthCredential`
- Add: `SecretToken`, `IdentityPassword`, `Certificate`, `KeyPair`, `SigningKey`, `FederatedAssertion`, `ChallengeSecret`, `OtpSeed`, `ConnectionUri`, `InstanceBinding`, `SharedKey`

- [ ] **Step 8: Update description.rs — add pattern field**

In `crates/credential/src/description.rs`, add `pattern: AuthPattern` field to `CredentialDescription` and the builder:

```rust
use nebula_core::AuthPattern;

pub struct CredentialDescription {
    pub key: String,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub icon_url: Option<String>,
    pub documentation_url: Option<String>,
    pub properties: ParameterCollection,
    pub pattern: AuthPattern,
}
```

Update the builder to include `.pattern()` method and require it in `build()`.

- [ ] **Step 9: Update sdk/prelude.rs**

In `crates/sdk/src/prelude.rs`, replace old scheme type re-exports with new ones.

- [ ] **Step 10: Update remaining references**

Search all `.rs` files in the workspace for any remaining references to old type names. Fix them:
- `crates/credential/src/snapshot.rs` — update any scheme type references
- `crates/credential/src/registry.rs` — update any scheme type references
- `crates/credential/src/resolver.rs` — update any scheme type references
- `crates/credential/src/handle.rs` — update any scheme type references
- `crates/credential/src/context.rs` — update any scheme type references
- `crates/resource/src/resource.rs` — update if `AuthScheme::KIND` was used
- `crates/action/src/context.rs` — update if `AuthScheme::KIND` was used
- `crates/credential/tests/units/scheme_roundtrip_tests.rs` — rewrite for new types

- [ ] **Step 11: Run full workspace check**

Run: `cargo check --workspace`
Expected: PASS — all crates compile with new types.

- [ ] **Step 12: Run tests**

Run: `cargo nextest run --workspace`
Expected: PASS

- [ ] **Step 13: Commit**

```bash
git add -A
git commit -m "refactor(credential)!: update all references to universal scheme types

Migrated built-in credentials from BearerToken/BasicAuth to
SecretToken/IdentityPassword. Removed DatabaseCredential and
HeaderAuthCredential (plugin concerns). Updated all cross-crate refs.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Phase 3: Security Hardening

### Task 9: Add key_id to EncryptedData + multi-key EncryptionLayer

**Files:**
- Modify: `crates/credential/src/crypto.rs`
- Modify: `crates/credential/src/layer/encryption.rs`

- [ ] **Step 1: Write test for key_id in EncryptedData**

Add to `crates/credential/src/crypto.rs` tests:

```rust
#[test]
fn encrypted_data_stores_key_id() {
    let key = EncryptionKey::from_bytes([0x42; 32]);
    let encrypted = encrypt_with_key_id(&key, "key-v1", b"hello", b"aad").unwrap();
    assert_eq!(encrypted.key_id, "key-v1");
}
```

- [ ] **Step 2: Add key_id to EncryptedData**

In `crates/credential/src/crypto.rs`, add `key_id: String` to `EncryptedData`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    pub version: u8,
    /// Which encryption key was used (for key rotation).
    pub key_id: String,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
    pub tag: [u8; 16],
}
```

Update `EncryptedData::new()` to take `key_id`:

```rust
pub fn new(key_id: impl Into<String>, nonce: [u8; 12], ciphertext: Vec<u8>, tag: [u8; 16]) -> Self {
    Self {
        version: Self::CURRENT_VERSION,
        key_id: key_id.into(),
        nonce,
        ciphertext,
        tag,
    }
}
```

Add a new `encrypt_with_key_id` function that takes key_id as parameter:

```rust
pub fn encrypt_with_key_id(
    key: &EncryptionKey,
    key_id: &str,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<EncryptedData, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let nonce = nonce_generator().next();
    let payload = Payload { msg: plaintext, aad };
    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

    let (ct, tag_slice) = ciphertext.split_at(ciphertext.len() - 16);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(tag_slice);

    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok(EncryptedData::new(key_id, nonce_bytes, ct.to_vec(), tag))
}
```

Update existing `encrypt` and `encrypt_with_aad` to use empty string key_id for backward compat during migration.

- [ ] **Step 3: Write test for multi-key EncryptionLayer**

Add to `crates/credential/src/layer/encryption.rs` tests:

```rust
#[tokio::test]
async fn key_rotation_re_encrypts_on_read() {
    let key_v1 = Arc::new(EncryptionKey::from_bytes([0x01; 32]));
    let key_v2 = Arc::new(EncryptionKey::from_bytes([0x02; 32]));
    let inner = InMemoryStore::new();

    // Write with key_v1
    let store_v1 = EncryptionLayer::with_keys(
        inner.clone(),
        "v1",
        vec![("v1".into(), key_v1.clone())],
    );
    let cred = make_credential("rot-1", b"secret-data");
    store_v1.put(cred, PutMode::CreateOnly).await.unwrap();

    // Read with key_v2 as current, key_v1 still available for decrypt
    let store_v2 = EncryptionLayer::with_keys(
        inner.clone(),
        "v2",
        vec![("v1".into(), key_v1), ("v2".into(), key_v2)],
    );
    let fetched = store_v2.get("rot-1").await.unwrap();
    assert_eq!(fetched.data, b"secret-data");
}
```

- [ ] **Step 4: Implement multi-key EncryptionLayer**

Rewrite `crates/credential/src/layer/encryption.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use crate::crypto::{self, EncryptionKey};
use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Wraps a store with AES-256-GCM encryption, supporting key rotation.
///
/// Maintains multiple decryption keys but encrypts new data with `current_key_id`.
/// On read, if data was encrypted with an older key, it is transparently
/// decrypted and re-encrypted with the current key (lazy rotation).
pub struct EncryptionLayer<S> {
    inner: S,
    current_key_id: String,
    keys: HashMap<String, Arc<EncryptionKey>>,
}

impl<S> EncryptionLayer<S> {
    /// Create with a single key (no rotation).
    pub fn new(inner: S, key: Arc<EncryptionKey>) -> Self {
        let mut keys = HashMap::new();
        keys.insert("default".into(), key);
        Self {
            inner,
            current_key_id: "default".into(),
            keys,
        }
    }

    /// Create with multiple keys for rotation.
    ///
    /// `current_key_id` is used for new encryptions.
    /// All keys in the list are available for decryption.
    pub fn with_keys(
        inner: S,
        current_key_id: impl Into<String>,
        keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        Self {
            inner,
            current_key_id: current_key_id.into(),
            keys: keys.into_iter().collect(),
        }
    }

    fn current_key(&self) -> Result<&EncryptionKey, StoreError> {
        self.keys
            .get(&self.current_key_id)
            .map(|k| k.as_ref())
            .ok_or_else(|| {
                StoreError::Backend(
                    format!("current encryption key '{}' not found", self.current_key_id).into(),
                )
            })
    }

    fn key_for_id(&self, key_id: &str) -> Result<&EncryptionKey, StoreError> {
        self.keys
            .get(key_id)
            .map(|k| k.as_ref())
            .ok_or_else(|| {
                StoreError::Backend(
                    format!("encryption key '{key_id}' not found").into(),
                )
            })
    }
}
```

The `CredentialStore` impl decrypts using the `key_id` from `EncryptedData`, and if it differs from `current_key_id`, re-encrypts with the current key and writes back (lazy rotation).

- [ ] **Step 5: Run tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/credential/src/crypto.rs crates/credential/src/layer/encryption.rs
git commit -m "feat(credential): encryption key rotation support

EncryptedData gains key_id field. EncryptionLayer supports multiple
keys with lazy re-encryption on read. Enables zero-downtime key rotation.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 10: Remove AAD legacy fallback

**Files:**
- Modify: `crates/credential/src/layer/encryption.rs`

- [ ] **Step 1: Write test that AAD-less data is rejected**

```rust
#[tokio::test]
async fn rejects_data_without_aad() {
    let inner = InMemoryStore::new();
    let key = test_key();

    // Simulate legacy write: encrypt without AAD
    let plaintext = b"legacy-secret";
    let encrypted = crate::crypto::encrypt(&key, plaintext).unwrap();
    let encrypted_bytes = serde_json::to_vec(&encrypted).unwrap();

    let cred = StoredCredential {
        id: "legacy-1".into(),
        credential_key: "test_credential".into(),
        data: encrypted_bytes,
        state_kind: "test".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: None,
        metadata: Default::default(),
    };
    inner.put(cred, PutMode::CreateOnly).await.unwrap();

    let store = EncryptionLayer::new(inner, key);
    let result = store.get("legacy-1").await;
    assert!(result.is_err(), "must reject data without AAD binding");
}
```

- [ ] **Step 2: Remove fallback in decrypt_data**

In the `decrypt_data` function, remove the legacy no-AAD fallback. Only attempt AAD-based decryption:

```rust
fn decrypt_data(key: &EncryptionKey, ciphertext: &[u8], id: &str) -> Result<Vec<u8>, StoreError> {
    let encrypted: crypto::EncryptedData =
        serde_json::from_slice(ciphertext).map_err(|e| StoreError::Backend(Box::new(e)))?;

    // AAD binding is mandatory — no legacy fallback
    crypto::decrypt_with_aad(key, &encrypted, id.as_bytes())
        .map_err(|e| StoreError::Backend(Box::new(e)))
}
```

- [ ] **Step 3: Remove the old `legacy_data_without_aad_still_readable` test**

Delete that test — it now contradicts the intended behavior.

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p nebula-credential`
Expected: PASS (old fallback test removed, new rejection test passes)

- [ ] **Step 5: Commit**

```bash
git add crates/credential/src/layer/encryption.rs
git commit -m "fix(credential)!: remove AAD legacy fallback

BREAKING: Data encrypted without AAD is no longer readable.
A one-time migration must re-encrypt all records with AAD before
upgrading. This closes a security gap where an attacker could
strip AAD to force fallback to unauthenticated decryption.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 11: Use Zeroizing<Vec<u8>> for plaintext buffers

**Files:**
- Modify: `crates/credential/src/layer/encryption.rs`
- Modify: `crates/credential/src/crypto.rs`

- [ ] **Step 1: Update decrypt functions to return Zeroizing<Vec<u8>>**

In `crates/credential/src/crypto.rs`, change return types:

```rust
use zeroize::Zeroizing;

pub fn decrypt(key: &EncryptionKey, encrypted: &EncryptedData) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    // ... existing logic ...
    Ok(Zeroizing::new(plaintext))
}

pub fn decrypt_with_aad(
    key: &EncryptionKey,
    encrypted: &EncryptedData,
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    // ... existing logic ...
    Ok(Zeroizing::new(plaintext))
}
```

- [ ] **Step 2: Update EncryptionLayer to use Zeroizing for intermediate buffers**

In `encrypt_data`, wrap the plaintext serialization in `Zeroizing`:

```rust
fn encrypt_data(key: &EncryptionKey, key_id: &str, plaintext: &[u8], id: &str) -> Result<Vec<u8>, StoreError> {
    // plaintext is borrowed, so the caller manages its lifetime
    let encrypted = crypto::encrypt_with_key_id(key, key_id, plaintext, id.as_bytes())
        .map_err(|e| StoreError::Backend(Box::new(e)))?;
    serde_json::to_vec(&encrypted).map_err(|e| StoreError::Backend(Box::new(e)))
}
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p nebula-credential`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/credential/src/crypto.rs crates/credential/src/layer/encryption.rs
git commit -m "fix(credential): use Zeroizing<Vec<u8>> for plaintext buffers

Decrypt functions now return Zeroizing<Vec<u8>> to ensure plaintext
is wiped from memory on drop. No manual .zeroize() calls needed.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Phase 4: AuthScheme Derive Macro

### Task 12: Create #[derive(AuthScheme)] macro

**Files:**
- Create: `crates/credential/macros/src/auth_scheme.rs`
- Modify: `crates/credential/macros/src/lib.rs`

- [ ] **Step 1: Write the macro entry point**

In `crates/credential/macros/src/lib.rs`, add:

```rust
mod auth_scheme;

/// Derive `AuthScheme` + `CredentialState` for a scheme type.
///
/// Generates both trait implementations, eliminating the need for
/// `identity_state!` macro calls.
///
/// # Attributes
///
/// - `pattern` (required): The [`AuthPattern`] variant.
/// - `kind` (optional): String identifier for `CredentialState::KIND`.
///   Defaults to snake_case of the type name.
/// - `version` (optional): Schema version. Defaults to `1`.
///
/// # Examples
///
/// ```ignore
/// #[derive(AuthScheme)]
/// #[auth_scheme(pattern = SecretToken)]
/// pub struct SecretToken {
///     token: SecretString,
/// }
/// // Generates:
/// // impl AuthScheme for SecretToken { fn pattern() -> AuthPattern { AuthPattern::SecretToken } }
/// // impl CredentialState for SecretToken { const KIND: &str = "secret_token"; const VERSION: u32 = 1; }
/// ```
#[proc_macro_derive(AuthScheme, attributes(auth_scheme))]
pub fn derive_auth_scheme(input: TokenStream) -> TokenStream {
    auth_scheme::derive(input)
}
```

- [ ] **Step 2: Write the macro implementation**

Create `crates/credential/macros/src/auth_scheme.rs`:

```rust
//! AuthScheme derive macro implementation.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

use nebula_macro_support::{attrs, diag};

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let attr_args = attrs::parse_attrs(&input.attrs, "auth_scheme")?;

    let pattern = attr_args.get_type("pattern")?.ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(AuthScheme)] requires `pattern = PatternVariant` attribute",
        )
    })?;

    // kind defaults to snake_case of type name
    let kind = attr_args.get_string("kind").unwrap_or_else(|| {
        to_snake_case(&struct_name.to_string())
    });

    let version: u32 = attr_args
        .get_string("version")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    Ok(quote! {
        impl #impl_generics ::nebula_core::AuthScheme
            for #struct_name #ty_generics #where_clause
        {
            fn pattern() -> ::nebula_core::AuthPattern {
                ::nebula_core::AuthPattern::#pattern
            }
        }

        impl #impl_generics ::nebula_credential::CredentialState
            for #struct_name #ty_generics #where_clause
        {
            const KIND: &'static str = #kind;
            const VERSION: u32 = #version;
        }
    })
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}
```

- [ ] **Step 3: Run check**

Run: `cargo check -p nebula-credential-macros`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/credential/macros/src/auth_scheme.rs crates/credential/macros/src/lib.rs
git commit -m "feat(credential): add #[derive(AuthScheme)] macro

Generates both AuthScheme and CredentialState impls from a single
derive. Replaces manual identity_state! calls with declarative attributes.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 13: Migrate scheme types to use #[derive(AuthScheme)]

**Files:**
- Modify: all scheme type files in `crates/credential/src/scheme/`
- Modify: `crates/credential/src/credentials/mod.rs` (remove identity_state! calls)

- [ ] **Step 1: Update SecretToken to use derive**

In `crates/credential/src/scheme/secret_token.rs`, replace the manual `impl AuthScheme` with:

```rust
use nebula_credential_macros::AuthScheme;

#[derive(Clone, Serialize, Deserialize, AuthScheme)]
#[auth_scheme(pattern = SecretToken)]
pub struct SecretToken {
    #[serde(with = "nebula_core::serde_secret")]
    token: SecretString,
}
```

Remove the manual `impl AuthScheme for SecretToken { ... }` block.

- [ ] **Step 2: Update all other scheme types similarly**

Apply the same pattern to: `IdentityPassword`, `KeyPair`, `SigningKey`, `FederatedAssertion`, `ChallengeSecret`, `OtpSeed`, `ConnectionUri`, `InstanceBinding`, `SharedKey`, `Certificate`, `OAuth2Token`.

For `OAuth2Token` which overrides `expires_at()`, keep the manual `AuthScheme` impl and only use the derive for `CredentialState`.

- [ ] **Step 3: Remove identity_state! calls from credentials/mod.rs**

In `crates/credential/src/credentials/mod.rs`, remove:

```rust
use crate::identity_state;
use crate::scheme::{IdentityPassword, SecretToken};

identity_state!(SecretToken, "secret_token", 1);
identity_state!(IdentityPassword, "identity_password", 1);
```

These are now handled by the derive macro on the scheme types themselves.

- [ ] **Step 4: Deprecate identity_state! macro**

In `crates/credential/src/state.rs`, add `#[deprecated]` to the macro:

```rust
/// Opt-in macro: make an `AuthScheme` also usable as `CredentialState`.
///
/// **Deprecated:** Use `#[derive(AuthScheme)]` instead, which generates
/// both `AuthScheme` and `CredentialState` impls.
#[deprecated(since = "0.4.0", note = "use #[derive(AuthScheme)] instead")]
#[macro_export]
macro_rules! identity_state { ... }
```

- [ ] **Step 5: Run tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/credential/
git commit -m "refactor(credential): migrate scheme types to #[derive(AuthScheme)]

All 12 scheme types now use the derive macro instead of manual
AuthScheme + identity_state! impls. identity_state! is deprecated.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Phase 5: Credential Derive v3

### Task 14: Rewrite #[derive(Credential)] for struct-based definitions

**Files:**
- Modify: `crates/credential/macros/src/credential.rs`

The current derive requires a unit struct + separate `protocol` type. The v3 derive works on structs with fields, using `#[param]` and `#[credential(into = "field")]` attributes.

- [ ] **Step 1: Design the new derive interface**

The new `#[derive(Credential)]` on a struct with fields:

```rust
#[derive(Credential, Parameters)]
#[credential(scheme = SecretToken)]
struct StripeAuth {
    #[param(label = "Secret Key", secret)]
    #[validate(required)]
    #[credential(into = "token")]
    api_key: String,
}
```

Generates:
- `impl Credential for StripeAuth` with `type Scheme = SecretToken`, `type State = SecretToken`, `type Pending = NoPendingState`
- `resolve()` reads fields from `ParameterValues`, constructs `SecretToken`
- `project()` clones the scheme
- `description()` returns metadata with parameters from `Parameters` derive
- `parameters()` delegates to `HasParameters`

- [ ] **Step 2: Rewrite credential.rs macro**

Rewrite `crates/credential/macros/src/credential.rs` to:
1. Accept both unit structs (legacy, delegates to `protocol` type) and field structs (v3, uses `#[credential(into)]` mapping)
2. For field structs: parse `#[credential(scheme = Type)]` and `#[credential(into = "field")]` on each field
3. Generate `resolve()` that reads each field from `ParameterValues` and constructs the scheme
4. Secret fields (those with `#[param(secret)]`) are auto-wrapped in `SecretString`

```rust
fn expand_struct_fields(
    input: &DeriveInput,
    data: &syn::DataStruct,
) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let attr_args = attrs::parse_attrs(&input.attrs, "credential")?;

    let scheme = attr_args.get_type("scheme")?.ok_or_else(|| {
        diag::error_spanned(struct_name, "requires `scheme = Type`")
    })?;

    // Parse field mappings: #[credential(into = "field_name")]
    let fields = match &data.fields {
        syn::Fields::Named(named) => &named.named,
        _ => return Err(diag::error_spanned(struct_name, "requires named fields")),
    };

    let mut field_mappings = Vec::new();
    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_attr = attrs::parse_attrs(&field.attrs, "credential")?;
        if let Some(into) = field_attr.get_string("into") {
            let is_secret = field.attrs.iter().any(|a| {
                a.path().is_ident("param")
                    && a.to_token_stream().to_string().contains("secret")
            });
            field_mappings.push((field_name.clone(), into, is_secret));
        }
    }

    // Generate resolve body that reads params and constructs scheme
    let param_reads: Vec<_> = field_mappings.iter().map(|(name, into_field, is_secret)| {
        let name_str = name.to_string();
        let into_ident = syn::Ident::new(into_field, name.span());
        if *is_secret {
            quote! {
                #into_ident: ::nebula_core::SecretString::new(
                    values.get_string(#name_str).unwrap_or_default().to_owned()
                )
            }
        } else {
            quote! {
                #into_ident: values.get_string(#name_str).unwrap_or_default().to_owned()
            }
        }
    }).collect();

    // ... generate full Credential impl ...
}
```

- [ ] **Step 3: Run check**

Run: `cargo check -p nebula-credential-macros`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/credential/macros/src/credential.rs
git commit -m "feat(credential): v3 derive macro with struct field mapping

#[derive(Credential)] now works on structs with #[credential(into)]
field mapping. Generates resolve(), project(), description() from
struct fields + #[param] attributes.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 15: Migrate built-in credentials to v3 derive

**Files:**
- Modify: `crates/credential/src/credentials/api_key.rs`
- Modify: `crates/credential/src/credentials/basic_auth.rs`

- [ ] **Step 1: Rewrite ApiKeyCredential**

Replace the entire `api_key.rs` with:

```rust
//! API key credential (static, non-interactive).

use nebula_credential_macros::Credential;
use nebula_parameter_macros::Parameters;

/// API key authentication — maps a secret key to [`SecretToken`].
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::credentials::ApiKeyCredential;
/// ```
#[derive(Credential, Parameters)]
#[credential(scheme = crate::scheme::SecretToken)]
pub struct ApiKeyCredential {
    /// The API key value.
    #[param(label = "API Key", secret)]
    #[validate(required)]
    #[credential(into = "token")]
    api_key: String,
}
```

- [ ] **Step 2: Rewrite BasicAuthCredential**

Replace `basic_auth.rs` with:

```rust
//! Basic auth credential (static, non-interactive).

use nebula_credential_macros::Credential;
use nebula_parameter_macros::Parameters;

/// Username + password authentication — maps to [`IdentityPassword`].
#[derive(Credential, Parameters)]
#[credential(scheme = crate::scheme::IdentityPassword)]
pub struct BasicAuthCredential {
    /// Username or email.
    #[param(label = "Username")]
    #[validate(required)]
    #[credential(into = "identity")]
    username: String,

    /// Password.
    #[param(label = "Password", secret)]
    #[validate(required)]
    #[credential(into = "password")]
    password: String,
}
```

- [ ] **Step 3: Delete StaticProtocol implementations**

The old `StaticProtocol` implementations for these credentials are no longer needed since the derive macro handles everything. Delete the old manual `impl StaticProtocol for ApiKeyProtocol` etc.

- [ ] **Step 4: Run tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/credential/src/credentials/api_key.rs \
        crates/credential/src/credentials/basic_auth.rs
git commit -m "refactor(credential): migrate built-in credentials to v3 derive

ApiKeyCredential and BasicAuthCredential now use 5-line struct-based
derive instead of manual StaticProtocol implementations.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Phase 6: API Surface Updates

### Task 16: Update Credential::test() signature

**Files:**
- Modify: `crates/credential/src/credential.rs`
- Modify: `crates/credential/src/resolve.rs` (if TestResult needs changes)

The spec changes `test()` to return `Option<TestResult>` instead of `Result<TestResult, CredentialError>`. The `Untestable` variant on `TestResult` becomes `None`.

- [ ] **Step 1: Update test() signature**

In `crates/credential/src/credential.rs`, change:

```rust
fn test(
    _scheme: &Self::Scheme,
    _ctx: &CredentialContext,
) -> impl Future<Output = Result<TestResult, CredentialError>> + Send
where
    Self: Sized,
{
    async { Ok(TestResult::Untestable) }
}
```

To:

```rust
fn test(
    _scheme: &Self::Scheme,
    _ctx: &CredentialContext,
) -> impl Future<Output = Option<TestResult>> + Send
where
    Self: Sized,
{
    async { None }
}
```

- [ ] **Step 2: Remove TestResult::Untestable variant**

In `crates/credential/src/resolve.rs`, remove the `Untestable` variant from `TestResult` if it exists. `None` replaces it.

- [ ] **Step 3: Update OAuth2Credential::test() if it overrides**

If `OAuth2Credential` overrides `test()`, update its return type to `Option<TestResult>`.

- [ ] **Step 4: Run tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/credential/src/credential.rs crates/credential/src/resolve.rs
git commit -m "refactor(credential)!: test() returns Option<TestResult>

BREAKING: Credential::test() now returns Option<TestResult> instead
of Result<TestResult, CredentialError>. None means untestable (was
TestResult::Untestable). Credentials that can self-test return Some.

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

### Task 17: Final verification and context update

**Files:**
- Modify: `.claude/crates/credential.md`
- Modify: `.claude/active-work.md`

- [ ] **Step 1: Run full workspace validation**

```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc
```

Expected: PASS — zero warnings, all tests green.

- [ ] **Step 2: Update credential context file**

Update `.claude/crates/credential.md` with:
- New scheme types (12 universal instead of 14 protocol-specific)
- AuthPattern classification
- Multi-key encryption with key rotation
- AAD-only encryption (no legacy fallback)
- #[derive(AuthScheme)] replaces identity_state!
- #[derive(Credential)] v3 with field mapping
- test() returns Option<TestResult>

- [ ] **Step 3: Update active-work.md**

Move "credential v3" to "Recently Completed" with summary of changes.

- [ ] **Step 4: Commit**

```bash
git add .claude/crates/credential.md .claude/active-work.md
git commit -m "docs(credential): update context files for v3

Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>"
```

---

## Migration Checklist

Before deploying v3 in any environment with existing encrypted data:

1. **Re-encrypt all records with AAD** — run migration tool (not in scope of this plan) to re-encrypt every `StoredCredential` using `encrypt_with_aad`. After migration, AAD fallback is removed.
2. **Update all plugin credentials** — any external crates implementing `AuthScheme` must change from `const KIND` to `fn pattern()`.
3. **Update scheme type references** — `BearerToken` → `SecretToken`, `BasicAuth` → `IdentityPassword`, etc.
4. **Update `identity_state!` calls** — replace with `#[derive(AuthScheme)]` or keep deprecated macro temporarily.
