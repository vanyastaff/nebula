//! Point-in-time credential snapshot with typed projection.
//!
//! [`CredentialSnapshot`] carries a type-erased [`AuthScheme`] projection
//! that consumers downcast via [`project()`](CredentialSnapshot::project) or
//! [`into_project()`](CredentialSnapshot::into_project).  This replaces the
//! previous `serde_json::Value` design, giving actions type-safe credential
//! access without manual deserialization.
//!
//! # Examples
//!
//! ```
//! use nebula_credential::{AuthPattern, AuthScheme, CredentialRecord, CredentialSnapshot};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Clone, Serialize, Deserialize)]
//! struct MyToken {
//!     value: String,
//! }
//!
//! impl AuthScheme for MyToken {
//!     fn pattern() -> AuthPattern {
//!         AuthPattern::SecretToken
//!     }
//! }
//!
//! let snapshot = CredentialSnapshot::new(
//!     "api_key",
//!     CredentialRecord::new(),
//!     MyToken {
//!         value: "secret".into(),
//!     },
//! );
//!
//! assert_eq!(snapshot.kind(), "api_key");
//! assert_eq!(snapshot.scheme_pattern(), "SecretToken");
//!
//! let token = snapshot.project::<MyToken>().unwrap();
//! assert_eq!(token.value, "secret");
//! ```
//!
//! ## Runtime construction
//!
//! The runtime constructs snapshots after resolving credentials:
//!
//! ```ignore
//! // In the runtime's CredentialAccessor implementation:
//! let handle = resolver.resolve::<ApiKeyCredential>(id).await?;
//! let scheme: Arc<SecretToken> = handle.snapshot();
//! let snapshot = CredentialSnapshot::new(
//!     ApiKeyCredential::KEY,
//!     record,
//!     (*scheme).clone(),
//! );
//! ```

use std::{any::Any, fmt};

use crate::{AuthScheme, CredentialRecord};

/// Error returned by [`CredentialSnapshot`] projection methods.
///
/// # Errors
///
/// - [`SchemeMismatch`](SnapshotError::SchemeMismatch) when the requested `AuthScheme` type does
///   not match the type stored in the snapshot.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The requested scheme type does not match the projected type.
    #[error("scheme mismatch: expected pattern `{expected}`, got `{actual}`")]
    SchemeMismatch {
        /// The pattern name of the requested `AuthScheme`.
        expected: String,
        /// The pattern name stored in the snapshot.
        actual: String,
    },
}

/// A point-in-time snapshot of a stored credential.
///
/// Returned when an action or context requests a credential by ID.
/// Contains the credential kind, the projected [`AuthScheme`] (type-erased),
/// and the associated runtime [`CredentialRecord`].
///
/// # Type safety
///
/// The projected value is stored as `Box<dyn Any + Send + Sync>`.
/// Use [`project()`](Self::project) to borrow-downcast or
/// [`into_project()`](Self::into_project) to consume-downcast.
///
/// # Serialization
///
/// `CredentialSnapshot` is intentionally **not** `Serialize`/`Deserialize`.
/// Snapshots are transient runtime objects — they exist only during action
/// execution and are never persisted or transmitted over the wire.
///
/// # Security
///
/// The [`Debug`] implementation intentionally redacts the projected value
/// because it may contain secrets (tokens, passwords, keys).
pub struct CredentialSnapshot {
    /// The credential type key (e.g. `"api_key"`, `"oauth2"`).
    kind: String,
    /// The scheme pattern name from `AuthScheme::pattern()` (e.g. `"SecretToken"`, `"OAuth2"`).
    scheme_pattern: String,
    /// Associated credential record (runtime state).
    record: CredentialRecord,
    /// Type-erased projected `AuthScheme`.
    projected: Box<dyn Any + Send + Sync>,
    /// Clone function captured at construction time from `S: AuthScheme + Clone`.
    clone_fn: fn(&(dyn Any + Send + Sync)) -> Box<dyn Any + Send + Sync>,
}

impl CredentialSnapshot {
    /// Creates a new snapshot from a credential kind, record, and projected scheme.
    ///
    /// `scheme_pattern` is derived from `S::pattern()` automatically.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::{AuthPattern, AuthScheme, CredentialRecord, CredentialSnapshot};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Bearer {
    ///     token: String,
    /// }
    /// impl AuthScheme for Bearer {
    ///     fn pattern() -> AuthPattern {
    ///         AuthPattern::SecretToken
    ///     }
    /// }
    ///
    /// let snap = CredentialSnapshot::new(
    ///     "api_key",
    ///     CredentialRecord::new(),
    ///     Bearer { token: "t".into() },
    /// );
    /// assert_eq!(snap.scheme_pattern(), "SecretToken");
    /// ```
    #[must_use]
    pub fn new<S: AuthScheme>(
        kind: impl Into<String>,
        record: CredentialRecord,
        scheme: S,
    ) -> Self {
        fn clone_projected<S: AuthScheme>(
            boxed: &(dyn Any + Send + Sync),
        ) -> Box<dyn Any + Send + Sync> {
            // This downcast cannot fail: clone_fn is monomorphized at new::<S>()
            // time with the same S that was stored in projected.
            let Some(s) = boxed.downcast_ref::<S>() else {
                unreachable!("clone_fn type parameter matches stored type")
            };
            Box::new(s.clone())
        }

        Self {
            kind: kind.into(),
            scheme_pattern: format!("{:?}", S::pattern()),
            record,
            projected: Box::new(scheme),
            clone_fn: clone_projected::<S>,
        }
    }

    /// Borrows the projected `AuthScheme` by downcasting.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::SchemeMismatch`] if `S` does not match
    /// the type originally stored.
    pub fn project<S: AuthScheme>(&self) -> Result<&S, SnapshotError> {
        self.projected
            .downcast_ref::<S>()
            .ok_or_else(|| SnapshotError::SchemeMismatch {
                expected: format!("{:?}", S::pattern()),
                actual: self.scheme_pattern.clone(),
            })
    }

    /// Returns `true` if the projected scheme is of type `S`.
    ///
    /// Useful for checking the type before calling
    /// [`into_project()`](Self::into_project), which consumes the snapshot.
    #[must_use]
    pub fn is<S: AuthScheme>(&self) -> bool {
        self.projected.downcast_ref::<S>().is_some()
    }

    /// Consumes the snapshot and returns the projected `AuthScheme`.
    ///
    /// # Note
    ///
    /// This method consumes the snapshot. If the type doesn't match, both
    /// the snapshot and the inner value are lost. Use
    /// [`project()`](Self::project) or [`is()`](Self::is) first to verify
    /// the type when uncertain.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::SchemeMismatch`] if `S` does not match
    /// the type originally stored.
    pub fn into_project<S: AuthScheme>(self) -> Result<S, SnapshotError> {
        let actual = self.scheme_pattern;
        self.projected
            .downcast::<S>()
            .map(|b| *b)
            .map_err(|_| SnapshotError::SchemeMismatch {
                expected: format!("{:?}", S::pattern()),
                actual,
            })
    }

    /// The credential type key (e.g. `"api_key"`, `"oauth2"`).
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The scheme pattern name (e.g. `"SecretToken"`, `"OAuth2"`).
    #[must_use]
    pub fn scheme_pattern(&self) -> &str {
        &self.scheme_pattern
    }

    /// Associated credential record (runtime state).
    #[must_use]
    pub fn record(&self) -> &CredentialRecord {
        &self.record
    }
}

impl Clone for CredentialSnapshot {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            scheme_pattern: self.scheme_pattern.clone(),
            record: self.record.clone(),
            projected: (self.clone_fn)(&*self.projected),
            clone_fn: self.clone_fn,
        }
    }
}

impl fmt::Debug for CredentialSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialSnapshot")
            .field("kind", &self.kind)
            .field("scheme_pattern", &self.scheme_pattern)
            .field("record", &self.record)
            .field("projected", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CredentialRecord, SecretString,
        scheme::{ConnectionUri, SecretToken},
    };

    fn token_snapshot() -> CredentialSnapshot {
        CredentialSnapshot::new(
            "api_key",
            CredentialRecord::new(),
            SecretToken::new(SecretString::new("test-token")),
        )
    }

    #[test]
    fn project_returns_correct_type() {
        let snap = token_snapshot();
        let token = snap.project::<SecretToken>();
        assert!(token.is_ok());
        assert_eq!(token.unwrap().token().expose_secret(), "test-token");
    }

    #[test]
    fn project_wrong_type_returns_error() {
        let snap = token_snapshot();
        let result = snap.project::<ConnectionUri>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        match &err {
            SnapshotError::SchemeMismatch { expected, actual } => {
                assert_eq!(expected, "ConnectionUri");
                assert_eq!(actual, "SecretToken");
            },
        }
        // Verify error message
        assert!(err.to_string().contains("ConnectionUri"));
        assert!(err.to_string().contains("SecretToken"));
    }

    #[test]
    fn into_project_consumes_and_returns() {
        let snap = token_snapshot();
        let token = snap.into_project::<SecretToken>();
        assert!(token.is_ok());
        assert_eq!(token.unwrap().token().expose_secret(), "test-token");
    }

    #[test]
    fn kind_and_record_accessors() {
        let snap = token_snapshot();
        assert_eq!(snap.kind(), "api_key");
        assert_eq!(snap.scheme_pattern(), "SecretToken");
        assert_eq!(snap.record().version, 1);
    }

    #[test]
    fn into_project_wrong_type_returns_error() {
        let snap = token_snapshot();
        let result = snap.into_project::<ConnectionUri>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        match &err {
            SnapshotError::SchemeMismatch { expected, .. } => {
                assert_eq!(expected, "ConnectionUri");
            },
        }
    }

    #[test]
    fn is_returns_true_for_matching_type() {
        let snap = token_snapshot();
        assert!(snap.is::<SecretToken>());
        assert!(!snap.is::<ConnectionUri>());
    }

    #[test]
    fn clone_preserves_projected_value() {
        let snap = token_snapshot();
        let cloned = snap;
        let token = cloned.project::<SecretToken>().unwrap();
        assert_eq!(token.token().expose_secret(), "test-token");
        assert_eq!(cloned.kind(), "api_key");
        assert_eq!(cloned.scheme_pattern(), "SecretToken");
    }

    #[test]
    fn debug_redacts_projected_value() {
        let snap = token_snapshot();
        let debug_output = format!("{snap:?}");
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("test-token"));
    }
}
