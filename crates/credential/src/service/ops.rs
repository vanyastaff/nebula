//! Type-erased credential *operation* closures keyed by
//! `Credential::KEY`, parameterised by the pending store `PS`.
//!
//! Capability is read from the
//! [`CredentialRegistry`](crate::CredentialRegistry) bitflag
//! (ADR-0088 D3); this table holds only the operation closures, which cannot
//! live on the generic-free registry: `resolve` threads the `PS` pending store
//! through [`crate::runtime::execute_resolve`], which is generic
//! over `PS`. A runtime string key selects a boxed closure that captures a
//! concrete `C`, so `Credential::resolve` / `Credential::project` run without
//! reflection. Registration is fail-closed on a duplicate `KEY`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nebula_schema::FieldValues;
use zeroize::Zeroizing;

use crate::pending_store::PendingStateStore;
use crate::resolve::{InteractionRequest, TestResult, UserInput};
use crate::runtime::{
    ResolveResponse, dispatch_revoke, dispatch_test, execute_continue, execute_resolve,
};
use crate::{
    Capabilities, Credential, CredentialContext, CredentialState, Interactive, PendingToken,
    ReauthReason, RefreshAttempt, RefreshNotAppliedContext, Refreshable, Revocable, Testable,
    contract::{RefreshReauthPhase, RefreshReportKind},
};

use super::error::{CredentialServiceError, CredentialValidationIssue, CredentialValidationReport};

/// Registration-time failure for the operation-dispatch table
/// ([`DispatchOps`]). Relocated here when the parallel `CredentialDispatch`
/// capability-flag table was removed (ADR-0088 D3): the ops table owns its own
/// registration errors, and capability is read from the
/// [`CredentialRegistry`](crate::CredentialRegistry) bitflag.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DispatchError {
    /// Two registrations shared a `Credential::KEY`. First wins; second
    /// rejected; table unchanged.
    #[error("duplicate credential dispatch key '{key}'")]
    DuplicateKey {
        /// The colliding key.
        key: &'static str,
    },

    /// A capability registrar (`register_testable_ops` /
    /// `register_refreshable_ops` / `register_revocable_ops` /
    /// `register_interactive_ops`) ran before the base ops for `key` were
    /// registered. Capability closures attach onto an existing base entry, so
    /// the base `register_runtime_ops` must run first.
    #[error("base credential ops absent for key '{key}'; register the base ops first")]
    BaseOpsMissing {
        /// The key whose base entry was missing.
        key: &'static str,
    },
}

/// Serialized credential state produced by a `resolve` closure, ready to
/// persist via the layered store (the `EncryptionLayer` ciphers `data`).
pub(crate) struct ResolvedState {
    /// Serialized `C::State` bytes. Plaintext in-process (the store's
    /// `EncryptionLayer` ciphers it at rest); held in `Zeroizing` so this
    /// intermediate is wiped on drop per credential secrecy.
    pub(crate) data: Zeroizing<Vec<u8>>,
    /// `<C::State as CredentialState>::KIND`.
    pub(crate) state_kind: String,
    /// `<C::State as CredentialState>::VERSION`.
    pub(crate) state_version: u32,
    /// `C::State::expires_at()` at resolve time, if any.
    pub(crate) expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Outcome of an acquisition attempt before the service decides whether
/// to persist (`Complete`) or surface an interaction (`Pending`). This is
/// the raw [`ResolveResponse`] shape projected to secret-free pieces: the
/// `Complete` arm carries serialized state for the create-persist path,
/// the `Pending` arm carries only the opaque token + the UI instruction.
pub(crate) enum AcquireOutcome {
    /// Credential resolved synchronously — `state` is ready to persist
    /// through the same path `create` uses.
    Complete(ResolvedState),
    /// Interactive acquisition kicked off — the caller surfaces the
    /// token + interaction and resumes via the continue path.
    Pending {
        /// Opaque pending-store handle (stringified for transport).
        token: PendingToken,
        /// What the UI must show / do next.
        interaction: InteractionRequest,
    },
    /// Framework asked to poll again after `after`.
    Retry {
        /// Delay before the next continuation poll.
        after: std::time::Duration,
    },
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

/// Boxed future returned by the erased acquisition closures.
type AcquireFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AcquireOutcome, CredentialServiceError>> + Send + 'a>>;

/// Erased acquisition `resolve`: runs [`execute_resolve`] for the
/// captured `C` and maps the full [`ResolveResponse`] (including
/// `Pending`) into [`AcquireOutcome`] — the create path's `ResolveFn`
/// rejects `Pending`, this one surfaces it.
type AcquireFn<PS> = Arc<
    dyn for<'a> Fn(&'a FieldValues, &'a CredentialContext, &'a PS) -> AcquireFuture<'a>
        + Send
        + Sync,
>;

/// Erased interactive continuation: loads the typed pending state for
/// `C: Interactive`, drives [`execute_continue`], maps the result into
/// [`AcquireOutcome`]. Only registered for `C: Interactive`.
type ContinueFn<PS> = Arc<
    dyn for<'a> Fn(
            &'a PendingToken,
            &'a UserInput,
            &'a CredentialContext,
            &'a PS,
        ) -> AcquireFuture<'a>
        + Send
        + Sync,
>;

/// Boxed future for the erased `test` closure.
type TestFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TestResult, CredentialServiceError>> + Send + 'a>>;

/// Erased `test`: deserializes stored state, projects the scheme, and
/// invokes [`dispatch_test`] for the captured `C: Testable`.
type TestFn = Arc<dyn for<'a> Fn(&'a [u8], &'a CredentialContext) -> TestFuture<'a> + Send + Sync>;

/// Result of one erased refresh implementation.
///
/// Framework-only phases remain explicit here so the coordinator can release
/// L2 only for proof-bearing no-effect outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshCommitPhase {
    ProviderConfirmed,
    LocalOnly,
}

pub(crate) enum RefreshExecutionResult {
    /// This caller refreshed; the service CAS-persists this freshly
    /// serialized state.
    Rewrote {
        /// Freshly serialized post-refresh `C::State` bytes.
        data: Zeroizing<Vec<u8>>,
        /// `<C::State as CredentialState>::expires_at()` read off the
        /// *refreshed* state. A refresh that rotated the token typically
        /// produces a new expiry; the service must persist this, not the
        /// stale pre-refresh `expires_at` (otherwise a refreshed
        /// credential keeps its old — possibly already-elapsed — expiry).
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        /// Side-effect phase that controls durable-finalization safety.
        phase: RefreshCommitPhase,
    },
    /// The implementation requested a durable reauthentication transition.
    ReauthRequired {
        reason: ReauthReason,
        phase: RefreshReauthPhase,
    },
    /// Linear evidence proved the provider transition was not applied.
    NotApplied(Box<RefreshNotAppliedContext>),
    /// Provider dispatch may have changed state.
    OutcomeUnknown,
    /// Provider work completed, but local state serialization failed.
    PostProviderPersistence,
    /// Providerless refresh state could not be serialized.
    LocalFinalizationFailed,
    /// State decoding failed before the implementation received its attempt.
    PreparationFailed(CredentialServiceError),
}

/// Boxed future for the erased `refresh` closure. Yields a
/// [`RefreshExecutionResult`] so the service can distinguish every refresh phase.
type RefreshFuture<'a> = Pin<
    Box<dyn Future<Output = Result<RefreshExecutionResult, CredentialServiceError>> + Send + 'a>,
>;

/// Erased `refresh`: deserializes stored state, runs
/// `<C as Refreshable>::refresh`, and returns an explicit phase disposition.
/// Only registered for `C: Refreshable`.
type RefreshFn =
    Arc<dyn for<'a> Fn(&'a [u8], &'a CredentialContext) -> RefreshFuture<'a> + Send + Sync>;

/// Boxed future for the erased `revoke` closure.
type RevokeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), CredentialServiceError>> + Send + 'a>>;

/// Erased `revoke`: deserializes stored state, runs
/// `<C as Revocable>::revoke` for the captured `C: Revocable`. Only
/// registered for `C: Revocable`.
type RevokeFn =
    Arc<dyn for<'a> Fn(&'a [u8], &'a CredentialContext) -> RevokeFuture<'a> + Send + Sync>;

/// Erased validation: runs the canonical credential properties pipeline
/// for the captured concrete `C` —
/// `schema_of::<C::Properties>().validate(FieldValues)` then a typed
/// `serde_json::from_value::<C::Properties>` round-trip. The typed step
/// is the `{"$expr": ..}` refusal point (credential secrecy). Returns only the
/// schema `code`/`path` on failure, never raw property values.
type ValidateFn =
    Arc<dyn Fn(&serde_json::Value) -> Result<(), CredentialServiceError> + Send + Sync>;

/// Erased property ingest: rewrites inbound serde-wire JSON into the validator's
/// [`FieldValues`] envelope for the captured concrete `C` via
/// `schema_of::<C::Properties>().values_from_wire(props)`. For a record
/// `Properties` this is plain [`FieldValues::from_json`]; for a tagged-union
/// `Properties` (a `#[derive(Schema)]` enum) it folds serde's external/adjacent
/// wire into the internal `{mode, value}` envelope so the same downstream
/// `validate` / `resolve` path the record case takes also accepts a union. The
/// schema's `serde_tagging` is the single source of truth for that rewrite.
type IngestFn =
    Arc<dyn Fn(&serde_json::Value) -> Result<FieldValues, CredentialServiceError> + Send + Sync>;

/// One credential type's erased operation closures.
///
/// `validate` / `resolve` / `acquire` are always present (the base
/// registration). The capability closures are `Option`: a
/// `Some` is set **only** by the matching capability-bounded
/// `register_*_ops` (callable only for `C: Testable` / `Refreshable` /
/// `Revocable` / `Interactive`), so closure presence *is* the capability
/// — structurally impossible to advertise one the type lacks (mirrors
/// `plugin_capability_report`).
struct OpsEntry<PS> {
    validate: ValidateFn,
    ingest: IngestFn,
    resolve: ResolveFn<PS>,
    acquire: AcquireFn<PS>,
    test_fn: Option<TestFn>,
    refresh_fn: Option<RefreshFn>,
    revoke_fn: Option<RevokeFn>,
    continue_fn: Option<ContinueFn<PS>>,
}

/// Key → erased operation closures. Built alongside the
/// [`CredentialRegistry`](crate::CredentialRegistry) and
/// `register_builtins` at the composition root.
///
/// The closures capture the concrete credential type `C` and thread `PS`;
/// the table is otherwise backend-agnostic, so it carries no backend param.
pub struct DispatchOps<PS: PendingStateStore> {
    entries: HashMap<&'static str, OpsEntry<PS>>,
}

impl<PS: PendingStateStore> Default for DispatchOps<PS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<PS: PendingStateStore> std::fmt::Debug for DispatchOps<PS> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchOps")
            .field("registered_keys", &self.entries.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl<PS: PendingStateStore> DispatchOps<PS> {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
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

    /// The capabilities backed by a registered operation closure for `key`,
    /// derived from which optional closures are present. Covers the four
    /// ops-modeled capabilities (`REFRESHABLE` / `TESTABLE` / `REVOCABLE` /
    /// `INTERACTIVE`); `DYNAMIC` is a lease-lifecycle concern with no ops
    /// closure and is never reported here. Empty set when `key` is absent.
    ///
    /// Used by the api-layer credential builder's `build()`
    /// to gate the registry's advertised capabilities against the closures
    /// actually registered, so discovery cannot advertise a capability that
    /// would fail at first call.
    #[must_use]
    pub fn capabilities_of(&self, key: &str) -> Capabilities {
        let Some(entry) = self.entries.get(key) else {
            return Capabilities::empty();
        };
        let mut caps = Capabilities::empty();
        caps.set(Capabilities::REFRESHABLE, entry.refresh_fn.is_some());
        caps.set(Capabilities::TESTABLE, entry.test_fn.is_some());
        caps.set(Capabilities::REVOCABLE, entry.revoke_fn.is_some());
        caps.set(Capabilities::INTERACTIVE, entry.continue_fn.is_some());
        caps
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

    /// Run the canonical credential properties validation pipeline for
    /// the type at `key` against `props` (schema + typed-deserialize;
    /// `{"$expr": ..}` refused at the typed step, credential secrecy).
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

    /// Ingest inbound serde-wire `props` into the validator's [`FieldValues`]
    /// envelope for the type at `key`, applying the union wire→envelope rewrite
    /// when `C::Properties` is a tagged union (driven by the schema's
    /// `serde_tagging`). This is the per-type counterpart of
    /// [`FieldValues::from_json`] that the type-erased facade must use so a union
    /// `Properties` resolves through the same envelope `validate` accepts.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::ValidationFailed`] when the wire shape does not
    /// match the type's schema (carries only `code`/`path`, never raw values).
    pub(crate) fn ingest(
        &self,
        key: &str,
        props: &serde_json::Value,
    ) -> Result<FieldValues, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        (entry.ingest)(props)
    }

    /// Drive an acquisition for the type at `key`: same canonical
    /// executor as [`Self::resolve`] but the full [`ResolveResponse`] is
    /// surfaced, so an interactive kickoff returns
    /// [`AcquireOutcome::Pending`] instead of being rejected.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent; any
    /// executor error mapped to a [`CredentialServiceError`].
    pub(crate) async fn acquire(
        &self,
        key: &str,
        values: &FieldValues,
        ctx: &CredentialContext,
        pending: &PS,
    ) -> Result<AcquireOutcome, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        (entry.acquire)(values, ctx, pending).await
    }

    /// Continue an interactive acquisition for the type at `key`.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::CapabilityUnsupported`] when the type is
    /// not interactive (no continuation closure registered); any executor
    /// error mapped to a [`CredentialServiceError`].
    pub(crate) async fn continue_resolve(
        &self,
        key: &str,
        token: &PendingToken,
        input: &UserInput,
        ctx: &CredentialContext,
        pending: &PS,
    ) -> Result<AcquireOutcome, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        let continue_fn = entry.continue_fn.as_ref().ok_or_else(|| {
            CredentialServiceError::CapabilityUnsupported {
                capability: "interactive".to_owned(),
                key: key.to_owned(),
            }
        })?;
        continue_fn(token, input, ctx, pending).await
    }

    /// Run the provider health probe for the type at `key`.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::CapabilityUnsupported`] when the type is
    /// not testable; otherwise the probe outcome.
    pub(crate) async fn test(
        &self,
        key: &str,
        data: &[u8],
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        let test_fn = entry.test_fn.as_ref().ok_or_else(|| {
            CredentialServiceError::CapabilityUnsupported {
                capability: "test".to_owned(),
                key: key.to_owned(),
            }
        })?;
        test_fn(data, ctx).await
    }

    /// Refresh the stored state for the type at `key`. Returns a
    /// [`RefreshExecutionResult`] so the caller can apply the matching durability
    /// and L2-claim disposition.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::CapabilityUnsupported`] when the type is
    /// not refreshable; otherwise the refresh failure.
    pub(crate) async fn refresh(
        &self,
        key: &str,
        data: &[u8],
        ctx: &CredentialContext,
    ) -> Result<RefreshExecutionResult, CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        let refresh_fn = entry.refresh_fn.as_ref().ok_or_else(|| {
            CredentialServiceError::CapabilityUnsupported {
                capability: "refresh".to_owned(),
                key: key.to_owned(),
            }
        })?;
        refresh_fn(data, ctx).await
    }

    /// Revoke the credential at `key` (mutating provider-side state).
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::TypeUnknown`] when `key` is absent;
    /// [`CredentialServiceError::CapabilityUnsupported`] when the type is
    /// not revocable; otherwise the revoke failure.
    pub(crate) async fn revoke(
        &self,
        key: &str,
        data: &[u8],
        ctx: &CredentialContext,
    ) -> Result<(), CredentialServiceError> {
        let entry = self
            .entries
            .get(key)
            .ok_or_else(|| CredentialServiceError::TypeUnknown {
                key: key.to_owned(),
            })?;
        let revoke_fn = entry.revoke_fn.as_ref().ok_or_else(|| {
            CredentialServiceError::CapabilityUnsupported {
                capability: "revoke".to_owned(),
                key: key.to_owned(),
            }
        })?;
        revoke_fn(data, ctx).await
    }
}

/// Register the erased operation closures for one concrete credential
/// type `C` into `ops`. Fail-closed on a duplicate `C::KEY`.
///
/// The composition root enumerates the first-party types explicitly
/// (mirroring `nebula_credential::register_builtins`), so the
/// monomorphised `execute_resolve::<C, PS>` / `C::project` calls are
/// captured here once per type — there is no reflection at the call site.
///
/// `C::Scheme: Clone` is required by
/// [`CredentialSnapshot::new`](crate::CredentialSnapshot::new);
/// every first-party scheme satisfies it.
///
/// # Errors
///
/// [`DispatchError::DuplicateKey`] if `C::KEY` is already registered; the
/// table is left unchanged for the rejected entry.
pub fn register_runtime_ops<C, PS>(ops: &mut DispatchOps<PS>) -> Result<(), DispatchError>
where
    C: Credential,
    C::Scheme: Clone,
    C::Properties: serde::de::DeserializeOwned,
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
                    .map_err(executor_error_to_service_error)?;
                match response {
                    ResolveResponse::Complete(state) => {
                        // Cleartext serialization for the encrypted-at-rest
                        // store; the bytes never leave this `Zeroizing` buffer
                        // un-encrypted. Outside this scope secrets redact.
                        let data = Zeroizing::new(
                            crate::serde_secret::expose_for_serialization(|| {
                                serde_json::to_vec(&state)
                            })
                            .map_err(|e| {
                                CredentialServiceError::Internal(format!(
                                    "state serialization failed: {e}"
                                ))
                            })?,
                        );
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
                        Err(CredentialServiceError::validation(
                            "",
                            "credential.interactive_required",
                        ))
                    },
                }
            }) as ResolveFuture<'_>
        },
    );

    let acquire: AcquireFn<PS> = Arc::new(
        |values: &FieldValues, ctx: &CredentialContext, pending: &PS| {
            Box::pin(async move {
                let response = execute_resolve::<C, PS>(values, ctx, pending)
                    .await
                    .map_err(executor_error_to_service_error)?;
                map_resolve_response::<C>(response)
            }) as AcquireFuture<'_>
        },
    );

    let validate: ValidateFn = Arc::new(|props: &serde_json::Value| {
        // Canonical pipeline (mirrors `properties_pipeline.rs`): ingest the serde
        // wire into the validator's envelope, schema-validate, then a typed
        // round-trip. `values_from_wire` is `FieldValues::from_json` for a record
        // `Properties` and the union wire→`{mode,value}` rewrite for a tagged-union
        // `Properties` (the schema's `serde_tagging` drives it). The credential
        // pipeline never resolves expressions, so a `{"$expr": ..}` envelope passes
        // schema validation but is refused by the typed deserialize below
        // (credential secrecy defense-in-depth #2).
        let schema = nebula_schema::schema_of::<C::Properties>();
        let values = schema.values_from_wire(props.clone()).map_err(|error| {
            CredentialServiceError::validation(
                error.path.to_json_pointer(),
                error.code.into_owned(),
            )
        })?;
        let valid = schema.validate(&values).map_err(|report| {
            let mut issues = report.errors().map(|error| {
                CredentialValidationIssue::new(
                    error.path.to_json_pointer(),
                    error.code.as_ref().to_owned(),
                )
            });
            let Some(first) = issues.next() else {
                return CredentialServiceError::Internal(
                    "schema validation returned an empty error report".to_owned(),
                );
            };
            CredentialServiceError::ValidationFailed {
                report: CredentialValidationReport::from_issues(first, issues.collect()),
            }
        })?;
        // Deserialize the CANONICALIZED output — read-aliases already folded onto
        // their canonical keys — NOT the raw props. validate's key-space must equal
        // deserialize's key-space: an alias-keyed (or canonical+alias) submission
        // validates under the canonical key, so the typed round-trip must see the
        // same keys, otherwise the two passes could disagree on a field's value.
        // `to_typed` is `from_value(raw().to_json())` for a record and the
        // envelope→serde-wire rebuild for a union, so the typed round-trip closes
        // for both schema kinds. ($expr envelopes survive canonicalization
        // unchanged, so defense-in-depth #2 — the typed deserialize refusing an
        // expression-bearing secret — still holds.)
        valid.raw().to_typed::<C::Properties>().map_err(|_| {
            // The deserialize error text can echo the offending field value
            // (a secret); deliberately omitted — only the policy reason
            // is surfaced.
            CredentialServiceError::validation("", "credential.properties_malformed")
        })?;
        Ok(())
    });

    let ingest: IngestFn = Arc::new(|props: &serde_json::Value| {
        // Per-type ingress: a record `Properties` folds via `FieldValues::from_json`;
        // a union `Properties` folds serde's external/adjacent wire into the
        // `{mode, value}` envelope. The type-erased facade calls this so a union
        // resolves through the same envelope `validate` / `resolve` consume.
        nebula_schema::schema_of::<C::Properties>()
            .values_from_wire(props.clone())
            .map_err(|error| {
                CredentialServiceError::validation(
                    error.path.to_json_pointer(),
                    error.code.into_owned(),
                )
            })
    });

    ops.entries.insert(
        key,
        OpsEntry {
            validate,
            ingest,
            resolve,
            acquire,
            test_fn: None,
            refresh_fn: None,
            revoke_fn: None,
            continue_fn: None,
        },
    );
    tracing::info!(credential.key = key, "credential runtime ops registered");
    Ok(())
}

/// Map a canonical [`ResolveResponse`] into the secret-free
/// [`AcquireOutcome`]. Shared by the acquisition `resolve` and the
/// interactive `continue` closures so both surface `Pending`/`Retry`
/// identically.
fn map_resolve_response<C>(
    response: ResolveResponse<C::State>,
) -> Result<AcquireOutcome, CredentialServiceError>
where
    C: Credential,
{
    match response {
        ResolveResponse::Complete(state) => {
            // Cleartext serialization for the encrypted-at-rest store; bytes
            // stay in this `Zeroizing` buffer until the store encrypts them.
            let data = Zeroizing::new(
                crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&state))
                    .map_err(|e| {
                        CredentialServiceError::Internal(format!("state serialization failed: {e}"))
                    })?,
            );
            Ok(AcquireOutcome::Complete(ResolvedState {
                data,
                state_kind: <C::State as CredentialState>::KIND.to_owned(),
                state_version: <C::State as CredentialState>::VERSION,
                expires_at: state.expires_at(),
            }))
        },
        ResolveResponse::Pending { token, interaction } => {
            Ok(AcquireOutcome::Pending { token, interaction })
        },
        ResolveResponse::Retry { after, .. } => Ok(AcquireOutcome::Retry { after }),
    }
}

/// Attach the erased `test` closure for `C: Testable` onto the existing
/// `OpsEntry` at `C::KEY`. The base [`register_runtime_ops`] must have
/// run first; closure presence *is* the testable capability (mirrors
/// `plugin_capability_report`).
///
/// # Errors
///
/// [`DispatchError::BaseOpsMissing`] when `C::KEY` has no base entry —
/// the capability registration must follow the base registration.
pub fn register_testable_ops<C, PS>(ops: &mut DispatchOps<PS>) -> Result<(), DispatchError>
where
    C: Testable,
    C::Scheme: Clone,
    PS: PendingStateStore,
{
    let key: &'static str = <C as Credential>::KEY;
    let entry = ops
        .entries
        .get_mut(key)
        .ok_or(DispatchError::BaseOpsMissing { key })?;
    let test_fn: TestFn = Arc::new(|data: &[u8], ctx: &CredentialContext| {
        Box::pin(async move {
            let state: C::State = serde_json::from_slice(data).map_err(|e| {
                CredentialServiceError::Internal(format!(
                    "stored state deserialization failed: {e}"
                ))
            })?;
            let scheme = C::project(&state);
            dispatch_test::<C>(&scheme, ctx).await.map_err(|e| {
                CredentialServiceError::Provider(format!("credential test failed: {e}"))
            })
        }) as TestFuture<'_>
    });
    entry.test_fn = Some(test_fn);
    tracing::info!(credential.key = key, "credential testable ops registered");
    Ok(())
}

/// Map a framework [`ExecutorError`](crate::runtime::ExecutorError) from an
/// `execute_resolve` / `execute_continue` call onto a `CredentialServiceError`,
/// preserving the fault class instead of flattening everything to
/// `ValidationFailed` (which the API renders as a client 400).
///
/// A resolve timeout is transient (503), a pending-store backend outage is
/// internal (500), an absent / expired / already-consumed pending token means
/// "restart the interactive flow" (401), and only a genuine input problem stays
/// `validation` (400).
fn executor_error_to_service_error(e: crate::runtime::ExecutorError) -> CredentialServiceError {
    use crate::pending_store::PendingStoreError;
    use crate::runtime::ExecutorError;
    match e {
        ExecutorError::Timeout { timeout } => CredentialServiceError::TransientProvider(format!(
            "credential resolution timed out after {timeout:?}"
        )),
        ExecutorError::PendingStore(PendingStoreError::Backend(_)) => CredentialServiceError::Store,
        ExecutorError::PendingStore(PendingStoreError::ValidationFailed { .. }) => {
            CredentialServiceError::validation("", "credential.pending_invalid")
        },
        ExecutorError::PendingStore(
            PendingStoreError::NotFound
            | PendingStoreError::Expired
            | PendingStoreError::AlreadyConsumed,
        ) => CredentialServiceError::PendingExpired,
        ExecutorError::MissingSessionId => CredentialServiceError::SessionRequired {
            capability: "resolve",
        },
        ExecutorError::Credential(ce) => credential_error_to_service_error(ce),
        // Base `Credential::resolve` returned `Pending` — an internal contract
        // violation (kickoffs must use the credential-specific helpers), not a
        // client input error.
        ExecutorError::BaseResolvePending => CredentialServiceError::Internal(
            "base Credential::resolve returned Pending; interactive flows must use \
             credential-specific kickoff helpers"
                .to_owned(),
        ),
    }
}

/// Classify a [`CredentialError`](crate::CredentialError) surfaced by a
/// credential implementation into a `CredentialServiceError`, keeping its
/// transient / terminal / validation character rather than collapsing it to a
/// blanket `ValidationFailed`.
fn credential_error_to_service_error(e: crate::CredentialError) -> CredentialServiceError {
    use nebula_error::{Classify, ErrorCategory};
    if e.is_retryable() {
        return CredentialServiceError::TransientProvider(e.to_string());
    }
    match e.category() {
        ErrorCategory::Validation | ErrorCategory::NotFound => {
            CredentialServiceError::validation("", "credential.invalid")
        },
        ErrorCategory::External => CredentialServiceError::Provider(e.to_string()),
        _ => CredentialServiceError::Internal(e.to_string()),
    }
}

/// Attach the erased `refresh` closure for `C: Refreshable`. The base
/// [`register_runtime_ops`] must have run first.
///
/// # Errors
///
/// [`DispatchError::BaseOpsMissing`] when `C::KEY` has no base entry.
pub fn register_refreshable_ops<C, PS>(ops: &mut DispatchOps<PS>) -> Result<(), DispatchError>
where
    C: Refreshable,
    C::Scheme: Clone,
    PS: PendingStateStore,
{
    let key: &'static str = <C as Credential>::KEY;
    let entry = ops
        .entries
        .get_mut(key)
        .ok_or(DispatchError::BaseOpsMissing { key })?;
    let refresh_fn: RefreshFn = Arc::new(|data: &[u8], ctx: &CredentialContext| {
        Box::pin(async move {
            // Forced refresh: invoke the capability trait method directly
            // (the same call the engine's internal `perform_refresh`
            // makes — there is no public engine forced-`dispatch_refresh`;
            // `resolve_with_refresh` is early-window-gated). The service
            // re-persists the `Rewrote` bytes under compare-and-swap.
            let mut state: C::State = match serde_json::from_slice(data) {
                Ok(state) => state,
                Err(_) => {
                    return Ok(RefreshExecutionResult::PreparationFailed(
                        CredentialServiceError::Internal(
                            "stored credential state is invalid".to_owned(),
                        ),
                    ));
                },
            };
            let outcome = <C as Refreshable>::refresh(
                &mut state,
                RefreshAttempt::new(ctx, C::REFRESH_EXECUTION_MODE),
            )
            .await
            .into_kind();
            match outcome {
                outcome @ (RefreshReportKind::ProviderRefreshed
                | RefreshReportKind::LocallyRefreshed) => {
                    let phase = match outcome {
                        RefreshReportKind::ProviderRefreshed => {
                            RefreshCommitPhase::ProviderConfirmed
                        },
                        RefreshReportKind::LocallyRefreshed => RefreshCommitPhase::LocalOnly,
                        _ => return Ok(RefreshExecutionResult::OutcomeUnknown),
                    };
                    // Read the expiry off the *refreshed* state — a token
                    // rotation typically sets a new TTL. Persisting the
                    // pre-refresh `expires_at` would leave a freshly
                    // refreshed credential carrying a stale (possibly
                    // already-elapsed) expiry.
                    let expires_at = state.expires_at();
                    // Cleartext serialization for the encrypted-at-rest store.
                    let Ok(data) = crate::serde_secret::expose_for_serialization(|| {
                        serde_json::to_vec(&state)
                    }) else {
                        tracing::warn!(
                            refresh.commit_phase = ?phase,
                            "credential refresh succeeded but state serialization failed"
                        );
                        return Ok(match phase {
                            RefreshCommitPhase::ProviderConfirmed => {
                                RefreshExecutionResult::PostProviderPersistence
                            },
                            RefreshCommitPhase::LocalOnly => {
                                RefreshExecutionResult::LocalFinalizationFailed
                            },
                        });
                    };
                    let data = Zeroizing::new(data);
                    Ok(RefreshExecutionResult::Rewrote {
                        data,
                        expires_at,
                        phase,
                    })
                },
                RefreshReportKind::ReauthRequired { reason, phase } => {
                    Ok(RefreshExecutionResult::ReauthRequired { reason, phase })
                },
                RefreshReportKind::NotApplied(context) => {
                    Ok(RefreshExecutionResult::NotApplied(context))
                },
                RefreshReportKind::OutcomeUnknown => Ok(RefreshExecutionResult::OutcomeUnknown),
            }
        }) as RefreshFuture<'_>
    });
    entry.refresh_fn = Some(refresh_fn);
    tracing::info!(
        credential.key = key,
        "credential refreshable ops registered"
    );
    Ok(())
}

/// Attach the erased `revoke` closure for `C: Revocable`. The base
/// [`register_runtime_ops`] must have run first.
///
/// `Revocable::revoke` takes `&mut state` and may mutate it (e.g. clear a
/// server-side handle). Those mutations are intentionally **not**
/// re-persisted: after this closure returns,
/// [`CredentialService::revoke`](crate::CredentialService::revoke) writes a
/// **tombstone** over the row (zeroing the secret bytes), not a delete — so
/// the id is non-resurrectable and slot bindings still pointing at it surface
/// a typed `CredentialTombstoned` rather than a bare `NotFound`. This is
/// correct-by-design, not a lost write — unlike `refresh`, which keeps the
/// row alive and CAS-persists its mutated state.
///
/// # Errors
///
/// [`DispatchError::BaseOpsMissing`] when `C::KEY` has no base entry.
pub fn register_revocable_ops<C, PS>(ops: &mut DispatchOps<PS>) -> Result<(), DispatchError>
where
    C: Revocable,
    C::Scheme: Clone,
    PS: PendingStateStore,
{
    let key: &'static str = <C as Credential>::KEY;
    let entry = ops
        .entries
        .get_mut(key)
        .ok_or(DispatchError::BaseOpsMissing { key })?;
    let revoke_fn: RevokeFn = Arc::new(|data: &[u8], ctx: &CredentialContext| {
        Box::pin(async move {
            let mut state: C::State = serde_json::from_slice(data).map_err(|e| {
                CredentialServiceError::Internal(format!(
                    "stored state deserialization failed: {e}"
                ))
            })?;
            // `revoke` may mutate `state`; the mutation is deliberately
            // dropped here. After this closure returns, the service writes a
            // tombstone over the row (zeroing the secret bytes) rather than
            // deleting it — so the id is non-resurrectable and slot bindings
            // still pointing at it surface a typed `CredentialTombstoned`
            // error. See this fn's doc comment and `CredentialService::revoke`.
            dispatch_revoke::<C>(&mut state, ctx)
                .await
                .map_err(|error| match error {
                    // The integration may know that provider work completed
                    // and only local finalization failed.
                    crate::CredentialError::PostProviderPersistence => {
                        CredentialServiceError::RevokePostProviderPersistence
                    },
                    // Once the erased revoke implementation is entered, any
                    // other error is phase-ambiguous: the trait has no proof
                    // that provider-side revocation did not occur.
                    _ => CredentialServiceError::OutcomeUnknown,
                })
        }) as RevokeFuture<'_>
    });
    entry.revoke_fn = Some(revoke_fn);
    tracing::info!(credential.key = key, "credential revocable ops registered");
    Ok(())
}

/// Attach the erased interactive `continue` closure for
/// `C: Interactive`. The base [`register_runtime_ops`] must have run
/// first.
///
/// # Errors
///
/// [`DispatchError::BaseOpsMissing`] when `C::KEY` has no base entry.
pub fn register_interactive_ops<C, PS>(ops: &mut DispatchOps<PS>) -> Result<(), DispatchError>
where
    C: Interactive,
    C::Scheme: Clone,
    PS: PendingStateStore,
{
    let key: &'static str = <C as Credential>::KEY;
    let entry = ops
        .entries
        .get_mut(key)
        .ok_or(DispatchError::BaseOpsMissing { key })?;
    let continue_fn: ContinueFn<PS> = Arc::new(
        |token: &PendingToken, input: &UserInput, ctx: &CredentialContext, pending: &PS| {
            Box::pin(async move {
                let response = execute_continue::<C, PS>(token, input, ctx, pending)
                    .await
                    .map_err(executor_error_to_service_error)?;
                map_resolve_response::<C>(response)
            }) as AcquireFuture<'_>
        },
    );
    entry.continue_fn = Some(continue_fn);
    tracing::info!(
        credential.key = key,
        "credential interactive ops registered"
    );
    Ok(())
}

/// Register the base runtime ops for the three first-party builtins
/// (`bearer_token`, `shared_key`, `signing_key`). All three are static
/// (no capability impls), so no capability-bounded `register_*_ops` is
/// called for them — that is correct: closure absence is "capability not
/// supported". Mirrors [`nebula_credential::register_builtins`].
///
/// # Errors
///
/// [`DispatchError::DuplicateKey`] if any builtin key is already present.
pub fn register_all_builtin_ops<PS>(ops: &mut DispatchOps<PS>) -> Result<(), DispatchError>
where
    PS: PendingStateStore,
{
    register_runtime_ops::<crate::BearerTokenCredential, PS>(ops)?;
    register_runtime_ops::<crate::SharedKeyCredential, PS>(ops)?;
    register_runtime_ops::<crate::SigningKeyCredential, PS>(ops)?;
    Ok(())
}
