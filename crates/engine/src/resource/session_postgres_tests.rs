//! PostgreSQL acceptance of managed row sessions (PG1–PG8).
//!
//! A pooled resource whose instance is a real `PgConnection`, authenticated
//! with a `basic_auth` credential projected by the real runtime over the
//! encrypted SQLite store of [`SqliteFixture`], on a strict manager. Each
//! session is a real transaction on a real backend: a commit is visible to
//! an admin connection, a failed body leaves nothing, a backend terminated
//! during `COMMIT` is an unknown outcome, a flagged or rotated credential
//! never reaches the database, and nothing waits for quota while holding a
//! connection.
//!
//! Needs `DATABASE_URL` (a role allowed to `CREATE ROLE`); without it every
//! test returns early, unless `NEBULA_REQUIRE_POSTGRES` is set. No test
//! sleeps to synchronize: advisory locks gate the backend and every wait on
//! the database is a bounded poll.

use std::{str::FromStr as _, time::Duration};

use nebula_credential::{BasicAuthCredential, CredentialGuard, scheme::IdentityPassword};
use nebula_resource::{
    PoolConfig, Pooled, TeardownCx,
    call::{
        Cost, ManagedRow, OpError, SentState, SessionClosed, SessionEnd, SessionProvider,
        SessionSpec,
    },
    rate_limit::Verdict,
    topology::pooled::{PoolProvider, RecycleDecision},
};
use sqlx::{
    Connection as _,
    postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions},
};

use super::*;

const PG_KIND: &str = "activation.pg";
const PASSWORD_1: &str = "row-password-1";
const PASSWORD_2: &str = "row-password-2";

// ── the resource ─────────────────────────────────────────────────────────

/// Where the connections go; the role and password come from the slot.
#[derive(Clone, Debug, serde::Deserialize, nebula_schema::Schema)]
struct PgConfig {
    #[serde(default)]
    #[field(default = "")]
    host: String,
    #[serde(default)]
    #[field(default = "")]
    port: String,
    #[serde(default)]
    #[field(default = "")]
    database: String,
    #[serde(default)]
    #[field(default = "")]
    schema: String,
}

impl ResourceConfig for PgConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.host.is_empty() || self.database.is_empty() || self.schema.is_empty() {
            return Err(ResourceError::permanent(
                "host, database and schema are required",
            ));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&self.host, &self.port, &self.database, &self.schema).hash(&mut hasher);
        hasher.finish()
    }
}

type PasswordGuard = CredentialGuard<IdentityPassword>;

/// One PostgreSQL connection per pooled instance, logged in with the `auth`
/// slot's role and password.
#[derive(Clone)]
struct PgRow {
    auth: Arc<nebula_resource::SlotCell<PasswordGuard>>,
}

impl PgRow {
    fn new() -> Self {
        Self {
            auth: Arc::new(nebula_resource::SlotCell::empty()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for PgRow {
    type Config = PgConfig;
    type Instance = PgConnection;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("activation.pg")
    }

    async fn create(
        &self,
        config: &PgConfig,
        _ctx: &ResourceContext,
    ) -> Result<PgConnection, ResourceError> {
        let login = self
            .auth
            .load()
            .ok_or_else(|| ResourceError::permanent("postgres password slot unbound"))?;
        let port = config
            .port
            .parse::<u16>()
            .map_err(|_| ResourceError::permanent("postgres port is not a number"))?;
        let options = PgConnectOptions::new_without_pgpass()
            .host(&config.host)
            .port(port)
            .database(&config.database)
            .username(login.identity())
            .password(login.password().expose_secret())
            // A backend notices its client went away while it runs a query.
            .options([
                ("search_path", config.schema.as_str()),
                ("client_connection_check_interval", "50ms"),
            ]);
        PgConnection::connect_with(&options)
            .await
            .map_err(|_| ResourceError::transient("postgres connect failed"))
    }

    async fn destroy(&self, instance: PgConnection, cx: TeardownCx) -> Result<(), ResourceError> {
        let budget = cx
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        match tokio::time::timeout(budget, instance.close()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(ResourceError::transient("postgres close failed")),
            // Dropped past the deadline: the socket closes with it.
            Err(_elapsed) => Ok(()),
        }
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("activation.pg"),
            String::new(),
        )
    }
}

impl DeclaresDependencies for PgRow {
    fn dependencies() -> Dependencies {
        Dependencies::new().slot_field(SlotField {
            slot_key: AUTH_SLOT,
            default_id: AUTH_SLOT,
            kind: SlotKind::Credential {
                type_id: std::any::TypeId::of::<BasicAuthCredential>(),
                type_name: "basic_auth",
                key: CredentialKey::new(BasicAuthCredential::KEY).expect("valid credential key"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

impl nebula_resource::HasCredentialSlots for PgRow {
    fn credential_slot_epoch(&self) -> u64 {
        self.auth.generation()
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &[AUTH_SLOT]
    }

    fn supports_credential_slot_projection(&self, slot: &str) -> bool {
        slot == AUTH_SLOT
    }

    fn credential_slot_metadata(
        &self,
        slot: &str,
    ) -> Option<nebula_credential::CredentialGuardMetadata> {
        (slot == AUTH_SLOT)
            .then(|| self.auth.projection_metadata())
            .flatten()
    }

    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<nebula_credential::CredentialGuardMetadata>)> {
        (slot == AUTH_SLOT).then(|| self.auth.projection_snapshot())
    }

    fn install_credential_slot_at_generation(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
        expected_generation: u64,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        let (metadata, guard) = password(slot, guard)?;
        self.auth
            .install_projected_at_generation(expected_generation, metadata, guard)
    }

    fn fence_credential_slot_at_generation(
        &self,
        slot: &str,
        expected_generation: u64,
        fence: &mut dyn FnMut(),
    ) -> Result<(), nebula_resource::SlotInstallError> {
        if slot != AUTH_SLOT {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        self.auth
            .fence_projection_at_generation(expected_generation, fence)
    }

    fn install_credential_slot(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        let (metadata, guard) = password(slot, guard)?;
        self.auth.install_projected(metadata, guard)
    }

    fn revoke_credential_slot(
        &self,
        slot: &str,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        if slot != AUTH_SLOT {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        Ok(self.auth.revoke())
    }
}

fn password(
    slot: &str,
    guard: nebula_credential::ErasedCredentialGuard,
) -> Result<
    (
        nebula_credential::CredentialGuardMetadata,
        Arc<PasswordGuard>,
    ),
    nebula_resource::SlotInstallError,
> {
    if slot != AUTH_SLOT {
        return Err(nebula_resource::SlotInstallError::UnknownSlot);
    }
    let metadata = guard.metadata().clone();
    let guard = guard
        .into_typed::<IdentityPassword>()
        .map_err(|_| nebula_resource::SlotInstallError::CredentialTypeMismatch)?;
    Ok((metadata, Arc::new(guard)))
}

impl PinSlots for PgRow {
    type Pinned = Option<(u64, Arc<PasswordGuard>)>;

    fn pin_slots(&self) -> Self::Pinned {
        self.auth.load_material_versioned()
    }
}

impl PoolProvider for PgRow {
    /// A committed or rolled-back connection is clean: keep it (a
    /// credentialed row is dropped on release by default).
    async fn recycle(
        &self,
        _instance: &PgConnection,
        _metrics: &nebula_resource::InstanceMetrics,
    ) -> Result<RecycleDecision, ResourceError> {
        Ok(RecycleDecision::Keep)
    }
}

/// Connection-bound sessions: a transaction on the checked-out connection.
impl SessionProvider for PgRow {
    type Session<'c> = sqlx::Transaction<'c, sqlx::Postgres>;

    async fn open<'c>(
        &'c self,
        connection: &'c mut PgConnection,
        _slots: &'c Self::Pinned,
    ) -> Result<Self::Session<'c>, OpError> {
        connection.begin().await.map_err(|_| {
            OpError::new(
                nebula_resource::ErrorKind::Transient,
                "postgres begin failed",
            )
        })
    }

    async fn close<'c>(&'c self, session: Self::Session<'c>, end: SessionEnd) -> SessionClosed {
        if end != SessionEnd::Commit {
            // An uncommitted transaction applied nothing, whatever the
            // rollback's own fate.
            let _rolled_back = session.rollback().await;
            return SessionClosed::RolledBack { refused: None };
        }
        match session.commit().await {
            Ok(()) => SessionClosed::Committed,
            Err(error) if server_refused(&error) => SessionClosed::RolledBack {
                refused: Some(OpError::new(
                    nebula_resource::ErrorKind::Permanent,
                    "postgres refused the commit",
                )),
            },
            Err(_) => SessionClosed::Unknown(OpError::new(
                nebula_resource::ErrorKind::Transient,
                "postgres connection lost during the commit",
            )),
        }
    }
}

/// Whether the server answered the commit with a refusal (a constraint, a
/// serialization failure) rather than losing the connection: SQLSTATE
/// classes 08 (connection exception) and 57 (operator intervention) say
/// nothing about whether the commit applied.
fn server_refused(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database) => database
            .code()
            .is_some_and(|code| !code.starts_with("08") && !code.starts_with("57")),
        _ => false,
    }
}

fn query_failed(_: sqlx::Error) -> OpError {
    OpError::new(
        nebula_resource::ErrorKind::Transient,
        "postgres query failed",
    )
}

// ── the fixture ──────────────────────────────────────────────────────────

/// A strict [`SqliteFixture`] with a stored, activated `PgRow` bound to a
/// fresh role, in a private schema.
struct Pg {
    fixture: SqliteFixture,
    admin: PgPool,
    role: String,
    credential_id: CredentialId,
    resource_id: ResourceId,
    key: ResourceKey,
    activated: ActivatedResource,
}

/// The fixture, or `None` without `DATABASE_URL`.
async fn setup() -> Option<Pg> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
            "NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL is not"
        );
        return None;
    };
    let admin = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("the admin connects");
    let role = format!(
        "nebula_row_{}",
        ulid::Ulid::new().to_string().to_lowercase()
    );
    for statement in [
        format!("CREATE ROLE {role} LOGIN PASSWORD '{PASSWORD_1}'"),
        format!("CREATE SCHEMA {role}"),
        format!("CREATE TABLE {role}.ledger (id integer PRIMARY KEY)"),
        format!("GRANT USAGE ON SCHEMA {role} TO {role}"),
        format!("GRANT SELECT, INSERT ON {role}.ledger TO {role}"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&admin)
            .await
            .expect("the private schema is set up");
    }

    let mut fixture = SqliteFixture::strict().await;
    fixture
        .registrars
        .insert(
            PG_KIND,
            Arc::new(KindActivator::<PgRow, _, _>::new(
                PgRow::new,
                nebula_resource::topology::fixed(|| {
                    Pooled::<PgRow>::new(
                        PoolConfig {
                            min_size: 0,
                            max_size: 1,
                            idle_timeout: None,
                            max_lifetime: None,
                            ..PoolConfig::default()
                        },
                        0,
                    )
                }),
            )),
        )
        .expect("the postgres resource admits");
    let credential_id = CredentialId::new();
    fixture
        .store
        .create(
            &CredentialSelector::new(CredentialOwner::from_scope(&fixture.scope), credential_id),
            CredentialCreate::new(
                BasicAuthCredential::KEY.to_owned(),
                SecretBytes::new(login_state(&role, PASSWORD_1)),
                <IdentityPassword as CredentialState>::KIND.to_owned(),
                <IdentityPassword as CredentialState>::VERSION,
                Some("pg".to_owned()),
                None,
                false,
                display("pg"),
            ),
        )
        .await
        .expect("the login credential is stored");
    let options = PgConnectOptions::from_str(&url).expect("DATABASE_URL parses");
    let (resource_id, key) = fixture
        .store_row_as(
            PG_KIND,
            serde_json::json!({
                "host": options.get_host(),
                "port": options.get_port().to_string(),
                "database": options.get_database().unwrap_or("postgres"),
                "schema": role,
            }),
            credential_id,
        )
        .await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("the postgres row activates");
    Some(Pg {
        fixture,
        admin,
        role,
        credential_id,
        resource_id,
        key,
        activated,
    })
}

/// The stored `identity_password` state for `role` / `password`.
fn login_state(role: &str, password: &str) -> Vec<u8> {
    let state = IdentityPassword::new(role, SecretString::new(password.to_owned()));
    nebula_credential::serde_secret::expose_for_serialization(|| serde_json::to_vec(&state))
        .expect("the login state encodes")
}

impl Pg {
    /// Drops the private schema and role (their backends first).
    async fn cleanup(self) {
        let _shutdown = self
            .fixture
            .manager
            .graceful_shutdown(nebula_resource::ShutdownConfig::default())
            .await;
        for statement in [
            format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = '{}'",
                self.role
            ),
            format!("DROP SCHEMA {} CASCADE", self.role),
            format!("DROP ROLE {}", self.role),
        ] {
            let _dropped = sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&self.admin)
                .await;
        }
        self.admin.close().await;
    }

    fn selector(&self) -> CredentialSelector {
        CredentialSelector::new(
            CredentialOwner::from_scope(&self.fixture.scope),
            self.credential_id,
        )
    }

    /// The facade of the activated row.
    fn row(&self, activated: &ActivatedResource) -> ManagedRow<PgRow> {
        let workspace = WorkspaceId::parse(&self.fixture.scope.workspace_id).expect("workspace id");
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                workspace_id: Some(workspace),
                ..Default::default()
            },
            CancellationToken::new(),
        );
        self.fixture
            .manager
            .managed_row_for_identity::<PgRow>(&ctx, &activated.slot_identity)
            .expect("the row facade")
    }

    /// Commits `insert into ledger values (id)`; yields the backend pid.
    fn insert(&self, row: &ManagedRow<PgRow>, id: i32) -> nebula_resource::call::Unit<i32> {
        row.session(SessionSpec::new(Cost::ONE), move |tx, _cx| {
            Box::pin(async move {
                sqlx::query("INSERT INTO ledger (id) VALUES ($1)")
                    .bind(id)
                    .execute(&mut **tx)
                    .await
                    .map_err(query_failed)?;
                sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(query_failed)
            })
        })
    }

    async fn has_row(&self, id: i32) -> bool {
        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {}.ledger WHERE id = $1",
            self.role
        )))
        .bind(id)
        .fetch_one(&self.admin)
        .await
        .expect("the admin reads the ledger")
            == 1
    }

    /// Backends logged in as the row's role, with their states.
    async fn backends(&self) -> Vec<(i32, Option<String>)> {
        sqlx::query_as("SELECT pid, state FROM pg_stat_activity WHERE usename = $1")
            .bind(&self.role)
            .fetch_all(&self.admin)
            .await
            .expect("the admin reads pg_stat_activity")
    }

    /// Polls `done` against the database, boundedly.
    async fn until<F, Fut>(&self, what: &str, mut done: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        for _ in 0..500 {
            if done().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out waiting until {what}");
    }

    async fn until_gone(&self, pid: i32) {
        self.until("the backend is gone", || async {
            !self.backends().await.iter().any(|(other, _)| *other == pid)
        })
        .await;
    }

    fn idle_saturation(&self) -> f32 {
        self.fixture
            .manager
            .get_row(
                &self.activated.resource_key,
                &self.activated.scope,
                &self.activated.slot_identity,
            )
            .expect("the row is registered")
            .admission_load()
            .expect("a pool reports its load")
            .saturation
    }

    /// Flags the login for reauthentication (the backend advances its use
    /// revision); `false` clears the flag at the same material.
    async fn flag_reauth(&self, required: bool) {
        let head = self
            .fixture
            .store
            .get_head(&self.selector())
            .await
            .expect("head");
        self.fixture
            .store
            .replace(
                &self.selector(),
                CredentialReplacement::new(
                    head.version(),
                    Some("pg".to_owned()),
                    required,
                    display("pg"),
                    CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
                ),
            )
            .await
            .expect("the reauthentication flag is written");
    }

    /// Stores `password` as new login material (the material epoch
    /// advances).
    async fn rotate_login(&self, password: &str) {
        let head = self
            .fixture
            .store
            .get_head(&self.selector())
            .await
            .expect("head");
        self.fixture
            .store
            .replace(
                &self.selector(),
                CredentialReplacement::new(
                    head.version(),
                    Some("pg".to_owned()),
                    false,
                    display("pg"),
                    CredentialMaterialTransition::advance(
                        nebula_storage_port::MaterialUpdate::Replace(
                            nebula_storage_port::CredentialMaterial::new(
                                SecretBytes::new(login_state(&self.role, password)),
                                <IdentityPassword as CredentialState>::KIND.to_owned(),
                                <IdentityPassword as CredentialState>::VERSION,
                                None,
                            ),
                        ),
                    ),
                ),
            )
            .await
            .expect("the new login is stored");
    }
}

// ── PG1–PG8 ──────────────────────────────────────────────────────────────

/// PG1: a committed session is visible to another connection, and the next
/// session reuses the same backend.
#[tokio::test]
async fn a_committed_session_is_visible_and_its_backend_reused() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);

    let first = pg.insert(&row, 1).await.expect("committed");
    assert!(pg.has_row(1).await, "visible to the admin");
    let second = pg.insert(&row, 2).await.expect("committed");
    assert_eq!(first, second, "the same backend served both sessions");
    assert_eq!(pg.backends().await.len(), 1);
    drop(row);
    pg.cleanup().await;
}

/// PG2: a failing body rolls back: nothing applied, nothing sent.
#[tokio::test]
async fn a_failed_body_leaves_nothing_behind() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);

    let error = row
        .session(SessionSpec::new(Cost::ONE), |tx, _cx| {
            Box::pin(async move {
                sqlx::query("INSERT INTO ledger (id) VALUES (2)")
                    .execute(&mut **tx)
                    .await
                    .map_err(query_failed)?;
                Err::<(), _>(OpError::new(
                    nebula_resource::ErrorKind::Permanent,
                    "the body gave up",
                ))
            })
        })
        .await
        .expect_err("rolled back");
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(!pg.has_row(2).await, "nothing applied");
    pg.insert(&row, 3).await.expect("the connection is reused");
    drop(row);
    pg.cleanup().await;
}

/// PG3: a backend terminated while its `COMMIT` waits on a deferred
/// constraint is an unknown outcome; the connection is destroyed and the
/// write did not apply.
#[tokio::test]
async fn a_backend_lost_during_commit_is_an_unknown_outcome() {
    let Some(pg) = setup().await else { return };
    let lock = i64::try_from(ulid::Ulid::new().random() & 0x7fff_ffff).expect("fits");
    for statement in [
        format!(
            "CREATE FUNCTION {role}.gate() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN PERFORM pg_advisory_lock({lock}); PERFORM pg_advisory_unlock({lock}); \
             RETURN NULL; END $$",
            role = pg.role
        ),
        format!(
            "CREATE CONSTRAINT TRIGGER gate AFTER INSERT ON {role}.ledger \
             DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION {role}.gate()",
            role = pg.role
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&pg.admin)
            .await
            .expect("the commit gate is installed");
    }
    let mut holder = pg.admin.acquire().await.expect("a lock holder");
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock)
        .execute(&mut *holder)
        .await
        .expect("the admin holds the gate");
    let row = pg.row(&pg.activated);
    let mut events = pg.fixture.manager.subscribe_events();

    let unit = tokio::spawn(pg.insert(&row, 3));
    let blocked = || async {
        sqlx::query_scalar::<_, i32>(
            "SELECT l.pid FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
             WHERE l.locktype = 'advisory' AND NOT l.granted AND a.usename = $1",
        )
        .bind(&pg.role)
        .fetch_optional(&pg.admin)
        .await
        .expect("the admin reads pg_locks")
    };
    pg.until("the commit waits on the gate", || async {
        blocked().await.is_some()
    })
    .await;
    let pid = blocked().await.expect("the committing backend");
    sqlx::query("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .execute(&pg.admin)
        .await
        .expect("the backend is terminated");

    let error = unit.await.expect("joined").expect_err("unknown");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(
        *nebula_resource::Error::from(error).kind(),
        nebula_resource::ErrorKind::OutcomeUnknown
    );
    let mut unknown = 0;
    while let Some(event) = events.try_recv() {
        if matches!(event, ResourceEvent::UnitOutcomeUnknown { .. }) {
            unknown += 1;
        }
    }
    assert_eq!(unknown, 1);
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock)
        .execute(&mut *holder)
        .await
        .expect("the gate opens");
    drop(holder);
    pg.until_gone(pid).await;
    assert!(!pg.has_row(3).await, "the commit never applied");
    let next = pg.insert(&row, 4).await.expect("a fresh connection");
    assert_ne!(next, pid, "the lost connection was not reused");
    drop(row);
    pg.cleanup().await;
}

/// PG4: a login flagged for reauthentication refuses the session before any
/// connection is made; clearing the flag serves again.
#[tokio::test]
async fn a_login_flagged_for_reauthentication_never_reaches_the_database() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);

    pg.flag_reauth(true).await;
    let error = pg.insert(&row, 1).await.expect_err("refused");
    assert_eq!(
        *error.kind(),
        nebula_resource::ErrorKind::CredentialUnavailable {
            reason: CredentialUnavailableReason::ReauthRequired
        }
    );
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(pg.backends().await.is_empty(), "no backend was started");
    assert!(pg.fixture.suspended(&pg.activated));

    // A row facade refuses a suspended row without reading; the next
    // activation observes the cleared flag and reopens it.
    pg.flag_reauth(false).await;
    let reactivated = pg
        .fixture
        .activate(pg.resource_id, &pg.key)
        .await
        .expect("activation reopens the row");
    assert!(!pg.fixture.suspended(&reactivated));
    let row = pg.row(&reactivated);
    pg.insert(&row, 1).await.expect("serves again");
    assert!(pg.has_row(1).await);
    drop(row);
    pg.cleanup().await;
}

/// PG5: a rotated password refuses as rebinding until activation installs
/// it; then a new backend logs in with the new password and the old one is
/// closed.
#[tokio::test]
async fn a_rotated_password_reaches_a_new_backend_after_activation() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);
    let old = pg
        .insert(&row, 1)
        .await
        .expect("committed on the first login");

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER ROLE {} PASSWORD '{PASSWORD_2}'",
        pg.role
    )))
    .execute(&pg.admin)
    .await
    .expect("the database password rotates");
    pg.rotate_login(PASSWORD_2).await;
    let error = pg.insert(&row, 2).await.expect_err("not installed yet");
    assert_eq!(
        *error.kind(),
        nebula_resource::ErrorKind::CredentialUnavailable {
            reason: CredentialUnavailableReason::Rebinding
        }
    );
    assert_eq!(error.sent(), SentState::NotSent);

    let reactivated = pg
        .fixture
        .activate(pg.resource_id, &pg.key)
        .await
        .expect("activation installs the new login");
    let row = pg.row(&reactivated);
    let new = pg.insert(&row, 2).await.expect("served on the new login");
    assert_ne!(new, old, "a new backend, logged in with the new password");
    pg.until_gone(old).await;
    assert!(pg.has_row(2).await);
    drop(row);
    pg.cleanup().await;
}

/// PG6: a session waiting out a provider pause holds no connection; a
/// cancel ends it unsent.
#[tokio::test]
async fn a_session_waiting_for_quota_holds_no_connection() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);
    pg.insert(&row, 1).await.expect("warms one connection");
    row.submit(Throttle).await.expect("the pause is reported");

    let mut unit = pg.insert(&row, 2);
    assert!(futures::poll!(&mut unit).is_pending());
    tokio::task::yield_now().await;
    assert!(
        futures::poll!(&mut unit).is_pending(),
        "waiting out the pause"
    );
    assert!(
        pg.idle_saturation().abs() < f32::EPSILON,
        "nothing checked out"
    );
    let backends = pg.backends().await;
    assert_eq!(backends.len(), 1);
    assert!(
        backends
            .iter()
            .all(|(_, state)| state.as_deref() == Some("idle")),
        "the only backend is idle: {backends:?}"
    );

    unit.cancel();
    let error = unit.await.expect_err("cancelled");
    assert_eq!(*error.kind(), nebula_resource::ErrorKind::Cancelled);
    assert_eq!(error.sent(), SentState::NotSent);
    assert!(!pg.has_row(2).await);
    drop(row);
    pg.cleanup().await;
}

/// Reports an hour-long provider pause on the row's quota.
struct Throttle;

impl Operation<PgRow> for Throttle {
    type Output = ();
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OpCx<'_, PgRow>) -> Result<(), OpError> {
        let attempt = cx.attempt(Cost::FREE).await?;
        attempt
            .report(Verdict::Throttled {
                retry_after: Some(Duration::from_hours(1)),
            })
            .await;
        attempt.settle(SentState::NotSent);
        Ok(())
    }
}

/// PG7: a session past its deadline mid-query is an unknown outcome; its
/// backend goes away and the write does not apply.
#[tokio::test]
async fn a_session_past_its_deadline_is_cut_off_and_applies_nothing() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);
    let pid = pg.insert(&row, 1).await.expect("warms one connection");

    let started = std::time::Instant::now();
    let error = row
        .session(SessionSpec::new(Cost::ONE), |tx, _cx| {
            Box::pin(async move {
                sqlx::query("INSERT INTO ledger (id) VALUES (7)")
                    .execute(&mut **tx)
                    .await
                    .map_err(query_failed)?;
                sqlx::query("SELECT pg_sleep(10)")
                    .execute(&mut **tx)
                    .await
                    .map_err(query_failed)?;
                Ok(())
            })
        })
        .with_deadline(std::time::Instant::now() + Duration::from_millis(200))
        .await
        .expect_err("cut off at the deadline");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(
        *nebula_resource::Error::from(error).kind(),
        nebula_resource::ErrorKind::OutcomeUnknown
    );
    pg.until_gone(pid).await;
    assert!(
        !pg.has_row(7).await,
        "the transaction died with its backend"
    );
    drop(row);
    pg.cleanup().await;
}

/// PG8: shutting the manager down closes every pooled connection.
#[tokio::test]
async fn a_shutdown_closes_every_backend() {
    let Some(pg) = setup().await else { return };
    let row = pg.row(&pg.activated);
    pg.insert(&row, 1).await.expect("warms one connection");
    assert_eq!(pg.backends().await.len(), 1);

    drop(row);
    let _report = pg
        .fixture
        .manager
        .graceful_shutdown(nebula_resource::ShutdownConfig::default())
        .await
        .expect("shuts down");
    pg.until("every backend is closed", || async {
        pg.backends().await.is_empty()
    })
    .await;
    pg.cleanup().await;
}
