//! Lock the `Send` bounds on `RefreshCoordinator::refresh_coalesced`
//! (sub-spec §3.6 / review feedback I2).
//!
//! `refresh_coalesced` runs the user's closure under
//! `tokio::time::timeout` and awaits the predicate from the spawn'd
//! backoff loop, so both the closure future and the predicate future
//! cross task boundaries. Without explicit `Send` bounds the trait
//! solver tolerated `!Send` futures and the diagnostic only surfaced
//! at the call site — frames away from the coordinator definition.
//!
//! This probe locks the bound by checking that:
//!
//! 1. A real call into `refresh_coalesced` with `Send` futures returns a `Send` future itself — the
//!    test only compiles when `T: Send`, `Fut: Send`, `PFut: Send`, etc. are part of the public
//!    contract.
//! 2. The hand-rolled `assert_send` helper is invoked on the returned future so a regression would
//!    fail at compile time, not at runtime.
//!
//! Adding the bounds is the contract; this file makes regressions
//! visible the moment they happen rather than several frames away.

use std::{sync::Arc, time::Duration};

use nebula_engine::credential::refresh::{RefreshCoordConfig, RefreshCoordinator, RefreshError};
use nebula_storage::credential::{InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId};

fn assert_send<T: Send>(_: &T) {}

/// Compile-time probe: a `refresh_coalesced` call with `Send` futures
/// returns a `Send` future (i.e. the coordinator does not auto-trait
/// downgrade `T`/`Fut`/`PFut` to non-`Send`).
#[tokio::test]
async fn refresh_coalesced_returns_send_future() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&repo),
            ReplicaId::new("send-probe"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid"),
    );

    let cid = nebula_core::CredentialId::new();
    let pred = |_id: &nebula_core::CredentialId| async { true };
    let do_refresh = |_claim| async move {
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok::<u32, RefreshError>(7u32)
    };

    let fut = coord.refresh_coalesced(&cid, pred, do_refresh);
    // The compile-time assertion. If a regression weakens the bounds
    // (e.g. `Fut: Future<Output = ...>` without `+ Send`), this line
    // refuses to compile — the diagnostic now points at the bound, not
    // at the user closure.
    assert_send(&fut);

    let result = fut.await;
    assert_eq!(result.unwrap(), 7);
}
