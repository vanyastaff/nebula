//! Credential references and provider traits.
//!
//! Provides type-safe references to credentials and a common provider interface
//! that decouples credential acquisition from the concrete Manager implementation.

use std::future::Future;
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use nebula_core::CredentialId;

use crate::core::{CredentialContext, CredentialError, SecretString};
use crate::traits::CredentialType;

// ─── CredentialRef<C> ────────────────────────────────────────────────────────

/// Typed, compile-time reference to a specific credential instance.
///
/// Captures BOTH which instance (`CredentialId` = UUID) and which type (`C: CredentialType`).
/// Use `erase()` when you need to store it in a collection without generics.
///
/// # Example
/// ```rust,ignore
/// let id = nebula_core::CredentialId::new();
/// let r = CredentialRef::<GithubOAuth2>::from_id(id);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRef<C: CredentialType> {
    pub id: CredentialId,
    _phantom: PhantomData<fn() -> C>,
}

impl<C: CredentialType> CredentialRef<C> {
    /// Create a reference from a credential instance ID (UUID).
    pub fn from_id(id: CredentialId) -> Self {
        Self {
            id,
            _phantom: PhantomData,
        }
    }

    /// Parse a UUID string into a credential reference.
    ///
    /// # Errors
    /// Returns `ValidationError::InvalidCredentialId` if the string is not a valid UUID.
    pub fn parse(id: &str) -> Result<Self, crate::core::ValidationError> {
        let id = CredentialId::parse(id).map_err(|e| {
            crate::core::ValidationError::InvalidCredentialId {
                id: id.to_string(),
                reason: e.to_string(),
            }
        })?;
        Ok(Self::from_id(id))
    }

    /// The protocol-level key for this credential type (from nebula-core, D-015).
    /// Stable across compilations, serializable, human-readable.
    pub fn credential_key() -> nebula_core::CredentialKey {
        C::credential_key()
    }

    /// Erase the type parameter for storage in collections / manager internals.
    pub fn erase(self) -> ErasedCredentialRef {
        ErasedCredentialRef {
            id: self.id,
            key: C::credential_key(),
        }
    }

    /// Create a type-only reference for dependency declaration (instance id TBD at runtime).
    ///
    /// Use in `ActionComponents` when declaring "I need a credential of type C".
    /// The instance id is [`CredentialId::nil()`](nebula_core::CredentialId::nil) until resolved.
    pub fn of() -> Self {
        Self::from_id(CredentialId::nil())
    }
}

// ─── ErasedCredentialRef ─────────────────────────────────────────────────────

impl<C: CredentialType> From<CredentialRef<C>> for ErasedCredentialRef {
    fn from(r: CredentialRef<C>) -> Self {
        r.erase()
    }
}

/// Type-erased credential reference — used inside `ResourceComponents` and manager internals.
///
/// Preserves both the instance id (UUID) and the protocol key (stable, serializable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasedCredentialRef {
    /// Which credential instance (UUID-backed, like ResourceId).
    pub id: CredentialId,
    /// Which protocol type ("oauth2_github", "api_key", …) — from nebula-core CredentialKey.
    pub key: nebula_core::CredentialKey,
}

// ─── CredentialProvider ──────────────────────────────────────────────────────

/// Provider trait for acquiring credentials — decouples acquisition from `CredentialManager`.
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
///
/// use nebula_credential::core::CredentialProvider;
/// use nebula_credential::prelude::*;
///
/// tokio_test::block_on(async {
///     let key = EncryptionKey::from_bytes([7u8; 32]);
///     let manager = CredentialManager::builder()
///         .storage(Arc::new(MockStorageProvider::new()))
///         .encryption_key(Arc::new(EncryptionKey::from_bytes([7u8; 32])))
///         .build();
///
///     let id = CredentialId::new();
///     let ctx = CredentialContext::new("user-123");
///     let encrypted = encrypt(&key, b"api-token").expect("encrypt");
///     manager
///         .store(&id, encrypted, CredentialMetadata::new(), &ctx)
///         .await
///         .expect("store");
///
///     let secret = manager.get(&id.to_string(), &ctx).await.expect("get");
///     let value = secret.expose_secret(ToOwned::to_owned);
///     assert_eq!(value, "api-token");
/// });
/// ```
pub trait CredentialProvider: Send + Sync {
    /// Acquire typed credential state (returns raw `SecretString` for simple cases).
    fn credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    /// Acquire by string id (type-erased fallback).
    fn get(
        &self,
        id: &str,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    /// Check if a credential exists by type.
    fn has_credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> impl Future<Output = bool> + Send {
        async move { self.credential::<C>(ctx).await.is_ok() }
    }

    /// Check if a credential exists by ID.
    fn has(&self, id: &str, ctx: &CredentialContext) -> impl Future<Output = bool> + Send {
        async move { self.get(id, ctx).await.is_ok() }
    }
}

#[cfg(test)]
mod new_tests {
    use super::*;

    use async_trait::async_trait;
    use nebula_parameter::collection::ParameterCollection;

    use crate::core::CredentialDescription;
    use crate::core::result::InitializeResult;

    struct GithubOAuth2;

    #[async_trait]
    impl CredentialType for GithubOAuth2 {
        type Input = ();
        type State = crate::protocols::ApiKeyState;

        fn description() -> CredentialDescription
        where
            Self: Sized,
        {
            CredentialDescription::builder()
                .key("oauth2_github")
                .name("GitHub OAuth2 (test)")
                .description("Test credential type for CredentialRef")
                .properties(ParameterCollection::new())
                .build()
                .unwrap()
        }

        async fn initialize(
            &self,
            _input: &Self::Input,
            _ctx: &mut crate::core::CredentialContext,
        ) -> Result<InitializeResult<Self::State>, crate::core::CredentialError> {
            unreachable!("initialize is not used in CredentialRef tests")
        }
    }

    #[test]
    fn credential_ref_captures_id_and_key() {
        let id = nebula_core::CredentialId::new();
        let r = CredentialRef::<GithubOAuth2>::from_id(id);
        assert_eq!(r.id, id);
        assert_eq!(
            CredentialRef::<GithubOAuth2>::credential_key().as_str(),
            "oauth2_github"
        );
    }

    #[test]
    fn credential_ref_parse_uuid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let r = CredentialRef::<GithubOAuth2>::parse(uuid_str).unwrap();
        assert_eq!(r.id.to_string(), uuid_str);
    }

    #[test]
    fn two_instances_same_type_are_different() {
        let id1 = nebula_core::CredentialId::new();
        let id2 = nebula_core::CredentialId::new();
        let prod = CredentialRef::<GithubOAuth2>::from_id(id1);
        let staging = CredentialRef::<GithubOAuth2>::from_id(id2);
        assert_ne!(prod.id, staging.id);
    }

    #[test]
    fn erase_preserves_id_and_key() {
        let id = nebula_core::CredentialId::new();
        let r = CredentialRef::<GithubOAuth2>::from_id(id);
        let erased: ErasedCredentialRef = r.erase();
        assert_eq!(erased.id, id);
        assert_eq!(erased.key.as_str(), "oauth2_github");
    }
}
