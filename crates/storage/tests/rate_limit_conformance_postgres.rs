//! Rate-limit store conformance for the PostgreSQL adapter: the same kit the
//! in-process store passes, against a real server clock and real row locks.

#![cfg(feature = "postgres")]

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nebula_resilience::rate_limiter::gcra::conformance;
use nebula_storage::postgres::{PgLimitStore, init_schema};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

fn unique_schema() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("nebula_rate_limit_{}_{nanos}", std::process::id())
}

async fn store() -> Option<PgLimitStore> {
    store_and_pool().await.map(|(store, _)| store)
}

async fn store_and_pool() -> Option<(PgLimitStore, sqlx::PgPool)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            assert_ne!(
                std::env::var("NEBULA_REQUIRE_POSTGRES").as_deref(),
                Ok("1"),
                "DATABASE_URL must be set when NEBULA_REQUIRE_POSTGRES=1"
            );
            return None;
        },
        Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
    };
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    let schema = unique_schema();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create isolated schema");
    let options = PgConnectOptions::from_str(&url)
        .expect("parse DATABASE_URL")
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        // More connections than the kit's concurrent callers contend for,
        // so the contention case exercises row locks, not pool queueing.
        .max_connections(16)
        .connect_with(options)
        .await
        .expect("connect to isolated schema");
    init_schema(&pool)
        .await
        .expect("initialize isolated schema");
    Some((PgLimitStore::new(pool.clone()), pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_limit_store_passes_the_conformance_kit() {
    let Some(store) = store().await else {
        panic!(
            "postgres_limit_store_passes_the_conformance_kit: backend unreachable — the case \
             cannot run and must fail rather than pass unchecked; reach the backend (set \
             DATABASE_URL for postgres) or run without this feature"
        );
    };
    conformance::run_all(Arc::new(store)).await;
}

/// Server clock in store nanoseconds.
async fn server_now(pool: &sqlx::PgPool) -> u64 {
    let now: i64 =
        sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000000000)::BIGINT")
            .fetch_one(pool)
            .await
            .expect("read the server clock");
    u64::try_from(now).expect("server clock after 1970")
}

/// Deterministic xorshift, so a failing case reproduces.
struct Cases(u64);

impl Cases {
    fn pick<T: Copy>(&mut self, values: &[T]) -> T {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        values[usize::try_from(self.0 % values.len() as u64).unwrap()]
    }
}

/// The one-statement reservation agrees with the Rust `step` functions the
/// transactional path runs: same decision, same booked slot and same state
/// written back, for busy, idle, tightened and loosened keys, stored rates
/// that no longer parse, zero and oversized permits, deadlines and
/// `not_before`. Store-clock values are compared within the time the call
/// took, since the statement reads its own clock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_statement_reservation_matches_the_step_functions() {
    use std::num::NonZeroU32;
    use std::time::Duration;

    use nebula_resilience::rate_limiter::gcra::{
        Denied, GcraState, LimitKey, LimitStore as _, Rate, ReserveRequest, step,
    };

    let Some((store, pool)) = store_and_pool().await else {
        panic!("one_statement_reservation_matches_the_step_functions: backend unreachable");
    };
    let second = 1_000_000_000_u64;
    let intervals = [second, 7 * second, 60 * second, 3_600 * second];
    let bursts = [1_u32, 2, 3, 4];
    let mut cases = Cases(0x9e37_79b9_7f4a_7c15);
    let mut checked = 0;
    for case in 0..400 {
        let key = LimitKey::new(format!("differential-{case}")).unwrap();
        let stored_interval: u64 = cases.pick(&intervals);
        let stored_burst: u32 = cases.pick(&bursts);
        // A stored burst window that overflows is forgotten by both paths.
        let (stored_interval, stored_burst) = if cases.pick(&[false, false, false, true]) {
            (u64::MAX / 4, 100)
        } else {
            (stored_interval, stored_burst)
        };
        let seq: i64 = cases.pick(&[0, 41, i64::MAX, -1]);
        let now = server_now(&pool).await;
        let tat_offset: i64 = cases.pick(&[-5, 0, 1, 10, 600, 10_800]);
        let tat = if tat_offset <= 0 {
            now.saturating_sub(tat_offset.unsigned_abs() * second)
        } else {
            now + tat_offset.unsigned_abs() * second
        };
        sqlx::query(
            "INSERT INTO rate_limits (limit_key, tat_ns, seq, emission_ns, burst) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(key.as_str())
        .bind(i64::try_from(tat).unwrap())
        .bind(seq)
        .bind(i64::try_from(stored_interval).unwrap())
        .bind(i32::try_from(stored_burst).unwrap())
        .execute(&pool)
        .await
        .unwrap();

        let rate = Rate::from_interval(
            Duration::from_nanos(cases.pick(&intervals)),
            NonZeroU32::new(cases.pick(&bursts)).unwrap(),
        )
        .unwrap();
        let permits = cases.pick(&[0_u32, 1, 1, 2, 3, 5]);
        let max_wait = cases.pick(&[
            Duration::ZERO,
            Duration::from_secs(30),
            Duration::from_hours(2),
            Duration::MAX,
        ]);
        let not_before_offset: u64 = cases.pick(&[0, 0, 20, 10_800]);
        let not_before = if not_before_offset == 0 {
            0
        } else {
            now + not_before_offset * second
        };
        let request = ReserveRequest::new(permits, max_wait).not_before(not_before);

        let lo = server_now(&pool).await;
        let decision = store.reserve(&key, &rate, request).await.unwrap();
        let hi = server_now(&pool).await;

        let state = GcraState {
            tat,
            seq: u64::from_ne_bytes(seq.to_ne_bytes()),
        };
        let stored = NonZeroU32::new(stored_burst).and_then(|burst| {
            Rate::from_interval(Duration::from_nanos(stored_interval), burst).ok()
        });
        let expect = |now: u64| {
            let (enforced, rebased) = step::enforce(state, now, stored.as_ref(), &rate);
            let (decision, next) = step::reserve_from(
                rebased,
                now,
                &enforced,
                request.permits,
                request.max_wait,
                request.not_before,
            );
            (decision, next.unwrap_or(rebased), enforced)
        };
        let (at_lo, written_lo, enforced) = expect(lo);
        let (at_hi, _, _) = expect(hi);
        let kind = |decision: &Result<_, Denied>| match decision {
            Ok(_) => 0,
            Err(Denied::Later { .. }) => 1,
            Err(_) => 2,
        };
        if kind(&at_lo) != kind(&at_hi) {
            // The decision flips within the call's own duration: either is right.
            continue;
        }
        // A quantity moves at most one nanosecond per nanosecond of clock,
        // or by the interval ratio when a busy key's schedule is stretched.
        let ratio = (enforced.emission_interval().as_nanos() / u128::from(stored_interval)).max(1);
        let slack = (hi - lo) * (u64::try_from(ratio).unwrap() + 1) + 1;
        let close = |actual: u64, expected: u64, what: &str| {
            assert!(
                actual.abs_diff(expected) <= slack,
                "case {case}: {what} {actual} vs expected {expected} (slack {slack})"
            );
        };
        match (&decision, &at_lo) {
            (Ok(actual), Ok(expected)) => {
                assert_eq!(actual.permits, expected.permits, "case {case}: permits");
                assert_eq!(actual.seq, expected.seq, "case {case}: seq");
                close(actual.allow_at, expected.allow_at, "allow_at");
                close(actual.end_tat, expected.end_tat, "end_tat");
                close(
                    u64::try_from(actual.wait.as_nanos()).unwrap(),
                    u64::try_from(expected.wait.as_nanos()).unwrap(),
                    "wait",
                );
            },
            (
                Err(Denied::Later {
                    retry_after: actual,
                }),
                Err(Denied::Later {
                    retry_after: expected,
                }),
            ) => close(
                u64::try_from(actual.as_nanos()).unwrap(),
                u64::try_from(expected.as_nanos()).unwrap(),
                "retry_after",
            ),
            (Err(Denied::Never { burst: actual }), Err(Denied::Never { burst: expected })) => {
                assert_eq!(actual, expected, "case {case}: burst");
            },
            (actual, expected) => {
                panic!("case {case}: decision {actual:?}, expected {expected:?}")
            },
        }

        let (row_tat, row_seq, row_interval, row_burst): (i64, i64, i64, i32) = sqlx::query_as(
            "SELECT tat_ns, seq, emission_ns, burst FROM rate_limits WHERE limit_key = $1",
        )
        .bind(key.as_str())
        .fetch_one(&pool)
        .await
        .unwrap();
        close(
            u64::try_from(row_tat).unwrap(),
            written_lo.tat.min(i64::MAX as u64),
            "written tat",
        );
        assert_eq!(
            u64::from_ne_bytes(row_seq.to_ne_bytes()),
            written_lo.seq,
            "case {case}: written seq"
        );
        assert_eq!(
            Duration::from_nanos(u64::try_from(row_interval).unwrap()),
            enforced.emission_interval(),
            "case {case}: written interval"
        );
        assert_eq!(
            u32::try_from(row_burst).unwrap(),
            enforced.burst().get(),
            "case {case}: written burst"
        );
        checked += 1;
    }
    assert!(checked > 300, "too few decisive cases: {checked}");
}
