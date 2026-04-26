//! `CredentialRegistry` — keyed by `Credential::KEY`. Append-only after
//! startup. Fatal duplicate-KEY per Tech Spec §15.6 (closes
//! security-lead N7 — supply-chain credential takeover via duplicate
//! KEY collision).
//!
//! # Why fatal, not warn-overwrite
//!
//! The previous shape (Tech Spec §3.1 lines 662-663, pre-§15.6) used
//! `HashMap::insert` semantics: the second registration of the same
//! `KEY` overwrote the first with only a `tracing::warn!` log. A
//! malicious or stale plugin shipping with a colliding `KEY` (e.g.
//! `"slack.oauth2"`) silently took over every `CredentialRef<SlackOAuth2>`
//! resolve — including its `resolve()`, `refresh()`, and stored state.
//! The warn-log was the only signal, easily lost in startup noise.
//!
//! §15.6 makes the second registration a hard error in BOTH debug and
//! release builds. Operators surface a clear, actionable failure at
//! startup; resolution is via plugin uninstall, version pin, or
//! namespace fix. Long-term defense is `arch-signing-infra` (queue #7,
//! post-MVP) — signed plugin manifests prove provenance and eliminate
//! the supply-chain risk entirely. §15.6 is the interim mitigation.
//!
//! # Append-only invariant
//!
//! Per Tech Spec line 669 ("registration invariant — registry is
//! append-only after startup"): the registry is mutated only during
//! service initialization (plugin registration phase). Runtime
//! credential resolution never mutates the registry, enabling the
//! lock-free hot read path. Hot-reload of credential types is
//! explicitly OUT of scope — restarting the service is the mechanism
//! for picking up new credential types (e.g., after loading a new
//! plugin).

use std::sync::Arc;

use ahash::AHashMap;

use super::{
    any::AnyCredential,
    capability_report::{Capabilities, compute_capabilities},
    credential::Credential,
};

/// Errors raised by [`CredentialRegistry::register`].
///
/// Currently single-variant — duplicate KEY is the only registration-time
/// failure that §15.6 promotes from "stealthy overwrite" to "hard error".
/// Marked `#[non_exhaustive]` so future registration-time validations
/// (e.g., metadata schema mismatch, plugin signature failures from the
/// post-MVP `arch-signing-infra` work) extend the enum without breaking
/// downstream `match` exhaustiveness.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum RegisterError {
    /// Two registrations submitted credentials sharing the same
    /// `Credential::KEY`. The first registration remains authoritative;
    /// the second is rejected. Operator resolves via plugin uninstall,
    /// version pin, or namespace fix.
    #[error(
        "duplicate credential key '{key}': existing crate {existing_crate}, \
         new crate {new_crate} (Tech Spec §15.6 fail-closed; resolve via plugin \
         uninstall, version pin, or namespace fix)"
    )]
    DuplicateKey {
        /// The colliding `Credential::KEY` whose second registration
        /// was rejected. Operators surface this verbatim in logs to
        /// identify which credential type collision occurred.
        key: &'static str,
        /// `CARGO_CRATE_NAME` of the crate that owns the first
        /// (authoritative) registration.
        existing_crate: &'static str,
        /// `CARGO_CRATE_NAME` of the crate whose second registration
        /// was rejected.
        new_crate: &'static str,
    },
}

/// KEY-keyed credential registry. Stores `Box<dyn AnyCredential>`
/// instances alongside their computed [`Capabilities`] set and the
/// `CARGO_CRATE_NAME` of the registering crate. Lookup is zero-allocation
/// via `Borrow<str>` on the `Arc<str>` key.
///
/// Created empty at service startup; populated by plugin init via
/// [`CredentialRegistry::register`]. After startup the registry is
/// read-only; concurrent reads are safe without locking because no
/// further mutations occur (Tech Spec §3.1 hot-path invariant).
pub struct CredentialRegistry {
    entries: AHashMap<Arc<str>, RegistryEntry>,
}

/// Internal storage row — one per registered credential KEY.
struct RegistryEntry {
    instance: Box<dyn AnyCredential>,
    capabilities: Capabilities,
    registering_crate: &'static str,
}

impl CredentialRegistry {
    /// Construct an empty registry. Typically created once per service
    /// instance and populated at startup via [`Self::register`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: AHashMap::new(),
        }
    }

    /// Register a concrete credential. Fatal on duplicate KEY in BOTH
    /// debug and release builds (Tech Spec §15.6).
    ///
    /// # First-wins semantics
    ///
    /// On collision the existing entry is retained unchanged and the
    /// caller receives [`RegisterError::DuplicateKey`]. The registry
    /// state is identical before and after a failed `register` call.
    ///
    /// # Errors
    ///
    /// Returns [`RegisterError::DuplicateKey`] if `C::KEY` is already
    /// present in the registry. Operators resolve via plugin uninstall,
    /// version pin, or namespace fix.
    pub fn register<C>(
        &mut self,
        instance: C,
        registering_crate: &'static str,
    ) -> Result<(), RegisterError>
    where
        C: Credential,
    {
        let key: &'static str = C::KEY;
        if let Some(existing) = self.entries.get(key) {
            tracing::error!(
                credential.key = key,
                existing_crate = existing.registering_crate,
                new_crate = registering_crate,
                "duplicate credential KEY rejected (Tech Spec §15.6 fail-closed)"
            );
            return Err(RegisterError::DuplicateKey {
                key,
                existing_crate: existing.registering_crate,
                new_crate: registering_crate,
            });
        }

        let capabilities = compute_capabilities::<C>();
        let arc_key: Arc<str> = key.into();
        self.entries.insert(
            arc_key,
            RegistryEntry {
                instance: Box::new(instance),
                capabilities,
                registering_crate,
            },
        );
        tracing::info!(
            credential.key = key,
            registering_crate,
            ?capabilities,
            "credential registered"
        );
        Ok(())
    }

    /// Type-erased lookup by KEY. Returns `None` if no credential is
    /// registered under `key`.
    #[must_use]
    pub fn resolve_any(&self, key: &str) -> Option<&(dyn AnyCredential + 'static)> {
        self.entries.get(key).map(|e| &*e.instance)
    }

    /// Typed lookup by KEY — downcasts the stored
    /// `Box<dyn AnyCredential>` to `&C` after a `TypeId` check via
    /// `Any::downcast_ref`. Returns `None` if either the KEY is
    /// unregistered OR the registered entry is a different type than
    /// `C` (concrete-type mismatch).
    #[must_use]
    pub fn resolve<C: Credential>(&self, key: &str) -> Option<&C> {
        let entry = self.entries.get(key)?;
        entry.instance.as_any().downcast_ref::<C>()
    }

    /// Returns the capability set for the credential at `key`, if registered.
    ///
    /// **Stage 5 stub.** Currently returns [`Capabilities::empty()`] for every
    /// entry until Stage 7 (Tech Spec §15.8) wires real detection via
    /// `plugin_capability_report::*` per-credential constants. Consumers must
    /// not rely on this for security-relevant decisions until Stage 7 lands.
    ///
    /// Hidden from the rustdoc index during the Stage 5–7 window so external
    /// consumers cannot accidentally bind to the empty-stub semantics. Stage 7
    /// will re-expose this for the engine `iter_compatible` filter (Tech Spec
    /// §15.8) without an API churn.
    #[doc(hidden)]
    #[must_use]
    pub fn capabilities_of(&self, key: &str) -> Option<Capabilities> {
        self.entries.get(key).map(|e| e.capabilities)
    }

    /// `CARGO_CRATE_NAME` of the crate that registered `key`. Returns
    /// `None` if `key` is unregistered. Used by the
    /// `RegisterError::DuplicateKey` operator-surface message and by
    /// audit-log tooling to attribute credential ownership across the
    /// plugin tree.
    #[must_use]
    pub fn registering_crate_of(&self, key: &str) -> Option<&'static str> {
        self.entries.get(key).map(|e| e.registering_crate)
    }

    /// Number of registered credentials.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when no credentials are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns `true` when a credential is registered under `key`.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}

impl std::fmt::Debug for CredentialRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRegistry")
            .field("registered_keys", &self.entries.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for CredentialRegistry {
    fn default() -> Self {
        Self::new()
    }
}
