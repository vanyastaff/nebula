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
//!     call::{Cost, ManagedRow, SessionProvider, SessionSpec},
//! };
//!
//! fn smuggle<R>(row: &ManagedRow<R>)
//! where
//!     R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
//! {
//!     let mut escaped = None;
//!     let _unit = row.session(SessionSpec::new(Cost::ONE), |tx, _cx| {
//!         escaped = Some(tx);
//!         Box::pin(async { Ok(()) })
//!     });
//! }
//! ```
//!
//! Sessions run on a [`ManagedRow`](super::ManagedRow): each checks out its
//! own connection after its quota wait (see
//! [`ManagedRow::session`](super::ManagedRow::session) for the settled
//! outcomes). Long-lived subscriptions (`LISTEN`/`NOTIFY`, IMAP `IDLE`) do
//! not fit a session: a unit's deadline is capped at
//! [`UNIT_DEADLINE_CAP`](super::UNIT_DEADLINE_CAP).

use std::{fmt, future::Future, marker::PhantomData, pin::Pin, time::Instant};

use nebula_core::ResourceKey;

use super::{
    Operation,
    cost::{Cost, Effect, SentState},
    error::OpError,
    managed::{OpCx, UnitHost},
    pin::PinSlots,
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
pub type SessionFuture<'s, T> = Pin<Box<dyn Future<Output = Result<T, OpError>> + Send + 's>>;

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
///     call::{Cost, OpError, SessionClosed, SessionEnd, SessionProvider, SessionSpec},
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
///     async fn execute(&mut self, statement: &str) -> Result<u64, OpError> {
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
///     async fn open<'c>(&'c self, conn: &'c mut Conn, _slots: &'c ()) -> Result<Tx<'c>, OpError> {
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
/// let row = manager.managed_row::<Ledger>(&ctx)?;
///
/// // The body needs no annotations: it borrows the session and returns a
/// // boxed future. A failed body rolls back; a committed one is `Sent`.
/// let rows = row
///     .session(SessionSpec::new(Cost::ONE), |tx, _cx| {
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
    ) -> impl Future<Output = Result<Self::Session<'c>, OpError>> + Send + 'c;

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
        refused: Option<OpError>,
    },
    /// The provider did not say whether the session's effects were applied
    /// (the connection dropped during the commit): the unit's outcome is
    /// unknown and the instance is not reused.
    Unknown(OpError),
}

/// What one session unit costs and what repeating it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    cost: Cost,
    effect: Effect,
}

impl SessionSpec {
    /// A session booked at `cost` whose effect is [`Effect::Write`].
    #[must_use]
    pub fn new(cost: Cost) -> Self {
        Self {
            cost,
            effect: Effect::Write,
        }
    }

    /// Declares what repeating the session does to the provider.
    #[must_use]
    pub fn with_effect(mut self, effect: Effect) -> Self {
        self.effect = effect;
        self
    }

    /// The cost booked once, before the checkout.
    #[must_use]
    pub fn cost(&self) -> &Cost {
        &self.cost
    }

    /// The session's effect.
    #[must_use]
    pub fn effect(&self) -> Effect {
        self.effect
    }
}

/// What a session body sees besides the session: the unit's deadline, the
/// checkout's closing notice and the row's key.
pub struct SessionCx {
    deadline: tokio::time::Instant,
    closing: LeaseClosing,
    key: ResourceKey,
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
        }
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
/// provider said (see [`ManagedRow::session`](super::ManagedRow::session)).
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

impl<R, F, T> Operation<R> for Sessioned<R, F, T>
where
    R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
    T: Send + 'static,
    F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
        + Send
        + 'static,
{
    type Output = T;

    async fn run(self, cx: &mut OpCx<'_, R>) -> Result<T, OpError> {
        let Self { spec, body, .. } = self;
        let deadline = cx.deadline;
        let marker = match cx.host {
            UnitHost::Row(row) => Some(row.marker()),
            UnitHost::Lease(_) => None,
        };
        let mut attempt = cx.attempt_session(spec.cost().clone()).await?;
        let ended = match attempt.session_parts() {
            Some((provider, instance, slots, closing)) => {
                let session_cx = SessionCx::new(deadline, closing, R::key());
                drive_session(provider, instance, slots, &session_cx, marker, body).await
            },
            None => SessionEnded {
                sent: SentState::NotSent,
                keep: false,
                outcome: SessionOutcome::OpenFailed,
                result: Err(OpError::new(
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
    result: Result<T, OpError>,
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
/// close (the table on [`ManagedRow::session`](super::ManagedRow::session)).
fn settle_session<T>(result: Result<T, OpError>, closed: SessionClosed) -> SessionEnded<T> {
    let (sent, keep, outcome, result) = match (result, closed) {
        (Ok(value), SessionClosed::Committed) => {
            (SentState::Sent, true, SessionOutcome::Committed, Ok(value))
        },
        (Ok(_), SessionClosed::RolledBack { refused }) => (
            SentState::NotSent,
            true,
            SessionOutcome::RolledBack,
            Err(refused.unwrap_or_else(|| {
                OpError::new(
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

    use super::{OpError, SessionClosed, SessionCx, SessionEnd, SessionFuture, SessionProvider};
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
    ) -> (Result<T, OpError>, SessionClosed)
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
                Err::<(), _>(OpError::new(crate::ErrorKind::Permanent, "constraint"))
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
