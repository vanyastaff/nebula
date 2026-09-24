use super::*;

#[test]
fn public_lifecycle_state_has_a_stable_secret_free_wire_shape() {
    let retry_at = DateTime::parse_from_rfc3339("2026-09-24T01:02:03Z")
        .expect("fixed timestamp is valid")
        .with_timezone(&Utc);
    let cases = [
        (
            CredentialLifecycleState::Ready,
            serde_json::json!({ "status": "ready" }),
        ),
        (
            CredentialLifecycleState::RefreshDeferred { retry_at },
            serde_json::json!({
                "status": "refresh_deferred",
                "retry_at": "2026-09-24T01:02:03Z"
            }),
        ),
        (
            CredentialLifecycleState::RefreshBlocked,
            serde_json::json!({ "status": "refresh_blocked" }),
        ),
        (
            CredentialLifecycleState::ReauthRequired,
            serde_json::json!({ "status": "reauth_required" }),
        ),
        (
            CredentialLifecycleState::OperationInFlight {
                operation: CredentialLifecycleOperation::Revoke,
            },
            serde_json::json!({ "status": "operation_in_flight", "operation": "revoke" }),
        ),
        (
            CredentialLifecycleState::ReconciliationRequired {
                operation: Some(CredentialLifecycleOperation::Revoke),
            },
            serde_json::json!({ "status": "reconciliation_required", "operation": "revoke" }),
        ),
        (
            CredentialLifecycleState::ReconciliationRequired { operation: None },
            serde_json::json!({ "status": "reconciliation_required", "operation": null }),
        ),
    ];

    for (state, expected) in cases {
        let status = match state {
            CredentialLifecycleState::Ready => "ready",
            CredentialLifecycleState::RefreshDeferred { .. } => "refresh_deferred",
            CredentialLifecycleState::RefreshBlocked => "refresh_blocked",
            CredentialLifecycleState::ReauthRequired => "reauth_required",
            CredentialLifecycleState::OperationInFlight { .. } => "operation_in_flight",
            CredentialLifecycleState::ReconciliationRequired { .. } => "reconciliation_required",
        };
        let encoded = serde_json::to_value(state).expect("public lifecycle state serializes");
        assert_eq!(encoded, expected);
        assert_eq!(encoded["status"], status);
        let decoded: CredentialLifecycleState =
            serde_json::from_value(encoded).expect("public lifecycle state deserializes");
        assert_eq!(decoded, state);
    }

    assert!(
        serde_json::from_value::<CredentialLifecycleState>(
            serde_json::json!({ "status": "refreshing" })
        )
        .is_err(),
        "in-flight claim ownership is not a public lifecycle status"
    );
}

#[test]
fn static_secret_is_inert() {
    let p = CredentialPolicy::static_secret();
    assert!(!p.is_expiring());
    assert!(!p.is_auto_renewable());
    assert_eq!(p.refresh, RefreshStrategy::Static);
    assert_eq!(p.revoke, RevokeStrategy::None);
}

#[test]
fn refresh_pair_is_auto_renewable_and_expiring() {
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: Some(now + chrono::Duration::minutes(60)),
        lease: None,
        refresh: RefreshStrategy::RefreshToken,
        revoke: RevokeStrategy::HandleBased,
    };
    assert!(p.is_expiring());
    assert!(p.is_auto_renewable());
    assert!(!p.is_expired_at(now));
    assert!(p.is_expired_at(now + chrono::Duration::minutes(61)));
}

#[test]
fn lease_is_expiring_but_inline_expiry_decides_is_expired() {
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: None,
        lease: Some(LeaseRef {
            lease_id: "vault/lease/abc".to_owned(),
            lease_duration: Duration::from_hours(1),
            renewable: true,
            renew_until: None,
        }),
        refresh: RefreshStrategy::Lease,
        revoke: RevokeStrategy::HandleBased,
    };
    assert!(p.is_expiring());
    assert!(p.is_auto_renewable());
    // No inline expiry → never reported expired locally (server-tracked).
    assert!(!p.is_expired_at(now + chrono::Duration::days(365)));
}

#[test]
fn one_shot_lease_is_not_auto_renewable() {
    let p = CredentialPolicy {
        expires_at: None,
        lease: Some(LeaseRef {
            lease_id: "vault/lease/one-shot".to_owned(),
            lease_duration: Duration::from_hours(1),
            renewable: false,
            renew_until: None,
        }),
        refresh: RefreshStrategy::Lease,
        revoke: RevokeStrategy::HandleBased,
    };
    // Leased + non-renewable ⇒ must re-acquire, not auto-renew.
    assert!(!p.is_auto_renewable());
    assert!(p.is_expiring());
}

#[test]
fn reacquire_is_not_auto_renewable() {
    let p = CredentialPolicy {
        expires_at: None,
        lease: None,
        refresh: RefreshStrategy::ReAcquire {
            from: None,
            interactive: false,
        },
        revoke: RevokeStrategy::IssueTimePolicy,
    };
    assert!(!p.is_auto_renewable());
}

#[test]
fn strategies_round_trip_json() {
    for r in [
        RefreshStrategy::Static,
        RefreshStrategy::RefreshToken,
        RefreshStrategy::Lease,
        RefreshStrategy::ReAcquire {
            from: Some(SchemeId::new("oauth2")),
            interactive: true,
        },
        RefreshStrategy::ReMintLocal,
        RefreshStrategy::Watched,
    ] {
        let back: RefreshStrategy =
            serde_json::from_str(&serde_json::to_string(&r).expect("ser")).expect("de");
        assert_eq!(r, back);
    }
}

const HOUR: Duration = Duration::from_hours(1);
const FIVE_MIN: Duration = Duration::from_mins(5);

#[test]
fn static_within_floor_is_usable() {
    let now = Utc::now();
    let p = CredentialPolicy::static_secret();
    // Validated 30 min ago, floor 1h → still fresh.
    let last = now - chrono::Duration::minutes(30);
    assert_eq!(
        p.decide_refresh(last, now, FIVE_MIN, HOUR),
        Decision::Usable
    );
}

#[test]
fn static_past_floor_revalidates_not_usable() {
    // Owner ruling 2026-06-12: no "valid forever". A static API key past its
    // mandatory floor must be re-validated, never silently served forever.
    let now = Utc::now();
    let p = CredentialPolicy::static_secret();
    let last = now - chrono::Duration::hours(2); // floor 1h elapsed
    assert_eq!(
        p.decide_refresh(last, now, FIVE_MIN, HOUR),
        Decision::Revalidate
    );
}

#[test]
fn refresh_pair_past_expiry_refreshes() {
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: Some(now - chrono::Duration::minutes(1)),
        lease: None,
        refresh: RefreshStrategy::RefreshToken,
        revoke: RevokeStrategy::HandleBased,
    };
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Refresh
    );
}

#[test]
fn refresh_pair_in_early_window_refreshes() {
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: Some(now + chrono::Duration::minutes(2)), // inside 5-min buffer
        lease: None,
        refresh: RefreshStrategy::RefreshToken,
        revoke: RevokeStrategy::HandleBased,
    };
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Refresh
    );
}

#[test]
fn leased_renewable_no_inline_expiry_refreshes() {
    // A leased secret (expires_at: None) must NOT be
    // treated as fresh forever — it enters the renew path.
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: None,
        lease: Some(LeaseRef {
            lease_id: "vault/lease/abc".to_owned(),
            lease_duration: HOUR,
            renewable: true,
            renew_until: None,
        }),
        refresh: RefreshStrategy::Lease,
        revoke: RevokeStrategy::HandleBased,
    };
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Refresh
    );
}

#[test]
fn leased_one_shot_reacquires() {
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: None,
        lease: Some(LeaseRef {
            lease_id: "vault/lease/one-shot".to_owned(),
            lease_duration: HOUR,
            renewable: false,
            renew_until: None,
        }),
        refresh: RefreshStrategy::Lease,
        revoke: RevokeStrategy::HandleBased,
    };
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Reacquire
    );
}

#[test]
fn reacquire_past_expiry_reacquires() {
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: Some(now - chrono::Duration::seconds(1)),
        lease: None,
        refresh: RefreshStrategy::ReAcquire {
            from: None,
            interactive: false,
        },
        revoke: RevokeStrategy::IssueTimePolicy,
    };
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Reacquire
    );
}

#[test]
fn past_renew_until_horizon_reacquires_even_with_inline_expiry() {
    // A policy carrying BOTH an inline `expires_at` AND a lease whose hard
    // `renew_until` horizon has passed must re-acquire — the horizon wins
    // over the otherwise-auto-renewable expiry path. Regression: the horizon
    // was previously only checked in the lease-only (`expires_at: None`)
    // branch, so an expired-but-renewable inline-expiry policy refreshed past
    // its hard boundary.
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: Some(now - chrono::Duration::seconds(1)),
        lease: Some(LeaseRef {
            lease_id: "vault/lease/horizon".to_owned(),
            lease_duration: HOUR,
            renewable: true,
            renew_until: Some(now - chrono::Duration::seconds(1)),
        }),
        refresh: RefreshStrategy::Lease,
        revoke: RevokeStrategy::HandleBased,
    };
    // Auto-renewable (renewable lease) + expired, but past the hard horizon.
    assert!(p.is_auto_renewable());
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Reacquire
    );
}

#[test]
fn within_renew_until_horizon_still_refreshes() {
    // Same shape but the horizon is in the future → the renewable lease may
    // still refresh (horizon does not prematurely force re-acquisition).
    let now = Utc::now();
    let p = CredentialPolicy {
        expires_at: Some(now - chrono::Duration::seconds(1)),
        lease: Some(LeaseRef {
            lease_id: "vault/lease/horizon".to_owned(),
            lease_duration: HOUR,
            renewable: true,
            renew_until: Some(now + chrono::Duration::hours(24)),
        }),
        refresh: RefreshStrategy::Lease,
        revoke: RevokeStrategy::HandleBased,
    };
    assert_eq!(
        p.decide_refresh(now, now, FIVE_MIN, HOUR),
        Decision::Refresh
    );
}
