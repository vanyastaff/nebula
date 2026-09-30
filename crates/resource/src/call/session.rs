//! Sessions: several native provider calls on one checked-out connection,
//! admitted and settled as one unit.
//!
//! A [`SessionProvider`] opens a session — a transaction, a pinned protocol
//! session — on an instance it has exclusively for the session's duration,
//! and closes it with a commit or a rollback. The author's body runs between
//! the two with the session borrowed mutably; nothing it holds can outlive
//! the session:
//!
//! ```compile_fail
//! use nebula_resource::{
//!     PoolProvider, Pooled, Provider,
//!     call::{Cost, ResourceHandle, SessionProvider, SessionSpec},
//! };
//!
//! fn smuggle<R>(row: &ResourceHandle<R>)
//! where
//!     R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
//! {
//!     let mut escaped = None;
//!     let _unit = row.session(SessionSpec::read("smuggle").cost(Cost::ONE), |tx, _cx| {
//!         escaped = Some(tx);
//!         Box::pin(async { Ok(()) })
//!     });
//! }
//! ```
//!
//! Sessions run on a [`ResourceHandle`](super::ResourceHandle): each checks out its
//! own connection after its quota wait (see
//! [`ResourceHandle::session`](super::ResourceHandle::session) for the settled
//! outcomes). Long-lived subscriptions (`LISTEN`/`NOTIFY`, IMAP `IDLE`) do
//! not fit a session: a unit's deadline is capped at
//! [`OPERATION_DEADLINE_CAP`](super::OPERATION_DEADLINE_CAP).

use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    num::NonZeroU32,
    pin::Pin,
    time::{Duration, Instant},
};

use nebula_core::ResourceKey;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    cost::{Cost, Effect, SentState},
    declaration::{IdempotencyKey, canonical_json, is_valid_operation_key},
    error::OperationError,
    journal::UnitKind,
    managed::{OperationCx, UnitHost},
    owned::OutputCodec,
    pin::PinSlots,
    work::{Declared, UnitWork},
};
use crate::{
    error::ErrorKind,
    guard::LeaseClosing,
    metrics::SessionOutcome,
    resource::Provider,
    topology::{PoolProvider, Pooled},
};

/// The future a session body returns: boxed, `Send`, and borrowing only the
/// session and its [`SessionCx`] for `'s`.
pub type SessionFuture<'s, T> =
    Pin<Box<dyn Future<Output = Result<T, OperationError>> + Send + 's>>;

/// A provider whose instance can host a session: several native calls run as
/// one admitted, settled unit on one checked-out instance.
///
/// [`open`](Self::open) starts the session on an instance the unit holds
/// exclusively, with the unit's pinned credential slots;
/// [`close`](Self::close) ends it with a commit when the body succeeded and
/// a rollback otherwise, and reports what the provider said. The session
/// borrows the instance for `'c`, so it cannot outlive the checkout.
///
/// ```
/// use async_trait::async_trait;
/// use nebula_core::{ResourceKey, ScopeLevel, resource_key};
/// use nebula_resource::{
///     Error, Manager, PoolConfig, PoolProvider, Pooled, Provider, RegistrationSpec, Resource,
///     ResourceContext, ResourceMetadataDraft, SlotIdentity, metadata_name,
///     call::{Cost, OperationError, SessionClosed, SessionEnd, SessionProvider, SessionSpec},
/// };
///
/// /// A connection that applies statements only when a transaction commits.
/// #[derive(Default)]
/// struct Conn {
///     applied: Vec<String>,
/// }
///
/// /// A transaction borrowing its connection for its whole lifetime.
/// struct Tx<'c> {
///     conn: &'c mut Conn,
///     pending: Vec<String>,
/// }
///
/// impl Tx<'_> {
///     async fn execute(&mut self, statement: &str) -> Result<u64, OperationError> {
///         self.pending.push(statement.to_owned());
///         Ok(1)
///     }
/// }
///
/// #[derive(Resource, Clone)]
/// struct Ledger;
///
/// #[async_trait]
/// impl Provider for Ledger {
///     type Config = ();
///     type Instance = Conn;
///     type Topology = Pooled<Self>;
///
///     fn key() -> ResourceKey {
///         resource_key!("doctest.ledger")
///     }
///
///     fn metadata() -> ResourceMetadataDraft {
///         ResourceMetadataDraft::new(Self::key(), metadata_name!("Ledger"), "")
///     }
///
///     async fn create(&self, _: &(), _: &ResourceContext) -> Result<Conn, Error> {
///         Ok(Conn::default())
///     }
/// }
///
/// impl PoolProvider for Ledger {}
///
/// impl SessionProvider for Ledger {
///     type Session<'c> = Tx<'c>;
///
///     async fn open<'c>(&'c self, conn: &'c mut Conn, _slots: &'c ()) -> Result<Tx<'c>, OperationError> {
///         Ok(Tx { conn, pending: Vec::new() })
///     }
///
///     async fn close<'c>(&'c self, tx: Tx<'c>, end: SessionEnd) -> SessionClosed {
///         match end {
///             SessionEnd::Commit => {
///                 tx.conn.applied.extend(tx.pending);
///                 SessionClosed::Committed
///             },
///             _ => SessionClosed::RolledBack { refused: None },
///         }
///     }
/// }
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let manager = Manager::new();
/// manager.register(RegistrationSpec {
///     resource: Ledger,
///     config: (),
///     scope: ScopeLevel::Global,
///     slot_identity: SlotIdentity::Unbound,
///     topology: Pooled::<Ledger>::new(PoolConfig::default(), 0),
///     recovery_gate: None,
///     rate_limit: None,
/// })?;
/// let ctx = ResourceContext::minimal(
///     nebula_core::scope::Scope::default(),
///     tokio_util::sync::CancellationToken::new(),
/// );
/// let row = manager.handle::<Ledger>(&ctx)?;
///
/// // The body needs no annotations: it borrows the session and returns a
/// // boxed future. A failed body rolls back; a committed one is `Sent`.
/// // The spec names the session and carries its request, which a
/// // journaled row records it under.
/// let spec = SessionSpec::write("ledger.insert", &1_u64).cost(Cost::ONE);
/// let rows = row
///     .session(spec, |tx, _cx| {
///         Box::pin(async move {
///             let rows = tx.execute("insert into ledger values (1)").await?;
///             Ok(rows)
///         })
///     })
///     .await?;
/// assert_eq!(rows, 1);
/// # Ok(())
/// # }
/// ```
pub trait SessionProvider: Provider + PinSlots {
    /// The open session, borrowing the instance it runs on.
    type Session<'c>: Send
    where
        Self: 'c;

    /// What the session is bound to; see [`SessionBinding`].
    const BINDING: SessionBinding = SessionBinding::Connection;

    /// Opens a session on `instance` with the unit's pinned `slots`. An
    /// error means nothing was sent; the instance is not reused.
    fn open<'c>(
        &'c self,
        instance: &'c mut Self::Instance,
        slots: &'c Self::Pinned,
    ) -> impl Future<Output = Result<Self::Session<'c>, OperationError>> + Send + 'c;

    /// Ends `session` as `end` asks and reports what the provider said.
    fn close<'c>(
        &'c self,
        session: Self::Session<'c>,
        end: SessionEnd,
    ) -> impl Future<Output = SessionClosed> + Send + 'c;
}

/// What a session's credentials are bound to, which decides whether an
/// instance built on superseded credential material may host it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionBinding {
    /// The connection authenticated once, when it was built (a database
    /// login): a session runs only on an instance built at the slot
    /// generation the unit pinned, and an older idle instance is evicted.
    Connection,
    /// Each session authenticates with the pinned slots itself (a token per
    /// request): any healthy instance may host it.
    Session,
}

/// How [`SessionProvider::close`] should end a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionEnd {
    /// The body succeeded: make its effects durable.
    Commit,
    /// The body failed: discard its effects.
    Rollback,
}

/// What the provider said when a session closed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SessionClosed {
    /// The commit is durable.
    Committed,
    /// Nothing was applied. `refused` is the provider's error when it
    /// refused a commit (a constraint, a serialization failure); `None`
    /// for a rollback the body asked for.
    RolledBack {
        /// The provider's refusal of a commit, if it refused one.
        refused: Option<OperationError>,
    },
    /// The provider did not say whether the session's effects were applied
    /// (the connection dropped during the commit): the unit's outcome is
    /// unknown and the instance is not reused.
    Unknown(OperationError),
}

/// What one session unit is: its name, what repeating it does, what it
/// costs and — for an effect — the request a journal records it under.
///
/// A session is declared like an [`Operation`](super::Operation): the
/// `name` follows the operation key rules (1 to 64 bytes of
/// `[A-Za-z0-9_.-]`, starting and ending alphanumeric) and is unique among
/// the resource's sessions; an `Idempotent` or `Write` session carries its
/// logical request, canonicalized here, so a journaled row can tell a
/// resumed session from a different one. A name that breaks the rules or a
/// request that does not canonicalize (or is over 1 MiB) is kept as a
/// defect and refuses the session at submit, `Permanent` / `NotSent`.
///
/// Defaults: [`Cost::ONE`], version 1, a 24-hour key window, no developer
/// key part.
///
/// ```
/// use std::time::Duration;
///
/// use nebula_resource::call::{Cost, Effect, SessionSpec};
///
/// let read = SessionSpec::read("ledger.balance");
/// assert_eq!(read.effect(), Effect::Read);
///
/// let transfer = SessionSpec::idempotent("ledger.transfer", &("alice", "bob", 10))
///     .cost(Cost::FREE)
///     .idempotency_key("transfer-42")
///     .key_window(Duration::from_secs(3600));
/// assert_eq!(transfer.effect(), Effect::Idempotent);
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct SessionSpec {
    name: &'static str,
    effect: Effect,
    cost: Cost,
    version: u32,
    key_window: Duration,
    key_part: Option<String>,
    /// The canonical request of an effect; empty for a read.
    canonical_request: Vec<u8>,
    defect: Option<&'static str>,
}

impl SessionSpec {
    fn declare(name: &'static str, effect: Effect) -> Self {
        let defect = (!is_valid_operation_key(name)).then_some(
            "session name must be 1..=64 bytes of [A-Za-z0-9_.-], starting and ending alphanumeric",
        );
        Self {
            name,
            effect,
            cost: Cost::ONE,
            version: 1,
            key_window: Duration::from_hours(24),
            key_part: None,
            canonical_request: Vec::new(),
            defect,
        }
    }

    fn with_request(mut self, request: &(impl Serialize + ?Sized)) -> Self {
        match canonical_json(request) {
            Ok(canonical) => self.canonical_request = canonical,
            Err(error) => {
                self.defect.get_or_insert_with(|| error.detail());
            },
        }
        self
    }

    /// A session named `name` that only reads: [`Effect::Read`], never
    /// recorded by an execution owner.
    #[must_use]
    pub fn read(name: &'static str) -> Self {
        Self::declare(name, Effect::Read)
    }

    /// A session named `name` whose repeat the provider absorbs
    /// ([`Effect::Idempotent`]), for `request` — its logical intent, no
    /// credentials, signatures or timestamps.
    #[must_use]
    pub fn idempotent(name: &'static str, request: &(impl Serialize + ?Sized)) -> Self {
        Self::declare(name, Effect::Idempotent).with_request(request)
    }

    /// A session named `name` whose repeat applies again
    /// ([`Effect::Write`]), for `request` — its logical intent, no
    /// credentials, signatures or timestamps.
    #[must_use]
    pub fn write(name: &'static str, request: &(impl Serialize + ?Sized)) -> Self {
        Self::declare(name, Effect::Write).with_request(request)
    }

    /// Books `cost` once, before the checkout.
    #[must_use]
    pub fn cost(mut self, cost: Cost) -> Self {
        self.cost = cost;
        self
    }

    /// The developer part of the provider idempotency key (1 to 256 bytes
    /// of visible ASCII), as [`Operation::idempotency_key`](super::Operation::idempotency_key).
    #[must_use]
    pub fn idempotency_key(mut self, part: impl Into<String>) -> Self {
        self.key_part = Some(part.into());
        self
    }

    /// How long the provider remembers an idempotency key, as
    /// [`Operation::KEY_WINDOW`](super::Operation::KEY_WINDOW). Non-zero for
    /// an `Idempotent` session.
    #[must_use]
    pub fn key_window(mut self, window: Duration) -> Self {
        self.key_window = window;
        self
    }

    /// The session's interface version, as
    /// [`Operation::VERSION`](super::Operation::VERSION). At least 1.
    #[must_use]
    pub fn version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    /// The session's effect.
    #[must_use]
    pub fn effect(&self) -> Effect {
        self.effect
    }
}

impl fmt::Debug for SessionSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSpec")
            .field("name", &self.name)
            .field("effect", &self.effect)
            .field("cost", &self.cost)
            .field("version", &self.version)
            .field("key_window", &self.key_window)
            .field("key_part", &self.key_part.is_some())
            .field("canonical_request_len", &self.canonical_request.len())
            .field("defect", &self.defect)
            .finish()
    }
}

/// What a session body sees besides the session: the unit's deadline, the
/// checkout's closing notice and the row's key.
pub struct SessionCx {
    deadline: tokio::time::Instant,
    closing: LeaseClosing,
    key: ResourceKey,
    idempotency_key: Option<IdempotencyKey>,
}

impl SessionCx {
    pub(crate) fn new(
        deadline: tokio::time::Instant,
        closing: LeaseClosing,
        key: ResourceKey,
    ) -> Self {
        Self {
            deadline,
            closing,
            key,
            idempotency_key: None,
        }
    }

    /// The context of a session presenting `idempotency_key`.
    fn with_idempotency_key(mut self, idempotency_key: Option<IdempotencyKey>) -> Self {
        self.idempotency_key = idempotency_key;
        self
    }

    /// The provider idempotency key to send: its execution owner's on a
    /// journaled row, a local one for a session that declared a developer
    /// key part elsewhere (see
    /// [`OperationCx::idempotency_key`](super::OperationCx::idempotency_key));
    /// `None` otherwise.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    /// The unit's deadline: the body is stopped at it and the session's
    /// outcome is then unknown.
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline.into_std()
    }

    /// The closing notice of the admission generation the checkout was
    /// granted under. A session is never aborted when it fires; a body that
    /// wants to stop early selects on [`LeaseClosing::closed`] and returns
    /// an error, which rolls the session back.
    #[must_use]
    pub fn closing(&self) -> &LeaseClosing {
        &self.closing
    }

    /// The row's key.
    #[must_use]
    pub fn resource_key(&self) -> &ResourceKey {
        &self.key
    }
}

impl fmt::Debug for SessionCx {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCx")
            .field("resource_key", &self.key)
            .field("closing", &self.closing.is_closing())
            .finish_non_exhaustive()
    }
}

tokio::task_local! {
    /// The rows (their [`RowShared::marker`](super::row::RowShared::marker))
    /// whose session body is running on this task.
    static SESSION_ROWS: Vec<usize>;
}

/// Whether this task is running a session body of the row `marker`.
pub(super) fn in_session_of(marker: usize) -> bool {
    SESSION_ROWS
        .try_with(|rows| rows.contains(&marker))
        .unwrap_or(false)
}

/// A session as the unit runtime runs it: one attempt, never retried
/// (`max_attempts` is one), open → body → close, settled from what the
/// provider said (see [`ResourceHandle::session`](super::ResourceHandle::session)).
pub(super) struct Sessioned<R, F, T> {
    spec: SessionSpec,
    body: F,
    output: PhantomData<fn() -> (R, T)>,
}

impl<R, F, T> Sessioned<R, F, T> {
    pub(super) fn new(spec: SessionSpec, body: F) -> Self {
        Self {
            spec,
            body,
            output: PhantomData,
        }
    }
}

impl<R, F, T> UnitWork<R> for Sessioned<R, F, T>
where
    R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
    T: Serialize + DeserializeOwned + Send + 'static,
    F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
        + Send
        + 'static,
{
    type Output = T;

    fn declared(&self) -> Declared {
        Declared {
            kind: UnitKind::Session,
            name: self.spec.name,
            version: self.spec.version,
            effect: self.spec.effect,
            key_window: self.spec.key_window,
            record_output: true,
        }
    }

    fn defect(&self) -> Option<&'static str> {
        self.spec.defect
    }

    fn key_part(&self) -> Option<String> {
        self.spec.key_part.clone()
    }

    fn canonical_request(&self) -> Result<Vec<u8>, OperationError> {
        Ok(self.spec.canonical_request.clone())
    }

    fn codec() -> Option<OutputCodec<T>> {
        Some(OutputCodec::json())
    }

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::MIN
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<T, OperationError> {
        let Self { spec, body, .. } = self;
        let deadline = cx.deadline;
        let marker = match cx.host {
            UnitHost::Row(row) => Some(row.marker()),
            UnitHost::Lease(_) => None,
        };
        let idempotency_key = cx.idempotency_key().copied();
        let mut attempt = cx.attempt_session(spec.cost).await?;
        let ended = match attempt.session_parts() {
            Some((provider, instance, slots, closing)) => {
                let session_cx = SessionCx::new(deadline, closing, R::key())
                    .with_idempotency_key(idempotency_key);
                drive_session(provider, instance, slots, &session_cx, marker, body).await
            },
            None => SessionEnded {
                sent: SentState::NotSent,
                keep: false,
                outcome: SessionOutcome::OpenFailed,
                result: Err(OperationError::new(
                    ErrorKind::Permanent,
                    "session attempt without a checkout",
                )),
            },
        };
        attempt.end_session(ended.outcome, ended.keep);
        attempt.settle(ended.sent);
        ended.result
    }
}

/// How a session ended: the attempt's sent state, whether its instance is
/// reused, the outcome counted, and the unit's result.
struct SessionEnded<T> {
    sent: SentState,
    keep: bool,
    outcome: SessionOutcome,
    result: Result<T, OperationError>,
}

/// Open, body (inside the nested-session marker), close; settled by
/// [`settle_session`]. An open that fails is `NotSent` and its instance is
/// not reused (a half-opened instance is in an unknown state).
async fn drive_session<R, F, T>(
    provider: &R,
    instance: &mut R::Instance,
    slots: &R::Pinned,
    session_cx: &SessionCx,
    marker: Option<usize>,
    body: F,
) -> SessionEnded<T>
where
    R: SessionProvider,
    T: Send + 'static,
    F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
        + Send
        + 'static,
{
    let mut session = match provider.open(instance, slots).await {
        Ok(session) => session,
        Err(error) => {
            return SessionEnded {
                sent: SentState::NotSent,
                keep: false,
                outcome: SessionOutcome::OpenFailed,
                result: Err(error),
            };
        },
    };
    let mut rows = SESSION_ROWS.try_with(Clone::clone).unwrap_or_default();
    rows.extend(marker);
    let result = SESSION_ROWS
        .scope(rows, body(&mut session, session_cx))
        .await;
    let end = if result.is_ok() {
        SessionEnd::Commit
    } else {
        SessionEnd::Rollback
    };
    let closed = provider.close(session, end).await;
    settle_session(result, closed)
}

/// The session's settled state, whether its instance is reused, and the
/// unit's outcome, from the body's result and what the provider said at
/// close (the table on [`ResourceHandle::session`](super::ResourceHandle::session)).
fn settle_session<T>(result: Result<T, OperationError>, closed: SessionClosed) -> SessionEnded<T> {
    let (sent, keep, outcome, result) = match (result, closed) {
        (Ok(value), SessionClosed::Committed) => {
            (SentState::Sent, true, SessionOutcome::Committed, Ok(value))
        },
        (Ok(_), SessionClosed::RolledBack { refused }) => (
            SentState::NotSent,
            true,
            SessionOutcome::RolledBack,
            Err(refused.unwrap_or_else(|| {
                OperationError::new(
                    ErrorKind::Transient,
                    "the provider rolled the commit back without a reason",
                )
            })),
        ),
        (Err(error), SessionClosed::RolledBack { .. }) => (
            SentState::NotSent,
            true,
            SessionOutcome::RolledBack,
            Err(error),
        ),
        // Asked to roll back, the provider reports a commit: whatever the
        // body did was applied.
        (Err(error), SessionClosed::Committed) => {
            (SentState::Sent, true, SessionOutcome::Committed, Err(error))
        },
        (_, SessionClosed::Unknown(error)) => (
            SentState::MaybeSent,
            false,
            SessionOutcome::Unknown,
            Err(error),
        ),
    };
    SessionEnded {
        sent,
        keep,
        outcome,
        result,
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::{
        OperationError, SessionClosed, SessionCx, SessionEnd, SessionFuture, SessionProvider,
    };
    use crate::{
        Provider, call::PinSlots, guard::LeaseClosing, manager::strict_fixtures::StrictPooled,
    };

    /// Open, body, close — the shape the runtime drives, generic over the
    /// provider so the `Send` proof below holds for every provider.
    async fn drive<R, T, F>(
        provider: &R,
        instance: &mut R::Instance,
        slots: &R::Pinned,
        cx: &SessionCx,
        body: F,
    ) -> (Result<T, OperationError>, SessionClosed)
    where
        R: SessionProvider,
        T: Send + 'static,
        F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
            + Send
            + 'static,
    {
        let mut session = match provider.open(instance, slots).await {
            Ok(session) => session,
            Err(error) => {
                return (Err(error), SessionClosed::RolledBack { refused: None });
            },
        };
        let result = body(&mut session, cx).await;
        let end = if result.is_ok() {
            SessionEnd::Commit
        } else {
            SessionEnd::Rollback
        };
        let closed = provider.close(session, end).await;
        (result, closed)
    }

    fn assert_send<T: Send>(_: &T) {}

    /// The driver's future is `Send` for any provider and body: a unit's
    /// runtime task can run it.
    fn drive_is_send<R, T, F>(
        provider: &R,
        instance: &mut R::Instance,
        slots: &R::Pinned,
        cx: &SessionCx,
        body: F,
    ) where
        R: SessionProvider,
        T: Send + 'static,
        F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
            + Send
            + 'static,
    {
        assert_send(&drive(provider, instance, slots, cx, body));
    }

    fn cx() -> SessionCx {
        SessionCx::new(
            tokio::time::Instant::now() + Duration::from_secs(1),
            LeaseClosing::detached(),
            StrictPooled::key(),
        )
    }

    #[tokio::test]
    async fn an_unannotated_body_borrows_the_session_and_commits() {
        let provider = StrictPooled::new();
        let slots = provider.pin_slots();
        let mut instance = 40;
        let cx = cx();
        let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let observed = Arc::clone(&seen);
        let (result, closed) = drive(&provider, &mut instance, &slots, &cx, move |tx, cx| {
            Box::pin(async move {
                tx.pending += 2;
                observed.store(*tx.instance, std::sync::atomic::Ordering::SeqCst);
                assert!(!cx.closing().is_closing());
                Ok(tx.pending)
            })
        })
        .await;
        assert_eq!(result.ok(), Some(2));
        assert!(matches!(closed, SessionClosed::Committed));
        assert_eq!(instance, 42, "the commit applied the pending change");
        assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 40);
        drive_is_send(&provider, &mut instance, &slots, &cx, |tx, _cx| {
            Box::pin(async move { Ok(tx.pending) })
        });
    }

    #[tokio::test]
    async fn a_failed_body_rolls_back() {
        let provider = StrictPooled::new();
        let slots = provider.pin_slots();
        let mut instance = 1;
        let (result, closed) = drive(&provider, &mut instance, &slots, &cx(), |tx, _cx| {
            Box::pin(async move {
                tx.pending = 5;
                Err::<(), _>(OperationError::new(
                    crate::ErrorKind::Permanent,
                    "constraint",
                ))
            })
        })
        .await;
        assert!(result.is_err());
        assert!(matches!(
            closed,
            SessionClosed::RolledBack { refused: None }
        ));
        assert_eq!(instance, 1, "nothing applied");
    }
}

#[cfg(test)]
#[path = "../call_session_tests.rs"]
mod runtime_tests;
