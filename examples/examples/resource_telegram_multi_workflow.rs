//! Telegram bot shared across 10 simulated workflows.
//!
//! Headline cross-workflow scenario — proves a single `Resource::create`
//! invocation deduplicates 10 concurrent acquires of the same
//! Resident-topology resource at the same scope. Mirrors the integration
//! test `cross_workflow_resource_sharing` in
//! `crates/engine/tests/resource_integration.rs`, but as an end-user
//! runnable demonstration.
//!
//! ## What this shows
//!
//! - **Resident topology** — one `TelegramBotInner` shared across N callers. Every lease is
//!   `Arc::ptr_eq` to every other.
//! - **Cross-workflow dedupe** — 10 spawned tasks all acquire the same resource at
//!   `ScopeLevel::Organization`; the manager collapses them to a single `Resource::create`.
//! - **Counter assertion** — the `create_counter` ends at exactly `1`.
//! - **EventSource direct usage** — the `EventTrigger` DX wrapper is deferred. We drive the fake
//!   update stream by directly broadcasting to a `tokio::sync::broadcast` channel and show how an
//!   `EventSource::recv` consumer reads from it. This is the canonical pattern until the wrapper
//!   ships.
//!
//! ## Run
//!
//! ```shell
//! cargo run -p nebula-examples --example resource_telegram_multi_workflow
//! ```

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::Resident;
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResidentConfig, ResourceContext,
    dedup::SlotIdentity,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadata},
    topology::resident::ResidentProvider,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

// ─── Fake bot resource ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct TelegramConfig {
    /// What we'd register the bot under in Telegram (display only).
    handle: String,
}

nebula_schema::impl_empty_has_schema!(TelegramConfig);

impl ResourceConfig for TelegramConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.handle.is_empty() {
            Err(ResourceError::permanent("handle must not be empty"))
        } else {
            Ok(())
        }
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.handle.hash(&mut h);
        h.finish()
    }
}

/// Synthetic update payload emitted by the fake polling loop.
#[derive(Debug, Clone)]
struct TelegramUpdate {
    update_id: i64,
    chat_id: i64,
    text: String,
}

/// What [`Resource::create`] returns. All callers share an `Arc` of this.
struct TelegramBotInner {
    instance_id: u64,
    /// Outbound publisher for updates. The polling loop holds the sender;
    /// `EventSource::subscribe` returns a fresh receiver.
    update_tx: broadcast::Sender<TelegramUpdate>,
}

#[derive(Debug, Clone)]
struct TelegramError(String);

impl std::fmt::Display for TelegramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TelegramError {}

impl From<TelegramError> for ResourceError {
    fn from(e: TelegramError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone)]
struct TelegramBot {
    create_counter: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}

impl TelegramBot {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for TelegramBot {
    type Config = TelegramConfig;
    type Instance = Arc<TelegramBotInner>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("demo.telegram.bot")
    }

    async fn create(
        &self,
        config: &TelegramConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<TelegramBotInner>, ResourceError> {
        let counter = Arc::clone(&self.create_counter);
        let handle = config.handle.clone();
        // Yield once so concurrent acquires can interleave more
        // aggressively — exposes any missing serialization in the
        // double-checked create path.
        tokio::task::yield_now().await;
        let id = counter.fetch_add(1, Ordering::SeqCst);
        let (update_tx, _) = broadcast::channel(64);
        tracing::info!(instance_id = id, handle = %handle, "creating shared Telegram bot");
        Ok(Arc::new(TelegramBotInner {
            instance_id: id,
            update_tx,
        }))
    }

    async fn destroy(
        &self,
        runtime: Arc<TelegramBotInner>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), ResourceError> {
        tracing::info!(instance_id = runtime.instance_id, "destroying Telegram bot");
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl nebula_resource::HasCredentialSlots for TelegramBot {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl ResidentProvider for TelegramBot {
    fn is_alive_sync(&self, _runtime: &Arc<TelegramBotInner>) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

// ─── Wiring + main ─────────────────────────────────────────────────────────

fn ctx_for_org(org: OrgId) -> ResourceContext {
    let scope = Scope {
        org_id: Some(org),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    println!("=== Telegram bot × 10 workflows ===\n");

    // 1. Register the bot at Organization scope. Every workflow under this org will share the same
    //    physical bot client.
    let manager = Arc::new(Manager::new());
    let bot = TelegramBot::new();
    let create_counter = Arc::clone(&bot.create_counter);
    let resident_runtime = Resident::<TelegramBot>::new(ResidentConfig::default());
    let org = OrgId::new();

    manager.register(RegistrationSpec {
        resource: bot,
        config: TelegramConfig {
            handle: "@nebula_demo_bot".into(),
        },
        scope: ScopeLevel::Organization(org),
        slot_identity: SlotIdentity::Unbound,
        topology: resident_runtime,
        recovery_gate: None,
    })?;
    println!("[1] TelegramBot registered at Organization scope (org={org:?})");

    // 2. Spawn 10 simulated workflows. Each acquires the bot, sends one "message" (just records
    //    that it would have sent), and holds the lease for a beat to keep concurrent acquires
    //    alive.
    println!("\n[2] Spawning 10 workflows that all acquire the same bot:");
    let mut handles = Vec::with_capacity(10);
    for workflow_id in 0..10 {
        let mgr = Arc::clone(&manager);
        handles.push(tokio::spawn(async move {
            let ctx = ctx_for_org(org);
            let lease = mgr
                .acquire_resident::<TelegramBot>(&ctx, &AcquireOptions::default())
                .await
                .expect("acquire");
            // Tiny work simulation: every workflow sends one outbound message.
            tokio::time::sleep(Duration::from_millis(20)).await;
            // Clone the inner `Arc<TelegramBotInner>` out of the guard so the
            // join_all collector can hand back a uniform shape; the guard
            // itself is dropped when this task returns, but the Arc keeps
            // the runtime alive for the assertion below.
            let inner: Arc<TelegramBotInner> = (*lease).clone();
            (workflow_id, inner)
        }));
    }

    let workflow_results: Vec<(i32, Arc<TelegramBotInner>)> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("task did not panic"))
        .collect();

    // 3. Assertions: every lease points at the same Arc, and create_counter == 1.
    let total_creates = create_counter.load(Ordering::SeqCst);
    println!("\n[3] Cross-workflow dedupe assertions:");
    println!("    Resource::create invocations: {total_creates} (expected: 1)");
    assert_eq!(
        total_creates, 1,
        "10 concurrent acquires of one Resident-scope resource must collapse to a single create"
    );

    let first_arc = &workflow_results[0].1;
    for (wid, lease) in &workflow_results[1..] {
        assert!(
            Arc::ptr_eq(first_arc, lease),
            "workflow {wid} got a different Arc — dedupe regressed"
        );
    }
    println!(
        "    Arc::ptr_eq across all 10 leases: PASS (instance_id = {})",
        first_arc.instance_id,
    );

    // 4. EventSource pattern (per ADR-0045 — direct broadcast subscribe/recv, no EventTrigger DX
    //    wrapper yet). We use the still-held first lease to publish a fake update stream and have a
    //    consumer receive it.
    println!("\n[4] EventSource direct usage (ADR-0045 pattern):");
    let mut subscriber = first_arc.update_tx.subscribe();

    // Background "polling loop" — synthesize 3 fake updates over 60ms.
    let publisher = first_arc.update_tx.clone();
    let publisher_handle = tokio::spawn(async move {
        for i in 0..3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let update = TelegramUpdate {
                update_id: 1000 + i,
                chat_id: 4242,
                text: format!("hello from update #{i}"),
            };
            let _ = publisher.send(update);
        }
    });

    // Consumer — equivalent to `EventSource::recv(&mut Subscription)` on the
    // resource trait; a TriggerAction would do the same in production.
    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_secs(1), subscriber.recv()).await {
            Ok(Ok(update)) => {
                println!(
                    "    consumer received: update_id={} chat={} text={:?}",
                    update.update_id, update.chat_id, update.text,
                );
            },
            Ok(Err(e)) => {
                println!("    consumer error: {e}");
                break;
            },
            Err(_) => {
                println!("    consumer timed out");
                break;
            },
        }
    }
    publisher_handle.await.ok();

    // 5. Drop all leases — Resident topology keeps the runtime alive in the manager regardless.
    //    Print final stats and shut down.
    drop(workflow_results);
    println!("\n[5] All workflow leases dropped");

    manager.shutdown();
    println!("\n=== Done ===");
    Ok(())
}
