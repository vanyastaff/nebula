//! Closed effect declarations in compiler-three executable-plan records.

use std::sync::Arc;

use nebula_action::{
    ActionFactory,
    effect::{
        ActionEffectContract, RemoteDestinationGuarantee, RemoteEffectDescriptor,
        RemoteEffectPolicy,
    },
};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum RecordedActionEffectV1 {
    NoExternalEffects,
    Remote {
        contract_id: String,
        canonicalization_version: u16,
        policy: RecordedEffectPolicyV1,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum RecordedDestinationCapabilityV1 {
    StableKey,
    Reconcilable,
    Opaque,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedEffectPolicyV1 {
    capability: RecordedDestinationCapabilityV1,
    max_invocations: u32,
    max_queries: u32,
    recovery_window_ms: u64,
    stable_window_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct InvalidEffectContract;

impl RecordedActionEffectV1 {
    pub(crate) fn project(contract: &ActionEffectContract) -> Result<Self, InvalidEffectContract> {
        match contract {
            ActionEffectContract::NoExternalEffects => Ok(Self::NoExternalEffects),
            ActionEffectContract::Remote(descriptor) => {
                descriptor.validate().map_err(|_| InvalidEffectContract)?;
                let policy = descriptor.policy();
                let capability = match policy.destination_guarantee() {
                    RemoteDestinationGuarantee::StableKey(_) => {
                        RecordedDestinationCapabilityV1::StableKey
                    },
                    RemoteDestinationGuarantee::Reconcilable => {
                        RecordedDestinationCapabilityV1::Reconcilable
                    },
                    RemoteDestinationGuarantee::Opaque => RecordedDestinationCapabilityV1::Opaque,
                    _ => return Err(InvalidEffectContract),
                };
                Ok(Self::Remote {
                    contract_id: descriptor.contract_id().to_owned(),
                    canonicalization_version: descriptor.canonicalization_version(),
                    policy: RecordedEffectPolicyV1 {
                        capability,
                        max_invocations: policy.max_invocations(),
                        max_queries: policy.max_queries(),
                        recovery_window_ms: policy.recovery_window_ms(),
                        stable_window_ms: policy.destination_guarantee().stable_window_ms(),
                    },
                })
            },
            _ => Err(InvalidEffectContract),
        }
    }

    pub(crate) fn checked_contract(&self) -> Result<ActionEffectContract, InvalidEffectContract> {
        match self {
            Self::NoExternalEffects => Ok(ActionEffectContract::NoExternalEffects),
            Self::Remote {
                contract_id,
                canonicalization_version,
                policy,
            } => {
                let destination_guarantee = match policy.capability {
                    RecordedDestinationCapabilityV1::StableKey => {
                        RemoteDestinationGuarantee::stable_key(
                            policy.stable_window_ms.ok_or(InvalidEffectContract)?,
                        )
                        .map_err(|_| InvalidEffectContract)?
                    },
                    RecordedDestinationCapabilityV1::Reconcilable => {
                        if policy.stable_window_ms.is_some() {
                            return Err(InvalidEffectContract);
                        }
                        RemoteDestinationGuarantee::Reconcilable
                    },
                    RecordedDestinationCapabilityV1::Opaque => {
                        if policy.stable_window_ms.is_some() {
                            return Err(InvalidEffectContract);
                        }
                        RemoteDestinationGuarantee::Opaque
                    },
                };
                let policy = RemoteEffectPolicy::builder(destination_guarantee)
                    .maximum_invocations(policy.max_invocations)
                    .maximum_queries(policy.max_queries)
                    .recovery_window(std::time::Duration::from_millis(policy.recovery_window_ms))
                    .build()
                    .map_err(|_| InvalidEffectContract)?;
                RemoteEffectDescriptor::new(contract_id, *canonicalization_version, policy)
                    .map(|descriptor| ActionEffectContract::Remote(Box::new(descriptor)))
                    .map_err(|_| InvalidEffectContract)
            },
        }
    }
}

/// Check the retained factory without constructing an executable action.
pub(crate) fn validate_factory_effect(
    declared: &ActionEffectContract,
    factory: &dyn ActionFactory,
) -> Result<(), InvalidEffectContract> {
    match (declared, factory.remote_effect_factory()) {
        (ActionEffectContract::Remote(descriptor), Some(capability)) => {
            descriptor.validate().map_err(|_| InvalidEffectContract)?;
            if !Arc::ptr_eq(factory.metadata(), capability.metadata())
                || descriptor.as_ref() != capability.descriptor()
            {
                return Err(InvalidEffectContract);
            }
            Ok(())
        },
        (ActionEffectContract::NoExternalEffects | ActionEffectContract::Undeclared, None) => {
            Ok(())
        },
        _ => Err(InvalidEffectContract),
    }
}
