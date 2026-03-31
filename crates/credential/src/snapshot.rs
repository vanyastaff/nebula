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
//! use nebula_core::AuthScheme;
//! use nebula_credential::{CredentialMetadata, CredentialSnapshot};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Clone, Serialize, Deserialize)]
//! struct MyToken { value: String }
//!
//! impl AuthScheme for MyToken {
//!     const KIND: &'static str = "my_token";
//! }
//!
//! let snapshot = CredentialSnapshot::new(
//!     "api_key",
//!     CredentialMetadata::new(),
//!     MyToken { value: "secret".into() },
//! );
//!
//! assert_eq!(snapshot.kind(), "api_key");
//! assert_eq!(snapshot.scheme_kind(), "my_token");
//!
//! let token = snapshot.project::<MyToken>().unwrap();
//! assert_eq!(token.value, "secret");
//! ```

use std::any::Any;
use std::fmt;

use nebula_core::AuthScheme;

use crate::metadata::CredentialMetadata;

/// Error returned by [`CredentialSnapshot`] projection methods.
///
/// # Errors
///
/// - [`SchemeMismatch`](SnapshotError::SchemeMismatch) when the requested
///   `AuthScheme` type does not match the type stored in the snapshot.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The requested scheme type does not match the projected type.
    #[error("scheme mismatch: expected `{expected}`, got `{actual}`")]
    SchemeMismatch {
        /// The `KIND` of the requested `AuthScheme`.
        expected: &'static str,
        /// The `scheme_kind` stored in the snapshot.
        actual: String,
    },
}

/// A point-in-time snapshot of a stored credential.
///
/// Returned when an action or context requests a credential by ID.
/// Contains the credential kind, the projected [`AuthScheme`] (type-erased),
/// and associated metadata.
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
    /// The scheme kind from `AuthScheme::KIND` (e.g. `"bearer"`, `"basic"`).
    scheme_kind: String,
    /// Associated credential metadata.
    metadata: CredentialMetadata,
    /// Type-erased projected `AuthScheme`.
    projected: Box<dyn Any + Send + Sync>,
    /// Clone function captured at construction time from `S: AuthScheme + Clone`.
    clone_fn: fn(&(dyn Any + Send + Sync)) -> Box<dyn Any + Send + Sync>,
}

impl CredentialSnapshot {
    /// Creates a new snapshot from a credential kind, metadata, and projected scheme.
    ///
    /// `scheme_kind` is derived from `S::KIND` automatically.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::AuthScheme;
    /// use nebula_credential::{CredentialMetadata, CredentialSnapshot};
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Clone, Serialize, Deserialize)]
    /// struct Bearer { token: String }
    /// impl AuthScheme for Bearer { const KIND: &'static str = "bearer"; }
    ///
    /// let snap = CredentialSnapshot::new("api_key", CredentialMetadata::new(), Bearer { token: "t".into() });
    /// assert_eq!(snap.scheme_kind(), "bearer");
    /// ```
    #[must_use]
    pub fn new<S: AuthScheme>(
        kind: impl Into<String>,
        metadata: CredentialMetadata,
        scheme: S,
    ) -> Self {
        fn clone_projected<S: AuthScheme>(
            boxed: &(dyn Any + Send + Sync),
        ) -> Box<dyn Any + Send + Sync> {
            let s = boxed
                .downcast_ref::<S>()
                .expect("type invariant: stored type matches S");
            Box::new(s.clone())
        }

        Self {
            kind: kind.into(),
            scheme_kind: S::KIND.to_owned(),
            metadata,
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
                expected: S::KIND,
                actual: self.scheme_kind.clone(),
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
        self.projected
            .downcast::<S>()
            .map(|b| *b)
            .map_err(|_| SnapshotError::SchemeMismatch {
                expected: S::KIND,
                actual: self.scheme_kind,
            })
    }

    /// The credential type key (e.g. `"api_key"`, `"oauth2"`).
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The scheme kind from `AuthScheme::KIND` (e.g. `"bearer"`, `"basic"`).
    #[must_use]
    pub fn scheme_kind(&self) -> &str {
        &self.scheme_kind
    }

    /// Associated credential metadata.
    #[must_use]
    pub fn metadata(&self) -> &CredentialMetadata {
        &self.metadata
    }
}

impl Clone for CredentialSnapshot {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            scheme_kind: self.scheme_kind.clone(),
            metadata: self.metadata.clone(),
            projected: (self.clone_fn)(&*self.projected),
            clone_fn: self.clone_fn,
        }
    }
}

impl fmt::Debug for CredentialSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialSnapshot")
            .field("kind", &self.kind)
            .field("scheme_kind", &self.scheme_kind)
            .field("metadata", &self.metadata)
            .field("projected", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::CredentialMetadata;
    use crate::scheme::{BearerToken, DatabaseAuth};
    use crate::utils::SecretString;

    fn bearer_snapshot() -> CredentialSnapshot {
        CredentialSnapshot::new(
            "api_key",
            CredentialMetadata::new(),
            BearerToken::new(SecretString::new("test-token")),
        )
    }

    #[test]
    fn project_returns_correct_type() {
        let snap = bearer_snapshot();
        let token = snap.project::<BearerToken>();
        assert!(token.is_ok());
        token
            .unwrap()
            .expose()
            .expose_secret(|t| assert_eq!(t, "test-token"));
    }

    #[test]
    fn project_wrong_type_returns_error() {
        let snap = bearer_snapshot();
        let result = snap.project::<DatabaseAuth>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        match &err {
            SnapshotError::SchemeMismatch { expected, actual } => {
                assert_eq!(*expected, "database");
                assert_eq!(actual, "bearer");
            }
        }
        // Verify error message
        assert!(err.to_string().contains("database"));
        assert!(err.to_string().contains("bearer"));
    }

    #[test]
    fn into_project_consumes_and_returns() {
        let snap = bearer_snapshot();
        let token = snap.into_project::<BearerToken>();
        assert!(token.is_ok());
        token
            .unwrap()
            .expose()
            .expose_secret(|t| assert_eq!(t, "test-token"));
    }

    #[test]
    fn kind_and_metadata_accessors() {
        let snap = bearer_snapshot();
        assert_eq!(snap.kind(), "api_key");
        assert_eq!(snap.scheme_kind(), "bearer");
        assert_eq!(snap.metadata().version, 1);
    }

    #[test]
    fn into_project_wrong_type_returns_error() {
        let snap = bearer_snapshot();
        let result = snap.into_project::<DatabaseAuth>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            SnapshotError::SchemeMismatch {
                expected: "database",
                ..
            }
        ));
    }

    #[test]
    fn is_returns_true_for_matching_type() {
        let snap = bearer_snapshot();
        assert!(snap.is::<BearerToken>());
        assert!(!snap.is::<DatabaseAuth>());
    }

    #[test]
    fn clone_preserves_projected_value() {
        let snap = bearer_snapshot();
        let cloned = snap.clone();
        let token = cloned.project::<BearerToken>().unwrap();
        token
            .expose()
            .expose_secret(|t| assert_eq!(t, "test-token"));
        assert_eq!(cloned.kind(), "api_key");
        assert_eq!(cloned.scheme_kind(), "bearer");
    }

    #[test]
    fn debug_redacts_projected_value() {
        let snap = bearer_snapshot();
        let debug_output = format!("{snap:?}");
        assert!(debug_output.contains("[REDACTED]"));
        assert!(!debug_output.contains("test-token"));
    }
}
