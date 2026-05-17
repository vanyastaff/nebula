//! Type-erased credential *operation* closures keyed by
//! `Credential::KEY`, parameterised by the pending store `PS`.
//!
//! [`CredentialDispatch`](crate::dispatch::CredentialDispatch) owns the
//! key→capability bookkeeping but is generic-free, so it cannot hold the
//! operation closures: `resolve` threads the `PS` pending store through
//! [`nebula_engine::credential::execute_resolve`], which is generic over
//! `PS`. This table carries those monomorphised closures.
//!
//! Mirrors the erasure shape of
//! [`nebula_engine::credential::StateProjectionRegistry`]: a runtime
//! string key selects a boxed closure that captures a concrete `C`, so
//! `Credential::resolve` / `Credential::project` run without reflection.
//! Registration is fail-closed on a duplicate `KEY`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nebula_credential::pending_store::PendingStateStore;
use nebula_credential::store::CredentialStore;
use nebula_credential::{
    Credential, CredentialContext, CredentialRecord, CredentialSnapshot, CredentialState,
};
use nebula_engine::credential::{ResolveResponse, execute_resolve};
use nebula_schema::FieldValues;
use zeroize::Zeroizing;

use crate::dispatch::DispatchError;
use crate::error::CredentialServiceError;

/// Serialized credential state produced by a `resolve` closure, ready to
/// persist via the layered store (the `EncryptionLayer` ciphers `data`).
pub(crate) struct ResolvedState {
    /// Serialized `C::State` bytes. Plaintext in-process (the store's
    /// `EncryptionLayer` ciphers it at rest); held in `Zeroizing` so this
    /// intermediate is wiped on drop per canon §12.5.
    pub(crate) data: Zeroizing<Vec<u8>>,
    /// `<C::State as CredentialState>::KIND`.
    pub(crate) state_kind: String,
    /// `<C::State as CredentialState>::VERSION`.
    pub(crate) state_version: u32,
    /// `C::State::expires_at()` at resolve time, if any.
    pub(crate) expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Boxed future returned by the erased `resolve` closure.
type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ResolvedState, CredentialServiceError>> + Send + 'a>>;

/// Erased `resolve`: runs the canonical [`execute_resolve`] for the
/// captured concrete `C`, then serializes `C::State`.
type ResolveFn<PS> = Arc<
    dyn for<'a> Fn(&'a FieldValues, &'a CredentialContext, &'a PS) -> ResolveFuture<'a>
        + Send
        + Sync,
>;

/// Erased `project`: deserializes stored state bytes into `C::State`,
/// runs `C::project`, and wraps the scheme in a secret-free
/// [`CredentialSnapshot`]. Never returns the raw secret bytes.
type SnapshotFn = Arc<
    dyn Fn(&[u8], CredentialRecord) -> Result<CredentialSnapshot, CredentialServiceError>
        + Send
        + Sync,
>;

/// Erased validation: runs the canonical credential properties pipeline
/// for the captured concrete `C` —
/// `C::properties_schema().validate(FieldValues)` then a typed
/// `serde_json::from_value::<C::Properties>` round-trip. The typed step
/// is the `{"$expr": ..}` refusal point (canon §12.5). Returns only the
/// schema `code`/`path` on failure, never raw property values.
type ValidateFn =
    Arc<dyn Fn(&serde_json::Value) -> Result<(), CredentialServiceError> + Send + Sync>;

/// One credential type's erased operation closures.
struct OpsEntry<PS> {
    validate: ValidateFn,
    resolve: ResolveFn<PS>,
    snapshot: SnapshotFn,
}

/// Key → erased operation closures. Built alongside
/// [`CredentialDispatch`](crate::dispatch::CredentialDispatch) and
/// `register_builtins` at the composition root.
///
/// `B` is the raw backend type the owning service is generic over; it
/// appears only as a marker so the table's type lines up with
/// `CredentialService<B, PS>` (the closures themselves capture `C` and
/// thread `PS`, never `B`).
pub struct DispatchOps<B: CredentialStore, PS: PendingStateStore> {
    entries: HashMap<&'static str, OpsEntry<PS>>,
    _backend: std::marker::PhantomData<fn() -> B>,
}

impl<B: CredentialStore, PS: PendingStateStore> Default for DispatchOps<B, PS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: CredentialStore, PS: PendingStateStore> std::fmt::Debug for DispatchOps<B, PS> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchOps")
            .field("registered_keys", &self.entries.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl<B: CredentialStore, PS: PendingStateStore> DispatchOps<B, PS> {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            _backend: std::marker::PhantomData,
        }
    }

    /// Number of registered types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no operation closures are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when `key` has operation closures registered.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Resolve `props` into serialized credential state for the type at
    /// `key`. Threads `pending` through the canonical executor.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` has no
    /// registered closure; any error the executor surfaces (timeout,
    /// credential error, interactive kickoff) mapped to a
    /// `CredentialServiceError`.
    pub(crate) async fn resolve(
        &self,
        key: &str,
        values: &FieldValues,
        ctx: &CredentialContext,
        pending: &PS,
    ) -> Result<ResolvedState, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        (entry.resolve)(values, ctx, pending).await
    }

    /// Project decrypted state bytes for the type at `key` into a
    /// secret-free [`CredentialSnapshot`].
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::Internal`] when stored bytes fail to
    /// deserialize into the registered state type.
    pub(crate) fn snapshot(
        &self,
        key: &str,
        data: &[u8],
        record: CredentialRecord,
    ) -> Result<CredentialSnapshot, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        (entry.snapshot)(data, record)
    }

    /// Run the canonical credential properties validation pipeline for
    /// the type at `key` against `props` (schema + typed-deserialize;
    /// `{"$expr": ..}` refused at the typed step, canon §12.5).
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::ValidationFailed`] on a schema or
    /// typed-deserialize rejection (carries only `code`/`path`, never
    /// raw values).
    pub(crate) fn validate(
        &self,
        key: &str,
        props: &serde_json::Value,
    ) -> Result<(), CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        (entry.validate)(props)
    }
}

/// Register the erased operation closures for one concrete credential
/// type `C` into `ops`. Fail-closed on a duplicate `C::KEY`.
///
/// The composition root enumerates the first-party types explicitly
/// (mirroring `nebula_credential_builtin::register_builtins`), so the
/// monomorphised `execute_resolve::<C, PS>` / `C::project` calls are
/// captured here once per type — there is no reflection at the call site.
///
/// `C::Scheme: Clone` is required by
/// [`CredentialSnapshot::new`](nebula_credential::CredentialSnapshot::new);
/// every first-party scheme satisfies it.
///
/// # Errors
///
/// [`DispatchError::DuplicateKey`] if `C::KEY` is already registered; the
/// table is left unchanged for the rejected entry.
pub fn register_runtime_ops<C, B, PS>(ops: &mut DispatchOps<B, PS>) -> Result<(), DispatchError>
where
    C: Credential,
    C::Scheme: Clone,
    C::Properties: serde::de::DeserializeOwned,
    B: CredentialStore,
    PS: PendingStateStore,
{
    let key: &'static str = C::KEY;
    if ops.entries.contains_key(key) {
        return Err(DispatchError::DuplicateKey { key });
    }

    let resolve: ResolveFn<PS> = Arc::new(
        |values: &FieldValues, ctx: &CredentialContext, pending: &PS| {
            Box::pin(async move {
                let response = execute_resolve::<C, PS>(values, ctx, pending)
                    .await
                    .map_err(|e| CredentialServiceError::ValidationFailed {
                        reason: format!("credential resolve failed: {e}"),
                    })?;
                match response {
                    ResolveResponse::Complete(state) => {
                        let data = Zeroizing::new(serde_json::to_vec(&state).map_err(|e| {
                            CredentialServiceError::Internal(format!(
                                "state serialization failed: {e}"
                            ))
                        })?);
                        Ok(ResolvedState {
                            data,
                            state_kind: <C::State as CredentialState>::KIND.to_owned(),
                            state_version: <C::State as CredentialState>::VERSION,
                            expires_at: state.expires_at(),
                        })
                    },
                    ResolveResponse::Pending { .. } | ResolveResponse::Retry { .. } => {
                        // CRUD `create` is the non-interactive path; an
                        // interactive kickoff or retry is not a stored
                        // credential. Interactive acquisition is a
                        // distinct operation.
                        Err(CredentialServiceError::ValidationFailed {
                            reason: "credential requires interactive acquisition; not creatable \
                                     via the synchronous create path"
                                .to_owned(),
                        })
                    },
                }
            }) as ResolveFuture<'_>
        },
    );

    let snapshot: SnapshotFn = Arc::new(|data: &[u8], record: CredentialRecord| {
        let state: C::State = serde_json::from_slice(data).map_err(|e| {
            CredentialServiceError::Internal(format!("stored state deserialization failed: {e}"))
        })?;
        let scheme = C::project(&state);
        Ok(CredentialSnapshot::new(C::KEY, record, scheme))
    });

    let validate: ValidateFn = Arc::new(|props: &serde_json::Value| {
        // Canonical pipeline (mirrors `properties_pipeline.rs`): schema
        // validate, then a typed `from_value` round-trip. The credential
        // pipeline never resolves expressions, so a `{"$expr": ..}`
        // envelope passes schema validation but is refused by the typed
        // deserialize below (canon §12.5 defense-in-depth #2).
        let schema = C::properties_schema();
        let values = FieldValues::from_json(props.clone()).map_err(|e| {
            CredentialServiceError::ValidationFailed {
                reason: format!("[{}] {}", e.code, e.path),
            }
        })?;
        schema
            .validate(&values)
            .map_err(|report| CredentialServiceError::ValidationFailed {
                reason: report
                    .errors()
                    .map(|e| format!("[{}] {}", e.code, e.path))
                    .collect::<Vec<_>>()
                    .join("; "),
            })?;
        serde_json::from_value::<C::Properties>(props.clone()).map_err(|_| {
            // The serde error text can echo the offending field value
            // (a secret); deliberately omitted — only the policy reason
            // is surfaced.
            CredentialServiceError::ValidationFailed {
                reason: "property payload rejected by typed schema (expression-bearing or \
                         malformed credential properties are not accepted)"
                    .to_owned(),
            }
        })?;
        Ok(())
    });

    ops.entries.insert(
        key,
        OpsEntry {
            validate,
            resolve,
            snapshot,
        },
    );
    tracing::info!(credential.key = key, "credential runtime ops registered");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DispatchOps, register_runtime_ops};
    use nebula_credential::{CredentialContext, CredentialRecord, InMemoryPendingStore};
    use nebula_credential_builtin::BearerTokenCredential;
    use nebula_schema::FieldValues;
    use nebula_storage::credential::InMemoryStore;

    type Ops = DispatchOps<InMemoryStore, InMemoryPendingStore>;

    #[test]
    fn register_and_lookup() {
        let mut ops = Ops::new();
        register_runtime_ops::<BearerTokenCredential, InMemoryStore, InMemoryPendingStore>(
            &mut ops,
        )
        .expect("register ok");
        assert!(ops.contains("bearer_token"));
        assert_eq!(ops.len(), 1);
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let mut ops = Ops::new();
        register_runtime_ops::<BearerTokenCredential, InMemoryStore, InMemoryPendingStore>(
            &mut ops,
        )
        .expect("first ok");
        let err =
            register_runtime_ops::<BearerTokenCredential, InMemoryStore, InMemoryPendingStore>(
                &mut ops,
            )
            .expect_err("second rejected");
        assert!(matches!(
            err,
            crate::dispatch::DispatchError::DuplicateKey { .. }
        ));
        assert_eq!(ops.len(), 1);
    }

    #[tokio::test]
    async fn resolve_then_snapshot_roundtrip() {
        let mut ops = Ops::new();
        register_runtime_ops::<BearerTokenCredential, InMemoryStore, InMemoryPendingStore>(
            &mut ops,
        )
        .expect("register ok");

        let mut values = FieldValues::new();
        values
            .try_set_raw("token", serde_json::Value::String("sk-roundtrip".into()))
            .expect("known-good key");
        let ctx = CredentialContext::for_test("owner");
        let pending = InMemoryPendingStore::new();

        let resolved = ops
            .resolve("bearer_token", &values, &ctx, &pending)
            .await
            .expect("resolve ok");
        assert_eq!(resolved.state_kind, "secret_token");

        let snap = ops
            .snapshot("bearer_token", &resolved.data, CredentialRecord::new())
            .expect("snapshot ok");
        assert_eq!(snap.kind(), "bearer_token");
        // Snapshot redacts secrets in Debug.
        assert!(format!("{snap:?}").contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn resolve_unknown_key_is_type_unknown() {
        let ops = Ops::new();
        let values = FieldValues::new();
        let ctx = CredentialContext::for_test("owner");
        let pending = InMemoryPendingStore::new();
        // `ResolvedState` is deliberately not `Debug` (it carries
        // plaintext secret bytes), so match the `Result` directly
        // rather than using `expect_err` (which needs `T: Debug`).
        match ops.resolve("nope", &values, &ctx, &pending).await {
            Err(crate::error::CredentialServiceError::TypeUnknown { .. }) => {},
            other => panic!("expected TypeUnknown, got {:?}", other.err()),
        }
    }
}
