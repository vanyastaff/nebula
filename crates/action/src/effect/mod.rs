//! Trusted effect declarations and preparation before provider I/O.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};

mod adapter;
pub(crate) use adapter::sealed as remote_effect_sealed;
pub use adapter::{
    EffectFailureCode, EffectInvocationContext, EffectInvocationOutcome, EffectPreparationContext,
    EffectPreparationError, EffectQueryContext, EffectReconciliationOutcome, PreparedEffectAdapter,
    PreparedRemoteEffect, ReadOnlyEffectQuery, RemoteEffectAction, RemoteEffectFactory,
};

/// Explicit effect declaration retained in the exact compiled action contract.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ActionEffectContract {
    /// No trusted declaration exists; durable admission must reject this action.
    #[default]
    Undeclared,
    /// The adapter performs no external business effect, including during construction.
    NoExternalEffects,
    /// Provider effects require the declared preparation and recovery protocol.
    Remote(Box<RemoteEffectDescriptor>),
}

/// Provider guarantee available to recover one remote business effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RemoteDestinationGuarantee {
    /// Reusing the operation key cannot repeat the effect during this window.
    StableKey(StableKeyGuarantee),
    /// An authenticated read-only query can determine the original outcome.
    Reconcilable,
    /// The provider offers neither stable-key nor reconciliation guarantees.
    Opaque,
}

impl RemoteDestinationGuarantee {
    /// Declare the nonzero lifetime of a provider's stable operation key.
    ///
    /// # Errors
    /// Rejects a zero lifetime. The policy constructor additionally requires
    /// this lifetime to fit within the recovery window.
    pub fn stable_key(validity_window_ms: u64) -> Result<Self, RemoteEffectPolicyError> {
        StableKeyGuarantee::new(validity_window_ms).map(Self::StableKey)
    }

    /// Provider guarantee lifetime when a stable key is available.
    #[must_use]
    pub const fn stable_window_ms(self) -> Option<u64> {
        match self {
            Self::StableKey(guarantee) => Some(guarantee.validity_window_ms()),
            Self::Reconcilable | Self::Opaque => None,
        }
    }
}

/// Validated, nonzero lifetime of a provider's stable operation key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StableKeyGuarantee {
    validity_window_ms: u64,
}

impl StableKeyGuarantee {
    fn new(validity_window_ms: u64) -> Result<Self, RemoteEffectPolicyError> {
        if validity_window_ms == 0 {
            return Err(RemoteEffectPolicyError::StableKeyWindow);
        }
        Ok(Self { validity_window_ms })
    }

    /// Provider guarantee lifetime from preparation, in milliseconds.
    #[must_use]
    pub const fn validity_window_ms(self) -> u64 {
        self.validity_window_ms
    }
}

impl<'de> Deserialize<'de> for StableKeyGuarantee {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Author-declared finite recovery policy for a remote effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteEffectPolicy {
    destination_guarantee: RemoteDestinationGuarantee,
    max_invocations: u32,
    max_queries: u32,
    recovery_window_ms: u64,
}

/// Named construction of an author-declared remote-effect policy.
#[derive(Debug)]
#[must_use = "a remote-effect policy builder must be completed with `build`"]
pub struct RemoteEffectPolicyBuilder {
    destination_guarantee: RemoteDestinationGuarantee,
    max_invocations: Option<u32>,
    max_queries: Option<u32>,
    recovery_window: Option<Duration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteEffectPolicyWire {
    destination_guarantee: RemoteDestinationGuarantee,
    max_invocations: u32,
    max_queries: u32,
    recovery_window_ms: u64,
}

impl RemoteEffectPolicy {
    /// Begin a named declaration of finite provider recovery limits.
    pub const fn builder(
        destination_guarantee: RemoteDestinationGuarantee,
    ) -> RemoteEffectPolicyBuilder {
        RemoteEffectPolicyBuilder {
            destination_guarantee,
            max_invocations: None,
            max_queries: None,
            recovery_window: None,
        }
    }

    fn from_milliseconds(
        destination_guarantee: RemoteDestinationGuarantee,
        max_invocations: u32,
        max_queries: u32,
        recovery_window_ms: u64,
    ) -> Result<Self, RemoteEffectPolicyError> {
        let policy = Self {
            destination_guarantee,
            max_invocations,
            max_queries,
            recovery_window_ms,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Validate the complete policy after decoding or named construction.
    ///
    /// # Errors
    /// Rejects empty or excessive budgets, an invalid recovery window, an
    /// opaque destination with queries, or a stable-key guarantee outside the
    /// recovery window.
    fn validate(&self) -> Result<(), RemoteEffectPolicyError> {
        let stable_window_is_invalid = self
            .destination_guarantee
            .stable_window_ms()
            .is_some_and(|window| window == 0 || window > self.recovery_window_ms);
        if self.max_invocations == 0 || self.max_invocations > 10_000 {
            return Err(RemoteEffectPolicyError::InvocationLimit);
        }
        if self.max_queries > 10_000 {
            return Err(RemoteEffectPolicyError::QueryLimit);
        }
        if self.recovery_window_ms == 0 || self.recovery_window_ms > 31_536_000_000 {
            return Err(RemoteEffectPolicyError::RecoveryWindow);
        }
        if stable_window_is_invalid {
            return Err(RemoteEffectPolicyError::StableKeyWindow);
        }
        if self.destination_guarantee == RemoteDestinationGuarantee::Opaque && self.max_queries != 0
        {
            return Err(RemoteEffectPolicyError::OpaqueQueries);
        }
        Ok(())
    }

    /// Destination guarantee declared by the provider adapter.
    #[must_use]
    pub const fn destination_guarantee(&self) -> RemoteDestinationGuarantee {
        self.destination_guarantee
    }
    /// Total permitted effect calls, including the first call.
    #[must_use]
    pub const fn max_invocations(&self) -> u32 {
        self.max_invocations
    }
    /// Total permitted authenticated read-only queries.
    #[must_use]
    pub const fn max_queries(&self) -> u32 {
        self.max_queries
    }
    /// Recovery lifetime from preparation, in milliseconds.
    #[must_use]
    pub const fn recovery_window_ms(&self) -> u64 {
        self.recovery_window_ms
    }
}

impl RemoteEffectPolicyBuilder {
    /// Set the total permitted provider invocations, including the first call.
    pub const fn maximum_invocations(mut self, maximum: u32) -> Self {
        self.max_invocations = Some(maximum);
        self
    }

    /// Set the total permitted authenticated read-only queries.
    pub const fn maximum_queries(mut self, maximum: u32) -> Self {
        self.max_queries = Some(maximum);
        self
    }

    /// Set the finite lifetime available for recovery.
    pub const fn recovery_window(mut self, window: Duration) -> Self {
        self.recovery_window = Some(window);
        self
    }

    /// Validate and finish the policy declaration.
    ///
    /// # Errors
    /// Returns a specific [`RemoteEffectPolicyError`] when a required limit is
    /// absent or the complete destination policy is incoherent.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use nebula_action::{RemoteDestinationGuarantee, RemoteEffectPolicy};
    ///
    /// let policy = RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
    ///     .maximum_invocations(1)
    ///     .maximum_queries(0)
    ///     .recovery_window(Duration::from_mins(1))
    ///     .build()?;
    /// assert_eq!(policy.max_invocations(), 1);
    /// # Ok::<(), nebula_action::RemoteEffectPolicyError>(())
    /// ```
    pub fn build(self) -> Result<RemoteEffectPolicy, RemoteEffectPolicyError> {
        let max_invocations = self
            .max_invocations
            .ok_or(RemoteEffectPolicyError::InvocationLimit)?;
        let max_queries = self
            .max_queries
            .ok_or(RemoteEffectPolicyError::QueryLimit)?;
        let recovery_window = self
            .recovery_window
            .ok_or(RemoteEffectPolicyError::RecoveryWindow)?;
        let recovery_window_ms = u64::try_from(recovery_window.as_millis())
            .map_err(|_| RemoteEffectPolicyError::RecoveryWindow)?;
        RemoteEffectPolicy::from_milliseconds(
            self.destination_guarantee,
            max_invocations,
            max_queries,
            recovery_window_ms,
        )
    }
}

impl TryFrom<RemoteEffectPolicyWire> for RemoteEffectPolicy {
    type Error = RemoteEffectPolicyError;

    fn try_from(wire: RemoteEffectPolicyWire) -> Result<Self, Self::Error> {
        Self::from_milliseconds(
            wire.destination_guarantee,
            wire.max_invocations,
            wire.max_queries,
            wire.recovery_window_ms,
        )
    }
}

/// Specific reason an author-declared recovery policy is incoherent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RemoteEffectPolicyError {
    /// Invocation budget must be between one and 10,000.
    #[error("remote effect invocation limit is outside the supported range")]
    InvocationLimit,
    /// Query budget cannot exceed 10,000.
    #[error("remote effect query limit is outside the supported range")]
    QueryLimit,
    /// Recovery must finish within a nonzero window of at most one year.
    #[error("remote effect recovery window is outside the supported range")]
    RecoveryWindow,
    /// Stable-key validity must be nonzero and cover no more than recovery.
    #[error("remote effect stable-key window is outside the recovery window")]
    StableKeyWindow,
    /// Opaque destinations cannot declare an authoritative query.
    #[error("opaque remote effects cannot declare reconciliation queries")]
    OpaqueQueries,
}

impl<'de> Deserialize<'de> for RemoteEffectPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(RemoteEffectPolicyWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

/// Versioned adapter contract and complete finite recovery policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteEffectDescriptor {
    contract_id: String,
    canonicalization_version: u16,
    policy: RemoteEffectPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteEffectDescriptorWire {
    contract_id: String,
    canonicalization_version: u16,
    policy: RemoteEffectPolicy,
}

/// Bounded rejection of an invalid effect declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EffectContractError {
    /// The static contract identifier is empty, oversized, or malformed.
    #[error("invalid remote effect contract identifier")]
    InvalidIdentifier,
    /// Canonicalization version zero carries no supported contract.
    #[error("invalid remote effect canonicalization version")]
    InvalidCanonicalizationVersion,
}

impl RemoteEffectDescriptor {
    /// Declare a canonicalization contract and finite provider recovery policy.
    ///
    /// # Errors
    /// Rejects malformed identifiers, version zero, or invalid policy limits.
    pub fn new(
        contract_id: impl Into<String>,
        canonicalization_version: u16,
        policy: RemoteEffectPolicy,
    ) -> Result<Self, EffectContractError> {
        let descriptor = Self {
            contract_id: contract_id.into(),
            canonicalization_version,
            policy,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Validate a declaration before compilation or factory dispatch.
    ///
    /// # Errors
    ///
    /// Returns [`EffectContractError::InvalidIdentifier`] for a malformed
    /// contract id and [`EffectContractError::InvalidCanonicalizationVersion`]
    /// when the canonicalization version is zero.
    pub fn validate(&self) -> Result<(), EffectContractError> {
        if self.contract_id.is_empty()
            || self.contract_id.len() > 128
            || !self.contract_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
            })
        {
            return Err(EffectContractError::InvalidIdentifier);
        }
        if self.canonicalization_version == 0 {
            return Err(EffectContractError::InvalidCanonicalizationVersion);
        }
        Ok(())
    }

    /// Exact integration contract identity; it is not a provider URL or secret.
    #[must_use]
    pub fn contract_id(&self) -> &str {
        &self.contract_id
    }
    /// Version of the adapter's canonical logical request encoding.
    #[must_use]
    pub const fn canonicalization_version(&self) -> u16 {
        self.canonicalization_version
    }
    /// Complete pinned author declaration.
    #[must_use]
    pub const fn policy(&self) -> &RemoteEffectPolicy {
        &self.policy
    }
}

impl TryFrom<RemoteEffectDescriptorWire> for RemoteEffectDescriptor {
    type Error = EffectContractError;

    fn try_from(wire: RemoteEffectDescriptorWire) -> Result<Self, Self::Error> {
        Self::new(wire.contract_id, wire.canonicalization_version, wire.policy)
    }
}

impl<'de> Deserialize<'de> for RemoteEffectDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(RemoteEffectDescriptorWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RemoteEffectPolicy {
        RemoteEffectPolicy::builder(RemoteDestinationGuarantee::stable_key(30_000).unwrap())
            .maximum_invocations(3)
            .maximum_queries(2)
            .recovery_window(Duration::from_mins(1))
            .build()
            .unwrap()
    }

    #[test]
    fn decoded_descriptors_are_validated() {
        let descriptor = RemoteEffectDescriptor::new("provider.charge/v1", 2, policy()).unwrap();
        let mut wire = serde_json::to_value(&descriptor).unwrap();
        let decoded: RemoteEffectDescriptor = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(decoded, descriptor);
        wire["policy"]["max_invocations"] = serde_json::json!(0);
        assert!(serde_json::from_value::<RemoteEffectDescriptor>(wire.clone()).is_err());
        wire["policy"]["max_invocations"] = serde_json::json!(1);
        wire["policy"]["destination_guarantee"]["StableKey"] = serde_json::json!(0);
        assert!(serde_json::from_value::<RemoteEffectDescriptor>(wire).is_err());
    }

    #[test]
    fn policies_reject_incoherent_destination_guarantees() {
        assert_eq!(
            RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
                .maximum_invocations(1)
                .maximum_queries(1)
                .recovery_window(Duration::from_secs(1))
                .build(),
            Err(RemoteEffectPolicyError::OpaqueQueries)
        );
        assert_eq!(
            RemoteEffectPolicy::builder(RemoteDestinationGuarantee::stable_key(2_000).unwrap(),)
                .maximum_invocations(1)
                .maximum_queries(0)
                .recovery_window(Duration::from_secs(1))
                .build(),
            Err(RemoteEffectPolicyError::StableKeyWindow)
        );
    }

    #[test]
    fn named_policy_builder_requires_every_limit() {
        let missing_queries = RemoteEffectPolicy::builder(RemoteDestinationGuarantee::Opaque)
            .maximum_invocations(1)
            .recovery_window(Duration::from_secs(1))
            .build();

        assert_eq!(missing_queries, Err(RemoteEffectPolicyError::QueryLimit));
    }

    #[test]
    fn descriptors_reject_invalid_identity() {
        for identifier in [
            String::new(),
            "x".repeat(129),
            "provider\nsecret".to_owned(),
        ] {
            assert_eq!(
                RemoteEffectDescriptor::new(identifier, 1, policy()),
                Err(EffectContractError::InvalidIdentifier)
            );
        }
        assert_eq!(
            RemoteEffectDescriptor::new("provider.charge", 0, policy()),
            Err(EffectContractError::InvalidCanonicalizationVersion)
        );
    }
}
