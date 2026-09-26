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
//! use nebula_resource::call::{SessionCx, SessionFuture, SessionProvider};
//!
//! fn session<R: SessionProvider, T, F>(_body: F)
//! where
//!     F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
//!         + Send
//!         + 'static,
//! {
//! }
//!
//! fn smuggle<R: SessionProvider>() {
//!     let mut escaped = None;
//!     session::<R, (), _>(|tx, _cx| {
//!         escaped = Some(tx);
//!         Box::pin(async { Ok(()) })
//!     });
//! }
//! ```

use std::{fmt, future::Future, pin::Pin, time::Instant};

use nebula_core::ResourceKey;

use super::{
    cost::{Cost, Effect},
    error::OpError,
    pin::PinSlots,
};
use crate::{guard::LeaseClosing, resource::Provider};

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
/// use nebula_core::{ResourceKey, resource_key};
/// use nebula_resource::{
///     Error, PoolProvider, Pooled, Provider, Resource, ResourceContext, ResourceMetadataDraft,
///     metadata_name,
///     call::{
///         OpError, SessionClosed, SessionCx, SessionEnd, SessionFuture, SessionProvider,
///     },
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
/// // The body's signature, as a session entry point bounds it: the closure
/// // needs no annotations.
/// fn session<R: SessionProvider, T, F>(_body: F)
/// where
///     F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
///         + Send
///         + 'static,
/// {
/// }
///
/// session::<Ledger, _, _>(|tx, _cx| {
///     Box::pin(async move {
///         let rows = tx.execute("insert into ledger values (1)").await?;
///         Ok(rows)
///     })
/// });
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
    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::{OpError, SessionClosed, SessionCx, SessionEnd, SessionFuture, SessionProvider};
    use crate::{
        Provider,
        call::PinSlots,
        guard::LeaseClosing,
        manager::strict_fixtures::{PinnedEpochs, StrictPooled},
    };

    /// A transaction over the fixture's `u64` instance: adds to it on
    /// commit.
    pub(crate) struct Tx<'c> {
        instance: &'c mut u64,
        pending: u64,
    }

    impl SessionProvider for StrictPooled {
        type Session<'c> = Tx<'c>;

        async fn open<'c>(
            &'c self,
            instance: &'c mut u64,
            _slots: &'c PinnedEpochs,
        ) -> Result<Tx<'c>, OpError> {
            Ok(Tx {
                instance,
                pending: 0,
            })
        }

        async fn close<'c>(&'c self, session: Tx<'c>, end: SessionEnd) -> SessionClosed {
            match end {
                SessionEnd::Commit => {
                    *session.instance += session.pending;
                    SessionClosed::Committed
                },
                SessionEnd::Rollback => SessionClosed::RolledBack { refused: None },
            }
        }
    }

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
