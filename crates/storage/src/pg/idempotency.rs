//! Postgres implementation of [`IdempotencyStoreRepo`] (M3.4 / ADR-0048).
//!
//! Schema: migration `0024_add_idempotency_dedup.sql`.
//!
//! Concurrency contract:
//!
//! - `put` uses `INSERT ... ON CONFLICT (cache_key) DO NOTHING` so two
//!   concurrent first writers see the same first-writer-wins semantics
//!   the in-memory backend has via `moka`'s `entry().or_insert_with`.
//! - The middleware does not retry — a racer's `INSERT` is a no-op and
//!   the next `get` from a retried caller hits the winner's row.
//! - `evict_expired` is the maintenance sweep, called from a startup
//!   background task on the cadence configured by
//!   `IdempotencyApiConfig::sweep_interval_secs`.
//!
//! Headers encoding: length-prefixed list
//! `<u16 count> [<u16 name_len><name_bytes><u32 value_len><value_bytes>]*`.
//! Length fields are big-endian. Decode failures surface as
//! [`StorageError::Serialization`] — never silently dropped.

use std::time::Duration;

use async_trait::async_trait;
use sqlx::{Pool, Postgres};

use crate::{
    error::StorageError,
    pg::map_db_err,
    repos::{CachedRecord, IdempotencyStoreRepo},
};

/// Postgres-backed durable dedup store (canon §M3.4 / ADR-0048).
#[derive(Clone, Debug)]
pub struct PgIdempotencyStore {
    pool: Pool<Postgres>,
}

impl PgIdempotencyStore {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IdempotencyStoreRepo for PgIdempotencyStore {
    async fn get(&self, cache_key: &str) -> Result<Option<CachedRecord>, StorageError> {
        let row: Option<(i16, Vec<u8>, Vec<u8>, Vec<u8>)> = sqlx::query_as(
            "SELECT status, headers, body, fingerprint \
             FROM api_idempotency_dedup \
             WHERE cache_key = $1 AND expires_at > NOW()",
        )
        .bind(cache_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| map_db_err("idempotency_dedup", err))?;

        let Some((status_i16, headers_blob, body, fingerprint_blob)) = row else {
            return Ok(None);
        };

        let status = u16::try_from(status_i16).map_err(|_| {
            StorageError::Serialization(format!(
                "api_idempotency_dedup.status out of u16 range: {status_i16} \
                 (cache_key={cache_key})"
            ))
        })?;
        let headers = decode_headers(&headers_blob).map_err(|err| {
            StorageError::Serialization(format!(
                "api_idempotency_dedup.headers decode failed (cache_key={cache_key}): {err}"
            ))
        })?;
        let fingerprint: [u8; 32] = fingerprint_blob.as_slice().try_into().map_err(|_| {
            StorageError::Serialization(format!(
                "api_idempotency_dedup.fingerprint length != 32 \
                 (cache_key={cache_key}, len={})",
                fingerprint_blob.len()
            ))
        })?;

        Ok(Some(CachedRecord {
            status,
            headers,
            body,
            fingerprint,
        }))
    }

    async fn put(
        &self,
        cache_key: String,
        record: CachedRecord,
        ttl: Duration,
    ) -> Result<(), StorageError> {
        let headers_blob = encode_headers(&record.headers);
        let expires_at = chrono::Utc::now()
            + chrono::Duration::from_std(ttl).map_err(|err| {
                StorageError::Configuration(format!("ttl out of chrono::Duration range: {err}"))
            })?;
        let status_i16 = i16::try_from(record.status).map_err(|_| {
            StorageError::Serialization(format!(
                "status out of i16 range: {} (cache_key={cache_key})",
                record.status
            ))
        })?;
        sqlx::query(
            "INSERT INTO api_idempotency_dedup \
             (cache_key, status, headers, body, fingerprint, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (cache_key) DO NOTHING",
        )
        .bind(&cache_key)
        .bind(status_i16)
        .bind(&headers_blob)
        .bind(&record.body)
        .bind(record.fingerprint.as_slice())
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|err| map_db_err("idempotency_dedup", err))?;
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        let result = sqlx::query("DELETE FROM api_idempotency_dedup WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await
            .map_err(|err| map_db_err("idempotency_dedup", err))?;
        let rows = result.rows_affected();
        if rows > 0 {
            tracing::info!(
                rows_evicted = rows,
                "idempotency: PG sweep evicted expired rows"
            );
        }
        Ok(rows)
    }
}

// ── Header codec ─────────────────────────────────────────────────────────────

/// Encode a header list as
/// `<u16 count> [<u16 name_len><name_bytes><u32 value_len><value_bytes>]*`
/// (big-endian length prefixes).
fn encode_headers(headers: &[(String, Vec<u8>)]) -> Vec<u8> {
    let count = u16::try_from(headers.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(
        2 + headers
            .iter()
            .map(|(n, v)| 6 + n.len() + v.len())
            .sum::<usize>(),
    );
    out.extend_from_slice(&count.to_be_bytes());
    for (name, value) in headers.iter().take(count as usize) {
        let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&name_len.to_be_bytes());
        let name_truncated = &name.as_bytes()[..name_len as usize];
        out.extend_from_slice(name_truncated);
        let value_len = u32::try_from(value.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&value_len.to_be_bytes());
        let value_truncated = &value[..value_len as usize];
        out.extend_from_slice(value_truncated);
    }
    out
}

/// Decode the header blob written by [`encode_headers`].
///
/// Returns an error string if the blob is truncated or any length
/// prefix overruns the buffer. Caller wraps in
/// [`StorageError::Serialization`].
fn decode_headers(buf: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    if buf.len() < 2 {
        return Err(format!(
            "buf too short for count prefix: {} bytes",
            buf.len()
        ));
    }
    let count = u16::from_be_bytes([buf[0], buf[1]]);
    let mut headers = Vec::with_capacity(count as usize);
    let mut cursor = 2usize;
    for _ in 0..count {
        if cursor + 2 > buf.len() {
            return Err("truncated name length".to_string());
        }
        let name_len = u16::from_be_bytes([buf[cursor], buf[cursor + 1]]) as usize;
        cursor += 2;
        if cursor + name_len > buf.len() {
            return Err("truncated name bytes".to_string());
        }
        let name = std::str::from_utf8(&buf[cursor..cursor + name_len])
            .map_err(|err| format!("non-UTF-8 header name: {err}"))?
            .to_owned();
        cursor += name_len;

        if cursor + 4 > buf.len() {
            return Err("truncated value length".to_string());
        }
        let value_len = u32::from_be_bytes([
            buf[cursor],
            buf[cursor + 1],
            buf[cursor + 2],
            buf[cursor + 3],
        ]) as usize;
        cursor += 4;
        if cursor + value_len > buf.len() {
            return Err("truncated value bytes".to_string());
        }
        let value = buf[cursor..cursor + value_len].to_vec();
        cursor += value_len;

        headers.push((name, value));
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_codec_roundtrips_typical_set() {
        let headers = vec![
            ("content-type".to_string(), b"application/json".to_vec()),
            ("x-request-id".to_string(), b"req-abc-123".to_vec()),
            ("x-binary".to_string(), vec![0u8, 1, 2, 0xff]),
        ];
        let blob = encode_headers(&headers);
        let decoded = decode_headers(&blob).expect("must decode");
        assert_eq!(decoded, headers);
    }

    #[test]
    fn header_codec_roundtrips_empty() {
        let headers: Vec<(String, Vec<u8>)> = Vec::new();
        let blob = encode_headers(&headers);
        let decoded = decode_headers(&blob).expect("must decode empty");
        assert!(decoded.is_empty());
    }

    #[test]
    fn header_codec_rejects_truncated_blob() {
        // Claim count=1 but no payload
        let bad = vec![0u8, 1];
        assert!(decode_headers(&bad).is_err());
    }
}
