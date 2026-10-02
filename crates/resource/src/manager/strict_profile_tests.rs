//! Credential admission profiles: chosen at registration, latched per
//! attempt when a strict row first serves a managed call facade, visible
//! in the health snapshot, the erased view and the erased handle; a strict
//! manager refuses a row it cannot observe.

use std::sync::{Arc, atomic::Ordering};

use nebula_core::ScopeLevel;
use nebula_credential::{CredentialAvailability, CredentialAvailabilityObserver};

use super::{
    Manager,
    credential_reads::tests::{ScriptedObserver, seen},
    strict_fixtures::{
        ProjectionlessRow, StrictResident, UnboundRow, bind, context, credential_id, register,
        resident, strict_manager, tenant,
    },
};
use crate::{
    AcquireOptions, CredentialAdmissionProfile, ErrorKind, Provider, Resident, ResidentConfig,
};

fn observer() -> Arc<dyn CredentialAvailabilityObserver> {
    ScriptedObserver::answering(seen(1, 1, CredentialAvailability::Available))
}

fn view_profile<R: Provider>(manager: &Manager) -> CredentialAdmissionProfile {
    manager
        .get_row(&R::key(), &ScopeLevel::Global, &tenant())
        .expect("row")
        .credential_admission_profile()
}

#[test]
fn profile_names_are_stable() {
    assert_eq!(CredentialAdmissionProfile::Unbound.as_str(), "unbound");
    assert_eq!(
        CredentialAdmissionProfile::StrictPerAcquire.as_str(),
        "strict_per_acquire"
    );
    assert_eq!(
        CredentialAdmissionProfile::StrictPerAttempt.as_str(),
        "strict_per_attempt"
    );
    assert_eq!(
        CredentialAdmissionProfile::InterimRowGate.as_str(),
        "interim_row_gate"
    );
    assert!(CredentialAdmissionProfile::InterimRowGate.is_interim());
    assert!(!CredentialAdmissionProfile::StrictPerAcquire.is_interim());
    assert!(!CredentialAdmissionProfile::StrictPerAttempt.is_interim());
    assert!(!CredentialAdmissionProfile::Unbound.is_interim());
}

fn facade<R: Provider + crate::PinSlots>(manager: &Manager) -> crate::call::ResourceHandle<R> {
    manager
        .handle_for_identity::<R>(&context(), &tenant())
        .expect("row handle")
}

/// The profile as the health snapshot, the erased view and the erased
/// handle report it; all three must agree.
fn reported<R: Provider>(manager: &Manager) -> CredentialAdmissionProfile {
    let health = manager
        .health_check::<R>(&ScopeLevel::Global)
        .expect("health")
        .credential_admission;
    let handle = manager
        .lookup_any_for_slot_identity_structural(&R::key(), &ScopeLevel::Global, &tenant())
        .expect("row")
        .credential_admission_profile();
    assert_eq!(health, view_profile::<R>(manager));
    assert_eq!(health, handle);
    health
}

#[tokio::test]
async fn a_strict_row_reports_per_attempt_once_it_serves_a_facade() {
    let metrics = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = strict_manager(observer(), &metrics);
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);
    register(
        &manager,
        UnboundRow(Arc::default()),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register unbound");

    let guard = manager
        .acquire_for_identity::<StrictResident>(&context(), &AcquireOptions::default(), &tenant())
        .await
        .expect("acquire");
    assert_eq!(
        reported::<StrictResident>(&manager),
        CredentialAdmissionProfile::StrictPerAcquire,
        "a plain lease leaves the row per acquire"
    );
    drop(guard);
    let managed = facade::<StrictResident>(&manager);
    assert_eq!(
        reported::<StrictResident>(&manager),
        CredentialAdmissionProfile::StrictPerAttempt
    );
    drop(managed);
    assert_eq!(
        reported::<StrictResident>(&manager),
        CredentialAdmissionProfile::StrictPerAttempt,
        "latched for the row's life"
    );
    let view = manager
        .get_row(&StrictResident::key(), &ScopeLevel::Global, &tenant())
        .expect("row");
    assert!(format!("{view:?}").contains("credential_admission_profile: StrictPerAttempt"));

    // A slot-less row stays unbound through the facade.
    drop(facade::<UnboundRow>(&manager));
    assert_eq!(
        reported::<UnboundRow>(&manager),
        CredentialAdmissionProfile::Unbound
    );
}

#[tokio::test]
async fn an_interim_row_stays_on_the_row_gate_through_the_facade() {
    let manager = Manager::new();
    let resource = resident(&manager);
    bind(&resource.db, credential_id(), 1, 1);
    drop(facade::<StrictResident>(&manager));
    assert_eq!(
        reported::<StrictResident>(&manager),
        CredentialAdmissionProfile::InterimRowGate
    );
}

#[tokio::test]
async fn a_manager_with_an_observer_makes_credential_bound_rows_strict() {
    let metrics = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = strict_manager(observer(), &metrics);
    resident(&manager);
    register(
        &manager,
        UnboundRow(Arc::default()),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register unbound");

    let health = manager
        .health_check::<StrictResident>(&ScopeLevel::Global)
        .expect("health");
    assert_eq!(
        health.credential_admission,
        CredentialAdmissionProfile::StrictPerAcquire
    );
    assert_eq!(
        view_profile::<StrictResident>(&manager),
        CredentialAdmissionProfile::StrictPerAcquire
    );
    let view = manager
        .get_row(&StrictResident::key(), &ScopeLevel::Global, &tenant())
        .expect("row");
    assert!(
        format!("{view:?}").contains("credential_admission_profile: StrictPerAcquire"),
        "the erased view shows the profile"
    );
    // A slot-less row has nothing to read, whatever the manager.
    assert_eq!(
        view_profile::<UnboundRow>(&manager),
        CredentialAdmissionProfile::Unbound
    );
    assert!(!manager.interim_credential_warned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_manager_without_an_observer_keeps_rows_on_the_interim_row_gate() {
    let manager = Manager::new();
    resident(&manager);
    let health = manager
        .health_check::<StrictResident>(&ScopeLevel::Global)
        .expect("health");
    assert_eq!(
        health.credential_admission,
        CredentialAdmissionProfile::InterimRowGate
    );
    assert!(health.credential_admission.is_interim());
    assert!(
        manager.interim_credential_warned.load(Ordering::SeqCst),
        "the first interim row is warned about once per manager"
    );
    // A row the strict manager would refuse still registers here.
    register(
        &manager,
        ProjectionlessRow,
        Resident::new(ResidentConfig::default()),
    )
    .expect("an interim manager does not observe slots");
    assert_eq!(
        view_profile::<ProjectionlessRow>(&manager),
        CredentialAdmissionProfile::InterimRowGate
    );
}

#[tokio::test]
async fn a_strict_manager_refuses_a_row_whose_slot_it_cannot_observe() {
    let metrics = Arc::new(nebula_metrics::MetricsRegistry::new());
    let manager = strict_manager(observer(), &metrics);
    let error = register(
        &manager,
        ProjectionlessRow,
        Resident::new(ResidentConfig::default()),
    )
    .err()
    .expect("strict registration refuses an unobservable slot");
    assert_eq!(*error.kind(), ErrorKind::Permanent);
    assert!(error.to_string().contains("cannot observe slot `db`"));
    assert!(
        manager
            .get_row(&ProjectionlessRow::key(), &ScopeLevel::Global, &tenant())
            .is_none(),
        "nothing was registered"
    );
}
