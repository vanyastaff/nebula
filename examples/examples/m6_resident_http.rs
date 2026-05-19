//! M6.3 Phase 10 — Resident HTTP client with OAuth-style credential refresh.
//!
//! Models a Google Sheets-style HTTP integration:
//!
//! - **Resident topology** — a single `GoogleSheetsClient` (HTTP client + cached access token)
//!   shared across the workflow. Cloning is `Arc` refcount.
//! - **OAuth2 refresh** — the cached access token expires; the client detects a 401-like response
//!   and exchanges a refresh token for a new access token transparently. This mirrors the
//!   `Refreshable` capability from `nebula-credential` but stays self-contained.
//! - **No real network** — `MockTokenServer` and `MockSheetsApi` simulate the OAuth provider and
//!   Sheets endpoints in-process so the example runs without external services.
//!
//! ## Run
//!
//! ```shell
//! cargo run -p nebula-examples --example m6_resident_http
//! ```
//!
//! ## Pattern explanation
//!
//! 1. The credential carries the **refresh token** + client id/secret.
//! 2. The Resident `GoogleSheetsClient` holds a **cached access token** (initially obtained at
//!    `Resource::create`).
//! 3. A `ReadSheet` call uses the cached token; if the mock API returns 401 (because we set a short
//!    expiry to force this), the client invokes `Refreshable::refresh` to mint a new access token
//!    and retries once.
//! 4. The example explicitly forces three refreshes to demonstrate the pattern — production code
//!    refreshes only when the cached token has actually expired.

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResidentConfig, ResourceContext,
    dedup::SlotIdentity,
    error::Error as ResourceError,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

// ─── OAuth2 credential (mock) ──────────────────────────────────────────────

/// In production, this would derive `Credential` and implement `Refreshable`
/// from `nebula-credential`. The mock keeps the shape but skips the storage
/// + projection layers so the example focuses on the refresh flow.
#[derive(Clone)]
#[allow(
    dead_code,
    reason = "client_secret models the SecretString slot — never read in mock, never logged in production"
)]
struct OAuth2Credential {
    /// Long-lived refresh token. Production keeps this in a `SecretString`.
    refresh_token: String,
    client_id: String,
    /// Production stores this in a `SecretString`; never logged.
    client_secret: String,
    /// Token endpoint URL (e.g. `https://oauth2.googleapis.com/token`).
    token_url: String,
}

/// Short-lived access token issued by the OAuth provider.
#[derive(Clone, Debug)]
struct AccessToken {
    value: String,
    expires_at: Instant,
}

impl AccessToken {
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Mock OAuth2 token server. In production this is the IdP; here it just
/// mints monotonically-increasing tokens with a short expiry so the example
/// can force refreshes deterministically.
#[derive(Clone)]
struct MockTokenServer {
    issued: Arc<AtomicU64>,
    /// Lifetime of every issued token. The example sets this to ~50ms to
    /// force refreshes inside the demo's runtime.
    token_lifetime: Duration,
}

impl MockTokenServer {
    fn new(token_lifetime: Duration) -> Self {
        Self {
            issued: Arc::new(AtomicU64::new(0)),
            token_lifetime,
        }
    }

    /// Simulates `POST <token_url> grant_type=refresh_token`. Production
    /// would route through `reqwest`; the mock just bumps a counter and
    /// hands back a fresh token tied to the refresh-token argument.
    fn refresh(&self, cred: &OAuth2Credential) -> AccessToken {
        let n = self.issued.fetch_add(1, Ordering::SeqCst);
        tracing::info!(
            issuance = n,
            client_id = %cred.client_id,
            token_url = %cred.token_url,
            "MockTokenServer: refresh_token exchanged for new access token",
        );
        AccessToken {
            value: format!(
                "at_{}_{n}",
                &cred.refresh_token[..8.min(cred.refresh_token.len())]
            ),
            expires_at: Instant::now() + self.token_lifetime,
        }
    }

    fn issuances(&self) -> u64 {
        self.issued.load(Ordering::Acquire)
    }
}

// Global mock — registered once, used both by `Resource::create` and by the
// resource's internal refresh path.
fn token_server() -> &'static MockTokenServer {
    static SERVER: OnceLock<MockTokenServer> = OnceLock::new();
    SERVER.get_or_init(|| MockTokenServer::new(Duration::from_millis(50)))
}

// ─── Mock Google Sheets API ────────────────────────────────────────────────

/// Result of `MockSheetsApi::values_get`.
enum SheetsResponse {
    Ok(Vec<Vec<String>>),
    /// The token is expired or the server rotated; client should refresh + retry.
    Unauthorized,
}

#[derive(Clone)]
struct MockSheetsApi {
    /// Simulates server-side validation: the server holds the latest token
    /// it issued. Older tokens are 401'd. In real Google APIs the same
    /// effect comes from token expiry.
    last_valid_token: Arc<RwLock<Option<String>>>,
    /// Counter for diagnostic output.
    requests: Arc<AtomicU64>,
}

impl MockSheetsApi {
    fn new() -> Self {
        Self {
            last_valid_token: Arc::new(RwLock::new(None)),
            requests: Arc::new(AtomicU64::new(0)),
        }
    }

    fn values_get(&self, bearer: &str) -> SheetsResponse {
        self.requests.fetch_add(1, Ordering::SeqCst);
        match &*self.last_valid_token.read() {
            Some(latest) if latest == bearer => SheetsResponse::Ok(vec![
                vec!["A1".into(), "B1".into()],
                vec!["A2".into(), "B2".into()],
            ]),
            _ => SheetsResponse::Unauthorized,
        }
    }

    fn update_known_token(&self, token: &str) {
        *self.last_valid_token.write() = Some(token.to_owned());
    }
}

fn sheets_api() -> &'static MockSheetsApi {
    static API: OnceLock<MockSheetsApi> = OnceLock::new();
    API.get_or_init(MockSheetsApi::new)
}

// ─── Resident resource ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct GoogleSheetsConfig {
    /// Namespace tag for tracing — would carry timeout, retries, rate-limit
    /// caps in production.
    application: String,
}

nebula_schema::impl_empty_has_schema!(GoogleSheetsConfig);

impl ResourceConfig for GoogleSheetsConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.application.is_empty() {
            Err(ResourceError::permanent("application must not be empty"))
        } else {
            Ok(())
        }
    }

    // Resident: single instance, config change forces destroy+recreate.
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Debug, Clone)]
struct SheetsError(String);

impl std::fmt::Display for SheetsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SheetsError {}

impl From<SheetsError> for ResourceError {
    fn from(e: SheetsError) -> Self {
        ResourceError::transient(e.0)
    }
}

/// What `Resource::create` returns. `Resident` topology requires this to be
/// `Clone + Send + Sync + 'static` — `Arc` makes that cheap.
#[derive(Clone)]
struct GoogleSheetsClient {
    /// The OAuth credential — needed for refresh on token expiry.
    cred: Arc<OAuth2Credential>,
    /// The current access token. `RwLock` lets `read` overlap with
    /// non-refreshing callers.
    access_token: Arc<RwLock<AccessToken>>,
}

impl GoogleSheetsClient {
    /// Performs `values.get(spreadsheet_id, range)` with one transparent
    /// refresh on 401. Mirrors what a `reqwest::Client` wrapper would do.
    fn read_sheet(&self, spreadsheet: &str, range: &str) -> Result<Vec<Vec<String>>, SheetsError> {
        // Try with the cached token first.
        let token = self.access_token.read().clone();
        if token.is_expired() {
            tracing::info!(
                spreadsheet,
                range,
                "ReadSheet: cached token expired locally — refreshing"
            );
        } else {
            match sheets_api().values_get(&token.value) {
                SheetsResponse::Ok(rows) => {
                    tracing::debug!(spreadsheet, range, "ReadSheet: cached token accepted");
                    return Ok(rows);
                },
                SheetsResponse::Unauthorized => {
                    tracing::warn!(
                        spreadsheet,
                        range,
                        "ReadSheet: server rotated key — forcing refresh"
                    );
                },
            }
        }

        // Refresh + retry once. In production this is the `Refreshable`
        // capability sub-trait on the credential; here we drive it inline.
        let new_token = token_server().refresh(&self.cred);
        // Make the mock server accept the new token.
        sheets_api().update_known_token(&new_token.value);
        *self.access_token.write() = new_token.clone();

        match sheets_api().values_get(&new_token.value) {
            SheetsResponse::Ok(rows) => Ok(rows),
            SheetsResponse::Unauthorized => Err(SheetsError(
                "still unauthorized after refresh — credential is wrong?".into(),
            )),
        }
    }
}

#[derive(Clone)]
struct GoogleSheets {
    /// Holds the OAuth credential for `create`. In production this comes
    /// from the credential slot resolution (ADR-0043).
    cred: Arc<OAuth2Credential>,
    /// Observable counter for the assertion at the end of `main`.
    create_counter: Arc<AtomicU64>,
}

impl GoogleSheets {
    fn new(cred: OAuth2Credential) -> Self {
        Self {
            cred: Arc::new(cred),
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Resource for GoogleSheets {
    type Config = GoogleSheetsConfig;
    type Runtime = GoogleSheetsClient;
    type Lease = GoogleSheetsClient;
    type Error = SheetsError;

    fn key() -> ResourceKey {
        resource_key!("demo.google.sheets")
    }

    fn create(
        &self,
        config: &GoogleSheetsConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<GoogleSheetsClient, SheetsError>> + Send {
        let cred = Arc::clone(&self.cred);
        let counter = Arc::clone(&self.create_counter);
        let app = config.application.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            tracing::info!(application = %app, "Resource::create — minting initial OAuth access token");
            // Initial token exchange.
            let access_token = token_server().refresh(&cred);
            sheets_api().update_known_token(&access_token.value);
            Ok(GoogleSheetsClient {
                cred,
                access_token: Arc::new(RwLock::new(access_token)),
            })
        }
    }

    async fn destroy(&self, _runtime: GoogleSheetsClient) -> Result<(), SheetsError> {
        tracing::info!("Resource::destroy — releasing GoogleSheetsClient");
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for GoogleSheets {
    fn is_alive_sync(&self, _runtime: &GoogleSheetsClient) -> bool {
        true
    }
}

// ─── Wiring + main ─────────────────────────────────────────────────────────

fn ctx_for_demo() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== M6.3 Phase 10 — Resident HTTP + OAuth refresh ===\n");

    // 1. Configure the credential. Production code reads this from the credential store via the
    //    slot-resolution pipeline; here we construct it inline.
    let cred = OAuth2Credential {
        refresh_token: "rt_long_lived_secret_42".into(),
        client_id: "nebula-demo-client".into(),
        client_secret: "client-secret-redacted".into(),
        token_url: "https://oauth2.example.com/token".into(),
    };

    // 2. Register the Resident resource at global scope.
    let manager = Arc::new(Manager::new());
    let sheets = GoogleSheets::new(cred);
    let create_counter = Arc::clone(&sheets.create_counter);
    let resident_runtime = ResidentRuntime::<GoogleSheets>::new(ResidentConfig::default());

    manager.register(RegistrationSpec {
        resource: sheets,
        config: GoogleSheetsConfig {
            application: "nebula-sheets-demo".into(),
        },
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Resident(resident_runtime),
        acquire: Manager::erased_acquire_resident_for::<GoogleSheets>(),
        resilience: None,
        recovery_gate: None,
    })?;
    println!("[1] GoogleSheets resource registered (Resident topology, Global scope)");
    println!("    Initial token issuance from Resource::create (count = 1)");

    // 3. Acquire the client and read a sheet 3 times. Between calls we sleep long enough for the
    //    access token to expire (token_server() lifetime is 50ms), forcing a refresh on the second
    //    and third reads.
    let ctx = ctx_for_demo();
    let lease = manager
        .acquire_resident::<GoogleSheets>(&ctx, &AcquireOptions::default())
        .await?;
    println!("\n[2] Three sequential ReadSheet calls (refresh forced by 50ms token TTL):");

    for run in 0..3 {
        let result = lease.read_sheet("spreadsheet_xyz", "Sheet1!A1:B2");
        match result {
            Ok(rows) => println!("  run={run} → ok, rows={}", rows.len()),
            Err(e) => println!("  run={run} → err: {e}"),
        }
        // Sleep past the token's 50ms lifetime so the next call sees an
        // expired cache and triggers refresh.
        tokio::time::sleep(Duration::from_millis(70)).await;
    }

    // 4. Assertions.
    let total_creates = create_counter.load(Ordering::Acquire);
    let token_issuances = token_server().issuances();
    println!("\n[3] Counters:");
    println!("    Resource::create invocations: {total_creates} (Resident → 1)");
    println!("    Token-server refresh exchanges: {token_issuances}");
    assert_eq!(
        total_creates, 1,
        "Resident topology must call Resource::create exactly once"
    );
    assert!(
        token_issuances >= 3,
        "expected at least 3 token issuances (initial + 2 refreshes); got {token_issuances}",
    );

    drop(lease);
    manager.shutdown();
    println!("\n=== Done ===");
    Ok(())
}
