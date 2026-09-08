//! Typed boundary for identifiers defined by the external gate registry.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(super) enum ExternalGateId {
    #[serde(rename = "NS01")]
    ExecutionIdentity,
    #[serde(rename = "NS02")]
    ExactRevisionRouting,
    #[serde(rename = "NS03")]
    ClaimGenerationFencing,
    #[serde(rename = "NS04")]
    KeyedAcceptance,
    #[serde(rename = "NS05")]
    ClaimHandoff,
    #[serde(rename = "NS06")]
    AdmissionFairness,
    #[serde(rename = "NS07")]
    PersistenceConformance,
    #[serde(rename = "NS08")]
    RequiredPostgresql,
    #[serde(rename = "NS09")]
    OrderedMigrations,
    #[serde(rename = "NS10")]
    ResourceLifecycleRecovery,
    #[serde(rename = "NS11")]
    TenantIsolation,
    #[serde(rename = "NS12")]
    TypedActionOnboarding,
    #[serde(rename = "NS13")]
    CredentialResourceOnboarding,
    #[serde(rename = "NS14")]
    ActivationDiagnostics,
    #[serde(rename = "NS15")]
    OpenApiRuntimeConformance,
    #[serde(rename = "NS16")]
    IncidentDiagnosis,
    #[serde(rename = "NS17")]
    RemoteEffects,
    #[serde(rename = "NS18")]
    SdkOnlyIntegration,
    #[serde(rename = "NS19")]
    PublishableManifests,
    #[serde(rename = "NS20")]
    SupportedApiSurface,
    #[serde(rename = "NS21")]
    RuntimeOutcomeObservability,
    #[serde(rename = "NS22")]
    HybridPersistenceGovernance,
}

impl ExternalGateId {
    pub(super) const ALL: [Self; 22] = [
        Self::ExecutionIdentity,
        Self::ExactRevisionRouting,
        Self::ClaimGenerationFencing,
        Self::KeyedAcceptance,
        Self::ClaimHandoff,
        Self::AdmissionFairness,
        Self::PersistenceConformance,
        Self::RequiredPostgresql,
        Self::OrderedMigrations,
        Self::ResourceLifecycleRecovery,
        Self::TenantIsolation,
        Self::TypedActionOnboarding,
        Self::CredentialResourceOnboarding,
        Self::ActivationDiagnostics,
        Self::OpenApiRuntimeConformance,
        Self::IncidentDiagnosis,
        Self::RemoteEffects,
        Self::SdkOnlyIntegration,
        Self::PublishableManifests,
        Self::SupportedApiSurface,
        Self::RuntimeOutcomeObservability,
        Self::HybridPersistenceGovernance,
    ];
}

impl From<ExternalGateId> for &'static str {
    fn from(gate: ExternalGateId) -> Self {
        match gate {
            ExternalGateId::ExecutionIdentity => "NS01",
            ExternalGateId::ExactRevisionRouting => "NS02",
            ExternalGateId::ClaimGenerationFencing => "NS03",
            ExternalGateId::KeyedAcceptance => "NS04",
            ExternalGateId::ClaimHandoff => "NS05",
            ExternalGateId::AdmissionFairness => "NS06",
            ExternalGateId::PersistenceConformance => "NS07",
            ExternalGateId::RequiredPostgresql => "NS08",
            ExternalGateId::OrderedMigrations => "NS09",
            ExternalGateId::ResourceLifecycleRecovery => "NS10",
            ExternalGateId::TenantIsolation => "NS11",
            ExternalGateId::TypedActionOnboarding => "NS12",
            ExternalGateId::CredentialResourceOnboarding => "NS13",
            ExternalGateId::ActivationDiagnostics => "NS14",
            ExternalGateId::OpenApiRuntimeConformance => "NS15",
            ExternalGateId::IncidentDiagnosis => "NS16",
            ExternalGateId::RemoteEffects => "NS17",
            ExternalGateId::SdkOnlyIntegration => "NS18",
            ExternalGateId::PublishableManifests => "NS19",
            ExternalGateId::SupportedApiSurface => "NS20",
            ExternalGateId::RuntimeOutcomeObservability => "NS21",
            ExternalGateId::HybridPersistenceGovernance => "NS22",
        }
    }
}

impl TryFrom<&str> for ExternalGateId {
    type Error = &'static str;

    fn try_from(identifier: &str) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|gate| <&'static str>::from(*gate) == identifier)
            .ok_or("unknown external gate identifier")
    }
}

impl fmt::Display for ExternalGateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(<&'static str>::from(*self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeAuthorityGate {
    ExecutionIdentity,
    ExactRevisionRouting,
    ClaimGenerationFencing,
    KeyedAcceptance,
    ClaimHandoff,
    PersistenceConformance,
    RequiredPostgresql,
    OrderedMigrations,
    ActivationDiagnostics,
    RemoteEffects,
}

impl RuntimeAuthorityGate {
    pub(super) const ALL: [Self; 10] = [
        Self::ExecutionIdentity,
        Self::ExactRevisionRouting,
        Self::ClaimGenerationFencing,
        Self::KeyedAcceptance,
        Self::ClaimHandoff,
        Self::PersistenceConformance,
        Self::RequiredPostgresql,
        Self::OrderedMigrations,
        Self::ActivationDiagnostics,
        Self::RemoteEffects,
    ];
}

impl From<RuntimeAuthorityGate> for ExternalGateId {
    fn from(gate: RuntimeAuthorityGate) -> Self {
        match gate {
            RuntimeAuthorityGate::ExecutionIdentity => Self::ExecutionIdentity,
            RuntimeAuthorityGate::ExactRevisionRouting => Self::ExactRevisionRouting,
            RuntimeAuthorityGate::ClaimGenerationFencing => Self::ClaimGenerationFencing,
            RuntimeAuthorityGate::KeyedAcceptance => Self::KeyedAcceptance,
            RuntimeAuthorityGate::ClaimHandoff => Self::ClaimHandoff,
            RuntimeAuthorityGate::PersistenceConformance => Self::PersistenceConformance,
            RuntimeAuthorityGate::RequiredPostgresql => Self::RequiredPostgresql,
            RuntimeAuthorityGate::OrderedMigrations => Self::OrderedMigrations,
            RuntimeAuthorityGate::ActivationDiagnostics => Self::ActivationDiagnostics,
            RuntimeAuthorityGate::RemoteEffects => Self::RemoteEffects,
        }
    }
}

impl TryFrom<ExternalGateId> for RuntimeAuthorityGate {
    type Error = ();

    fn try_from(gate: ExternalGateId) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|candidate| ExternalGateId::from(*candidate) == gate)
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(super) enum ActivationCheckpoint {
    #[serde(rename = "CP0")]
    Foundation,
    #[serde(rename = "CP1")]
    DomainContracts,
    #[serde(rename = "CP2")]
    DurableRuntime,
    #[serde(rename = "CP3")]
    Integration,
    #[serde(rename = "CP4")]
    Operations,
    #[serde(rename = "CP5")]
    Performance,
    #[serde(rename = "CP6")]
    Product,
}

impl From<ActivationCheckpoint> for &'static str {
    fn from(checkpoint: ActivationCheckpoint) -> Self {
        match checkpoint {
            ActivationCheckpoint::Foundation => "CP0",
            ActivationCheckpoint::DomainContracts => "CP1",
            ActivationCheckpoint::DurableRuntime => "CP2",
            ActivationCheckpoint::Integration => "CP3",
            ActivationCheckpoint::Operations => "CP4",
            ActivationCheckpoint::Performance => "CP5",
            ActivationCheckpoint::Product => "CP6",
        }
    }
}
