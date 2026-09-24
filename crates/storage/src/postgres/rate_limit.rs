//! PostgreSQL [`LimitStore`]: cluster-wide GCRA rate limits.
//!
//! Every operation is one transaction over one key row: the row is locked
//! (created if absent) by an upsert that also reads the server clock *after*
//! the lock is granted, the same [`step`] functions the in-process store
//! uses decide, and the new state is written back. Time is
//! `clock_timestamp()` in nanoseconds since the Unix epoch — never a
//! worker's clock, and never `now()`, which is frozen for the transaction.
//!
//! A key's row stores the rate it last enforced next to its state, so
//! callers that disagree on the rate get the stricter one while the key is
//! busy ([`step::enforce`]). Rows whose schedule has passed carry no
//! state and are swept every [`SWEEP_EVERY`] operations.

use std::{
    fmt,
    num::NonZeroU32,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use nebula_resilience::rate_limiter::gcra::{
    Denied, GcraState, Grant, LimitKey, LimitStore, LimitStoreError, Rate, ReservationId,
    ReserveRequest, step,
};
use sqlx::{PgPool, Postgres, Row as _, Transaction};

/// Operations between sweeps of idle keys.
const SWEEP_EVERY: u64 = 1024;

/// Server time in nanoseconds since the Unix epoch, as a SQL literal for
/// `concat!`. `EXTRACT` returns an exact `numeric`, so no precision is lost
/// before the cast.
macro_rules! now_ns {
    () => {
        "(EXTRACT(EPOCH FROM clock_timestamp()) * 1000000000)::BIGINT"
    };
}

/// PostgreSQL implementation of [`LimitStore`], for limits shared by every
/// worker process.
pub struct PgLimitStore {
    pool: PgPool,
    ops: AtomicU64,
}

impl fmt::Debug for PgLimitStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PgLimitStore")
            .finish_non_exhaustive()
    }
}

impl PgLimitStore {
    /// Wrap a pool initialized through [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            ops: AtomicU64::new(0),
        }
    }

    /// Locks `key`'s row, creating it with `rate` if absent, and returns its
    /// state, the rate it enforced last, and the server time read after the
    /// lock was granted.
    async fn lock(
        tx: &mut Transaction<'_, Postgres>,
        key: &LimitKey,
        rate: &Rate,
    ) -> Result<(GcraState, Option<Rate>, u64), LimitStoreError> {
        let row = sqlx::query(concat!(
            "INSERT INTO port_rate_limits (limit_key, tat_ns, seq, emission_ns, burst) ",
            "VALUES ($1, 0, 0, $2, $3) ",
            "ON CONFLICT (limit_key) DO UPDATE SET limit_key = EXCLUDED.limit_key ",
            "RETURNING tat_ns, seq, emission_ns, burst, ",
            now_ns!(),
            " AS now_ns"
        ))
        .bind(key.as_str())
        .bind(to_db(nanos(rate.emission_interval())))
        .bind(burst_to_db(rate.burst()))
        .fetch_one(&mut **tx)
        .await
        .map_err(unavailable)?;
        let state = GcraState {
            tat: from_db(row.try_get("tat_ns").map_err(unavailable)?),
            seq: seq_from_db(row.try_get("seq").map_err(unavailable)?),
        };
        let emission: i64 = row.try_get("emission_ns").map_err(unavailable)?;
        let burst: i32 = row.try_get("burst").map_err(unavailable)?;
        // A stored rate that no longer parses is forgotten, never loosened
        // into: the caller's rate applies as on an idle key.
        let stored = u32::try_from(burst)
            .ok()
            .and_then(NonZeroU32::new)
            .and_then(|burst| {
                Rate::from_interval(Duration::from_nanos(from_db(emission)), burst).ok()
            });
        let now = from_db(row.try_get("now_ns").map_err(unavailable)?);
        Ok((state, stored, now))
    }

    async fn write(
        tx: &mut Transaction<'_, Postgres>,
        key: &LimitKey,
        state: GcraState,
        rate: &Rate,
    ) -> Result<(), LimitStoreError> {
        sqlx::query(
            "UPDATE port_rate_limits SET tat_ns = $2, seq = $3, emission_ns = $4, burst = $5 \
             WHERE limit_key = $1",
        )
        .bind(key.as_str())
        .bind(to_db(state.tat))
        .bind(seq_to_db(state.seq))
        .bind(to_db(nanos(rate.emission_interval())))
        .bind(burst_to_db(rate.burst()))
        .execute(&mut **tx)
        .await
        .map_err(unavailable)?;
        Ok(())
    }

    /// Deletes rows whose schedule has passed and reservations whose slot
    /// has arrived, every [`SWEEP_EVERY`] operations. Both behave exactly
    /// like absent rows, so a sweep never loosens a limit; a row a
    /// concurrent operation just advanced no longer matches when the delete
    /// re-checks it.
    async fn maybe_sweep(&self) {
        if !self
            .ops
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
            .is_multiple_of(SWEEP_EVERY)
        {
            return;
        }
        let swept = sqlx::query(concat!(
            "WITH reservations AS (DELETE FROM port_rate_limit_reservations WHERE allow_at_ns <= ",
            now_ns!(),
            ") DELETE FROM port_rate_limits WHERE tat_ns <= ",
            now_ns!()
        ))
        .execute(&self.pool)
        .await;
        if let Err(error) = swept {
            tracing::debug!(
                target: "nebula_storage::rate_limit",
                error = %error,
                "idle rate-limit sweep failed; retried on a later operation"
            );
        }
    }
}

impl LimitStore for PgLimitStore {
    #[tracing::instrument(skip_all, fields(storage.role = "rate_limit", storage.operation = "reserve"))]
    async fn reserve(
        &self,
        key: &LimitKey,
        rate: &Rate,
        request: ReserveRequest,
    ) -> Result<Result<Grant, Denied>, LimitStoreError> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        let (state, stored, now) = Self::lock(&mut tx, key, rate).await?;
        let before = state;
        let (rate, state) = step::enforce(state, now, stored.as_ref(), rate);
        let enforced = state != before || stored != Some(rate);
        // A repeat books nothing, but a stricter rate it declares still
        // applies to the key, as it does in the in-memory store.
        if let Some(id) = request.id
            && let Some(original) = Self::pending(&mut tx, key, id, now).await?
        {
            if enforced {
                Self::write(&mut tx, key, state, &rate).await?;
            }
            tx.commit().await.map_err(unavailable)?;
            return Ok(Ok(original));
        }
        let (decision, next) = step::reserve_from(
            state,
            now,
            &rate,
            request.permits,
            request.max_wait,
            request.not_before,
        );
        // A refusal books nothing, but the rate the key now enforces and its
        // rebased schedule are kept, as the in-memory store keeps them:
        // otherwise a later caller at the looser rate would run against the
        // schedule the stricter one already stretched.
        match next {
            Some(next) => Self::write(&mut tx, key, next, &rate).await?,
            None if enforced => {
                Self::write(&mut tx, key, state, &rate).await?;
            },
            None => {},
        }
        if let (Ok(grant), Some(id)) = (&decision, request.id)
            && grant.allow_at > now
        {
            sqlx::query(
                "INSERT INTO port_rate_limit_reservations \
                 (limit_key, reservation_id, permits, allow_at_ns, end_tat_ns, seq) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (limit_key, reservation_id) DO UPDATE SET \
                 permits = EXCLUDED.permits, allow_at_ns = EXCLUDED.allow_at_ns, \
                 end_tat_ns = EXCLUDED.end_tat_ns, seq = EXCLUDED.seq",
            )
            .bind(key.as_str())
            .bind(reservation_id(id))
            .bind(permits_to_db(grant.permits))
            .bind(to_db(grant.allow_at))
            .bind(to_db(grant.end_tat))
            .bind(seq_to_db(grant.seq))
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        }
        tx.commit().await.map_err(unavailable)?;
        self.maybe_sweep().await;
        Ok(decision)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "rate_limit", storage.operation = "penalize"))]
    async fn penalize(
        &self,
        key: &LimitKey,
        rate: &Rate,
        retry_after: Duration,
        max_penalty: Duration,
    ) -> Result<(), LimitStoreError> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        let (state, stored, now) = Self::lock(&mut tx, key, rate).await?;
        let (rate, state) = step::enforce(state, now, stored.as_ref(), rate);
        let next = step::penalize(state, now, &rate, retry_after, max_penalty);
        Self::write(&mut tx, key, next, &rate).await?;
        let until = now.saturating_add(nanos(retry_after.min(max_penalty)));
        sqlx::query(
            "UPDATE port_rate_limits SET penalized_until_ns = GREATEST(penalized_until_ns, $2) \
             WHERE limit_key = $1",
        )
        .bind(key.as_str())
        .bind(to_db(until))
        .execute(&mut *tx)
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        self.maybe_sweep().await;
        Ok(())
    }

    /// Refunds at the caller's `rate`, as the contract requires (see
    /// [`LimitStore::cancel`]).
    #[tracing::instrument(skip_all, fields(storage.role = "rate_limit", storage.operation = "cancel"))]
    async fn cancel(
        &self,
        key: &LimitKey,
        rate: &Rate,
        grant: &Grant,
    ) -> Result<bool, LimitStoreError> {
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        let (state, stored, now) = Self::lock(&mut tx, key, rate).await?;
        let Some(next) = step::cancel(state, now, rate, grant) else {
            tx.rollback().await.map_err(unavailable)?;
            return Ok(false);
        };
        // The key keeps the rate it enforced; a refund changes only the
        // schedule.
        let enforced = stored.unwrap_or(*rate);
        Self::write(&mut tx, key, next, &enforced).await?;
        sqlx::query("DELETE FROM port_rate_limit_reservations WHERE limit_key = $1 AND seq = $2")
            .bind(key.as_str())
            .bind(seq_to_db(grant.seq))
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        Ok(true)
    }

    #[tracing::instrument(skip_all, fields(storage.role = "rate_limit", storage.operation = "penalty"))]
    async fn penalty(&self, key: &LimitKey) -> Result<Duration, LimitStoreError> {
        let left: Option<i64> = sqlx::query_scalar(concat!(
            "SELECT GREATEST(penalized_until_ns - ",
            now_ns!(),
            ", 0) FROM port_rate_limits WHERE limit_key = $1"
        ))
        .bind(key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(unavailable)?;
        Ok(Duration::from_nanos(left.map_or(0, from_db)))
    }
}

impl PgLimitStore {
    /// The grant booked under `id` whose slot has not arrived yet.
    async fn pending(
        tx: &mut Transaction<'_, Postgres>,
        key: &LimitKey,
        id: ReservationId,
        now: u64,
    ) -> Result<Option<Grant>, LimitStoreError> {
        let row = sqlx::query(
            "SELECT permits, allow_at_ns, end_tat_ns, seq FROM port_rate_limit_reservations \
             WHERE limit_key = $1 AND reservation_id = $2 AND allow_at_ns > $3",
        )
        .bind(key.as_str())
        .bind(reservation_id(id))
        .bind(to_db(now))
        .fetch_optional(&mut **tx)
        .await
        .map_err(unavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let allow_at = from_db(row.try_get("allow_at_ns").map_err(unavailable)?);
        let permits: i32 = row.try_get("permits").map_err(unavailable)?;
        Ok(Some(Grant {
            wait: Duration::from_nanos(allow_at.saturating_sub(now)),
            permits: u32::try_from(permits).unwrap_or(0),
            allow_at,
            end_tat: from_db(row.try_get("end_tat_ns").map_err(unavailable)?),
            seq: seq_from_db(row.try_get("seq").map_err(unavailable)?),
        }))
    }
}

fn unavailable(error: sqlx::Error) -> LimitStoreError {
    LimitStoreError::Unavailable(Box::new(error))
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Store-clock nanoseconds fit `BIGINT` until the year 2262; beyond that
/// they clamp.
fn to_db(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn from_db(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// The mutation counter wraps and is only compared, so it is stored bit for
/// bit.
fn seq_to_db(seq: u64) -> i64 {
    i64::from_ne_bytes(seq.to_ne_bytes())
}

fn seq_from_db(seq: i64) -> u64 {
    u64::from_ne_bytes(seq.to_ne_bytes())
}

fn burst_to_db(burst: NonZeroU32) -> i32 {
    i32::try_from(burst.get()).unwrap_or(i32::MAX)
}

fn permits_to_db(permits: u32) -> i32 {
    i32::try_from(permits).unwrap_or(i32::MAX)
}

fn reservation_id(id: ReservationId) -> String {
    format!("{:032x}", id.0)
}
