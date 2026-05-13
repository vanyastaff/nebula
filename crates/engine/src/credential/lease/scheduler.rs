//! Background scheduler driving lease renewals and revocations.
//!
//! One scheduler task per [`LeaseLifecycle`](super::LeaseLifecycle).
//! Owns the registry and a min-heap of `(next_renew_at, LeaseToken)`,
//! drains commands from an `mpsc`, and wakes itself with
//! [`tokio::time::sleep_until`] on the earliest renewal. Cancellation
//! drops all outstanding leases with a `Shutdown` reason.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use nebula_core::accessor::MetricsEmitter;
use nebula_credential::{
    CredentialId, CredentialMetrics, LeaseEvent, LeaseExpiryReason, LeaseHandle, LeasedProvider,
    ProviderError, ProviderResolution,
};
use nebula_eventbus::EventBus;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use super::policy::RenewalPolicy;
use super::registry::{LeaseEntry, LeaseToken};

/// Configuration for the lease lifecycle.
#[derive(Debug, Clone)]
pub struct LeaseLifecycleConfig {
    /// Renewal policy. Defaults to the Vault Agent recommendation (see
    /// [`RenewalPolicy`]).
    pub policy: RenewalPolicy,
    /// Hard upper bound on a single `renew` / `revoke` call. Protects
    /// the single scheduler task from a hung backend monopolising the
    /// loop and starving other due leases. Mirrors the 30s framework
    /// timeout used elsewhere in `nebula-engine` (see
    /// `CredentialResolver::perform_refresh`). On elapsed the call is
    /// treated as `ProviderError::Unavailable` — the standard backoff
    /// schedule applies.
    pub provider_call_timeout: Duration,
}

impl Default for LeaseLifecycleConfig {
    fn default() -> Self {
        Self {
            policy: RenewalPolicy::default(),
            provider_call_timeout: Duration::from_secs(30),
        }
    }
}

/// Command messages the public handle sends to the scheduler task.
pub(super) enum Command {
    Track {
        provider: Arc<dyn LeasedProvider>,
        lease: LeaseHandle,
        credential_id: Option<CredentialId>,
        reply: oneshot::Sender<LeaseToken>,
    },
    Revoke {
        token: LeaseToken,
        reply: oneshot::Sender<RevokeOutcome>,
    },
    RevokeForCredential {
        credential_id: CredentialId,
        reply: oneshot::Sender<usize>,
    },
    Snapshot {
        reply: oneshot::Sender<usize>,
    },
}

/// Outcome of a `Revoke` command, distinct from a generic error type so
/// the public surface can downgrade "unknown token" to a typed result.
pub(super) enum RevokeOutcome {
    /// Lease was found, revoke completed successfully.
    Revoked,
    /// Lease was found, but the provider returned an error. Carries the
    /// typed [`ProviderError`] so callers preserve variant semantics
    /// (transient `Unavailable` vs. auth `AccessDenied`) and the source
    /// chain. The revoke event already fired before this is returned.
    ProviderFailed(ProviderError),
    /// No lease found under that token — likely already expired or
    /// previously revoked.
    Unknown,
}

/// Inputs assembled by [`LeaseLifecycle::spawn`](super::LeaseLifecycle::spawn).
pub(super) struct SchedulerInputs {
    pub(super) config: LeaseLifecycleConfig,
    pub(super) commands: mpsc::UnboundedReceiver<Command>,
    pub(super) lease_bus: Option<Arc<EventBus<LeaseEvent>>>,
    pub(super) metrics: Option<Arc<dyn MetricsEmitter>>,
    pub(super) shutdown: CancellationToken,
}

/// The scheduler's mutable state.
struct Scheduler {
    inputs: SchedulerInputs,
    registry: HashMap<LeaseToken, LeaseEntry>,
    heap: BinaryHeap<Reverse<(Instant, LeaseToken)>>,
    next_id: u64,
}

/// Entry point for the spawned task.
pub(super) async fn run(inputs: SchedulerInputs) {
    let mut scheduler = Scheduler {
        inputs,
        registry: HashMap::new(),
        heap: BinaryHeap::new(),
        next_id: 0,
    };
    scheduler.run_loop().await;
}

impl Scheduler {
    async fn run_loop(&mut self) {
        loop {
            // Compute next wake-up: earliest non-stale heap entry, or
            // far future if registry is empty. The heap is a soft
            // schedule — entries can be stale when an earlier revoke
            // removed the token; we skip stale heads at wake.
            let next_wake = self.peek_next_wake();
            let sleep = match next_wake {
                Some(when) => tokio::time::sleep_until(when),
                None => tokio::time::sleep(Duration::from_hours(1)),
            };
            tokio::pin!(sleep);

            tokio::select! {
                () = self.inputs.shutdown.cancelled() => {
                    self.drain_on_shutdown();
                    return;
                }
                cmd = self.inputs.commands.recv() => {
                    if let Some(c) = cmd {
                        self.handle_command(c).await;
                    } else {
                        // Sender dropped — no more commands will arrive.
                        // Treat as graceful shutdown.
                        self.drain_on_shutdown();
                        return;
                    }
                }
                () = &mut sleep => {
                    if next_wake.is_some() {
                        self.tick().await;
                    }
                }
            }
        }
    }

    fn peek_next_wake(&self) -> Option<Instant> {
        self.heap.peek().map(|Reverse((when, _))| *when)
    }

    async fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Track {
                provider,
                lease,
                credential_id,
                reply,
            } => {
                let token = self.allocate_token();
                let policy = &self.inputs.config.policy;
                // Compute the renewal point from the lease's *issue time*,
                // not from "now": a resolution may sit in a cache for a
                // non-trivial fraction of its TTL before reaching the
                // lifecycle, and scheduling a full fresh window from now
                // would let the lease expire before the first renew. If
                // the lease is already past its renewal point, fire
                // immediately (the heap pop loop tolerates `next_renew_at
                // <= now`).
                let renew_after_from_issue = policy.renew_after(lease.ttl);
                let aged = chrono::Utc::now().signed_duration_since(lease.issued_at);
                let aged_std = aged.to_std().unwrap_or(Duration::ZERO);
                let remaining = renew_after_from_issue.saturating_sub(aged_std);
                let next_renew_at = Instant::now() + remaining;
                let provider_name = provider.provider_name().to_owned();
                let lease_id = lease.lease_id.clone();

                let entry = LeaseEntry {
                    lease,
                    provider,
                    credential_id,
                    next_renew_at,
                    consecutive_failures: 0,
                };
                self.registry.insert(token, entry);
                self.heap.push(Reverse((next_renew_at, token)));
                self.update_active_gauge();

                tracing::debug!(
                    target: "nebula_engine::credential::lease",
                    provider = %provider_name,
                    lease_id = %lease_id,
                    renew_in_secs = remaining.as_secs(),
                    aged_secs = aged_std.as_secs(),
                    "lease tracked; renewal scheduled"
                );
                // Best-effort reply — caller may have dropped the future.
                let _ = reply.send(token);
            },
            Command::Revoke { token, reply } => {
                let outcome = self.do_revoke(token).await;
                let _ = reply.send(outcome);
            },
            Command::RevokeForCredential {
                credential_id,
                reply,
            } => {
                let tokens: Vec<LeaseToken> = self
                    .registry
                    .iter()
                    .filter_map(|(t, e)| (e.credential_id == Some(credential_id)).then_some(*t))
                    .collect();
                let mut count = 0;
                for token in tokens {
                    match self.do_revoke(token).await {
                        RevokeOutcome::Revoked => count += 1,
                        // Revoke failures during rotation are best-effort
                        // — already logged + emitted as event; do not
                        // block the rotation pipeline.
                        RevokeOutcome::ProviderFailed(_) | RevokeOutcome::Unknown => {},
                    }
                }
                let _ = reply.send(count);
            },
            Command::Snapshot { reply } => {
                let _ = reply.send(self.registry.len());
            },
        }
    }

    fn allocate_token(&mut self) -> LeaseToken {
        let t = LeaseToken(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        t
    }

    /// Fired when the timer for the heap head elapses. Process every
    /// due entry; the head may be stale if the entry was revoked or
    /// rescheduled — skip such heads.
    async fn tick(&mut self) {
        let now = Instant::now();
        while let Some(Reverse((when, token))) = self.heap.peek().copied() {
            if when > now {
                break;
            }
            // Pop the stale or current head.
            let _ = self.heap.pop();
            // If the entry was already revoked / dropped, skip.
            let Some(entry) = self.registry.get(&token) else {
                continue;
            };
            // If the schedule was bumped after this heap entry was
            // queued (e.g. a successful renew rescheduled further out),
            // skip this stale firing.
            if entry.next_renew_at > now {
                continue;
            }
            self.attempt_renew(token).await;
        }
    }

    /// Run one renewal attempt against the lease's provider. Re-schedules
    /// on success, backs off on transient failure, drops on permanent.
    async fn attempt_renew(&mut self, token: LeaseToken) {
        let (provider, lease, credential_id, attempt) = {
            let Some(entry) = self.registry.get(&token) else {
                return;
            };
            (
                Arc::clone(&entry.provider),
                entry.lease.clone(),
                entry.credential_id,
                entry.consecutive_failures,
            )
        };
        let provider_name = provider.provider_name().to_owned();
        let lease_id = lease.lease_id.clone();
        let span = tracing::info_span!(
            target: "nebula_engine::credential::lease",
            "lease.renew",
            provider = %provider_name,
            lease_id = %lease_id,
        );

        // Bounded provider call: instrument via `Instrument` so the
        // span propagates correctly across the `.await` (an `Entered`
        // guard would not survive task scheduling on a multi-thread
        // runtime). Timeout maps to `Unavailable` so the standard
        // backoff schedule absorbs a hung backend instead of stalling
        // the lifecycle.
        let timeout = self.inputs.config.provider_call_timeout;
        let result =
            match tokio::time::timeout(timeout, provider.renew(&lease).instrument(span)).await {
                Ok(r) => r,
                Err(_) => Err(ProviderError::Unavailable {
                    reason: format!("provider renew timed out after {}s", timeout.as_secs()),
                }),
            };
        match result {
            Ok(resolution) => {
                self.on_renew_success(token, &provider_name, &lease_id, credential_id, &resolution);
            },
            Err(err) => {
                self.on_renew_failure(
                    token,
                    &provider_name,
                    &lease_id,
                    credential_id,
                    attempt,
                    err,
                );
            },
        }
    }

    fn on_renew_success(
        &mut self,
        token: LeaseToken,
        provider_name: &str,
        lease_id: &str,
        credential_id: Option<CredentialId>,
        resolution: &ProviderResolution,
    ) {
        // Pull the refreshed TTL from the renewed lease (preferred) or
        // the envelope-level ttl as a fallback. Vault's `/sys/leases/renew`
        // returns the new `lease_duration` in both places via
        // `ProviderResolution::with_lease`.
        let new_ttl = resolution
            .lease
            .as_ref()
            .map(|l| l.ttl)
            .or(resolution.ttl)
            .unwrap_or(Duration::ZERO);

        // If the provider reports zero TTL on renew, treat that as a
        // signal that no further renew is meaningful (some backends
        // return zero to indicate "non-renewable"). Distinct from
        // `NotFoundUpstream` (the lease is gone) — the renew succeeded
        // but the grant is exhausted.
        let renew_after = self.inputs.config.policy.renew_after(new_ttl);
        if renew_after.is_zero() {
            self.drop_lease(
                token,
                lease_id,
                provider_name,
                credential_id,
                LeaseExpiryReason::NonRenewable,
            );
            return;
        }

        // Use the refreshed lease_id (when the backend rotated it) for
        // the renewed event so subscribers can correlate against the
        // identifier the lifecycle now tracks.
        let event_lease_id = resolution
            .lease
            .as_ref()
            .map_or_else(|| lease_id.to_owned(), |l| l.lease_id.clone());
        let next_renew_at = Instant::now() + renew_after;
        if let Some(entry) = self.registry.get_mut(&token) {
            // Update the stored lease so future renew calls carry the
            // refreshed metadata (lease_id may change on some backends).
            if let Some(refreshed) = resolution.lease.clone() {
                entry.lease = refreshed;
            } else {
                entry.lease.ttl = new_ttl;
                entry.lease.issued_at = chrono::Utc::now();
            }
            entry.next_renew_at = next_renew_at;
            entry.consecutive_failures = 0;
        }
        self.heap.push(Reverse((next_renew_at, token)));

        tracing::debug!(
            target: "nebula_engine::credential::lease",
            provider = provider_name,
            lease_id = %event_lease_id,
            new_ttl_secs = new_ttl.as_secs(),
            "lease renewed; renewal rescheduled"
        );
        self.emit_lease_event(LeaseEvent::LeaseRenewed {
            credential_id,
            lease_id: event_lease_id,
            provider: std::borrow::Cow::Owned(provider_name.to_owned()),
            new_ttl,
        });
        self.emit_counter(
            CredentialMetrics::DYNAMIC_LEASE_RENEWED_TOTAL,
            &[
                (CredentialMetrics::LABEL_OUTCOME, "success"),
                (CredentialMetrics::LABEL_PROVIDER, provider_name),
            ],
        );
    }

    fn on_renew_failure(
        &mut self,
        token: LeaseToken,
        provider_name: &str,
        lease_id: &str,
        credential_id: Option<CredentialId>,
        attempt: u32,
        err: ProviderError,
    ) {
        let reason = err.to_string();
        let permanent = matches!(
            err,
            ProviderError::NotFound { .. } | ProviderError::AccessDenied { .. }
        );

        tracing::warn!(
            target: "nebula_engine::credential::lease",
            provider = provider_name,
            lease_id = lease_id,
            attempt,
            error = %reason,
            permanent,
            "lease renewal failed"
        );
        self.emit_lease_event(LeaseEvent::LeaseRenewalFailed {
            credential_id,
            lease_id: lease_id.to_owned(),
            provider: std::borrow::Cow::Owned(provider_name.to_owned()),
            reason,
        });
        // `DYNAMIC_LEASE_RENEWED_TOTAL` counts only successful renewals
        // — failures are surfaced exclusively through
        // `DYNAMIC_LEASE_RENEW_FAILED_TOTAL` (labelled by reason). This
        // keeps dashboards built on the obvious metric name correct and
        // avoids double-counting failed attempts across both counters.
        self.emit_counter(
            CredentialMetrics::DYNAMIC_LEASE_RENEW_FAILED_TOTAL,
            &[
                (
                    CredentialMetrics::LABEL_FAILURE_REASON,
                    failure_reason_label(&err),
                ),
                (CredentialMetrics::LABEL_PROVIDER, provider_name),
            ],
        );

        if permanent {
            self.drop_lease(
                token,
                lease_id,
                provider_name,
                credential_id,
                LeaseExpiryReason::NotFoundUpstream,
            );
            return;
        }

        // Transient: consult the backoff schedule. Exhaustion drops the lease.
        let next_attempt = attempt + 1;
        match self.inputs.config.policy.backoff_for(attempt) {
            Some(wait) => {
                let next_renew_at = Instant::now() + wait;
                if let Some(entry) = self.registry.get_mut(&token) {
                    entry.consecutive_failures = next_attempt;
                    entry.next_renew_at = next_renew_at;
                }
                self.heap.push(Reverse((next_renew_at, token)));
            },
            None => {
                self.drop_lease(
                    token,
                    lease_id,
                    provider_name,
                    credential_id,
                    LeaseExpiryReason::RenewalFailed,
                );
            },
        }
    }

    /// Remove an entry without attempting upstream revoke. Emits a
    /// `LeaseExpired` event and decrements the gauge.
    fn drop_lease(
        &mut self,
        token: LeaseToken,
        lease_id: &str,
        provider_name: &str,
        credential_id: Option<CredentialId>,
        reason: LeaseExpiryReason,
    ) {
        if self.registry.remove(&token).is_none() {
            return;
        }
        self.update_active_gauge();
        tracing::info!(
            target: "nebula_engine::credential::lease",
            provider = provider_name,
            lease_id = lease_id,
            ?reason,
            "lease dropped from lifecycle"
        );
        self.emit_lease_event(LeaseEvent::LeaseExpired {
            credential_id,
            lease_id: lease_id.to_owned(),
            provider: std::borrow::Cow::Owned(provider_name.to_owned()),
            reason,
        });
    }

    async fn do_revoke(&mut self, token: LeaseToken) -> RevokeOutcome {
        let Some(entry) = self.registry.remove(&token) else {
            return RevokeOutcome::Unknown;
        };
        let provider_name = entry.provider.provider_name().to_owned();
        let lease_id = entry.lease.lease_id.clone();
        let credential_id = entry.credential_id;

        let span = tracing::info_span!(
            target: "nebula_engine::credential::lease",
            "lease.revoke",
            provider = %provider_name,
            lease_id = %lease_id,
        );

        // Same bounded-call pattern as renew: a hung backend must not
        // monopolise the scheduler. Span propagated via `Instrument`
        // because an `Entered` guard would not survive task scheduling.
        let timeout = self.inputs.config.provider_call_timeout;
        let result = match tokio::time::timeout(
            timeout,
            entry.provider.revoke(&entry.lease).instrument(span),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => Err(ProviderError::Unavailable {
                reason: format!("provider revoke timed out after {}s", timeout.as_secs()),
            }),
        };
        self.update_active_gauge();
        match result {
            Ok(_) => {
                tracing::debug!(
                    target: "nebula_engine::credential::lease",
                    provider = %provider_name,
                    lease_id = %lease_id,
                    "lease revoked upstream"
                );
                self.emit_lease_event(LeaseEvent::LeaseRevoked {
                    credential_id,
                    lease_id,
                    provider: std::borrow::Cow::Owned(provider_name.clone()),
                });
                self.emit_counter(
                    CredentialMetrics::DYNAMIC_LEASE_REVOKED_TOTAL,
                    &[
                        (CredentialMetrics::LABEL_OUTCOME, "success"),
                        (CredentialMetrics::LABEL_PROVIDER, &provider_name),
                    ],
                );
                RevokeOutcome::Revoked
            },
            Err(err) => {
                let reason = err.to_string();
                tracing::warn!(
                    target: "nebula_engine::credential::lease",
                    provider = %provider_name,
                    lease_id = %lease_id,
                    error = %reason,
                    "lease revoke failed; lease removed from registry anyway"
                );
                self.emit_lease_event(LeaseEvent::LeaseRevocationFailed {
                    credential_id,
                    lease_id,
                    provider: std::borrow::Cow::Owned(provider_name.clone()),
                    reason,
                });
                self.emit_counter(
                    CredentialMetrics::DYNAMIC_LEASE_REVOKED_TOTAL,
                    &[
                        (CredentialMetrics::LABEL_OUTCOME, "failure"),
                        (CredentialMetrics::LABEL_PROVIDER, &provider_name),
                    ],
                );
                RevokeOutcome::ProviderFailed(err)
            },
        }
    }

    /// Cancellation path: drop every outstanding lease with `Shutdown`
    /// reason. We do NOT attempt upstream revoke — shutdown can be
    /// triggered by process termination and we must not block on I/O.
    fn drain_on_shutdown(&mut self) {
        let entries: Vec<_> = self.registry.drain().collect();
        self.update_active_gauge();
        for (_, entry) in entries {
            let provider_name = entry.provider.provider_name().to_owned();
            tracing::info!(
                target: "nebula_engine::credential::lease",
                provider = %provider_name,
                lease_id = %entry.lease.lease_id,
                "lease lifecycle shutdown — lease left to expire upstream"
            );
            self.emit_lease_event(LeaseEvent::LeaseExpired {
                credential_id: entry.credential_id,
                lease_id: entry.lease.lease_id.clone(),
                provider: std::borrow::Cow::Owned(provider_name),
                reason: LeaseExpiryReason::Shutdown,
            });
        }
    }

    fn emit_lease_event(&self, event: LeaseEvent) {
        if let Some(bus) = &self.inputs.lease_bus {
            // Emission is best-effort — a no-subscriber bus or a
            // lagged broadcast channel must not interrupt the lifecycle
            // loop. Surface failures at debug so a quiet event bus is
            // observable when subscribers are expected.
            let outcome = bus.emit(event);
            if !outcome.is_sent() {
                tracing::debug!(
                    target: "nebula_engine::credential::lease",
                    ?outcome,
                    "lease event bus emit did not reach any subscriber"
                );
            }
        }
    }

    fn emit_counter(&self, name: &str, labels: &[(&str, &str)]) {
        if let Some(m) = &self.inputs.metrics {
            m.counter(name, 1, labels);
        }
    }

    fn update_active_gauge(&self) {
        if let Some(m) = &self.inputs.metrics {
            #[allow(clippy::cast_precision_loss)] // counts beyond f64 mantissa are not realistic
            let value = self.registry.len() as f64;
            m.gauge(CredentialMetrics::DYNAMIC_LEASE_ACTIVE, value, &[]);
        }
    }
}

fn failure_reason_label(err: &ProviderError) -> &'static str {
    match err {
        ProviderError::NotFound { .. } => "not_found",
        ProviderError::Unavailable { .. } => "unavailable",
        ProviderError::AccessDenied { .. } => "access_denied",
        ProviderError::Backend(_) => "backend",
        // `ProviderError` is `#[non_exhaustive]`; future variants will
        // label as `other` until classified explicitly.
        _ => "other",
    }
}
