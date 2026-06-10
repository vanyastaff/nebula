-- Migration 0030: durable credential store (Model A)
--
-- Drop Model B's never-populated tables (credentials / pending_credentials /
-- credential_audit from migrations 0008 and 0017). They have zero rows, zero
-- FK dependents, and are not referenced by schema.sql. Removing them before
-- creating the new `credentials` table avoids a name collision and leaves the
-- schema consistent with the A-plus decision recorded in ADR-0088.
DROP TABLE IF EXISTS credential_audit;
DROP TABLE IF EXISTS pending_credentials;
DROP TABLE IF EXISTS credentials;

-- Model A durable store — opaque ciphertext blob, identity columns queryable.
--
-- * `data`      — BLOB, byte-exact ciphertext from EncryptionLayer; the store
--                 never inspects or decrypts it.
-- * `version`   — CAS counter (u64 stored as INTEGER; guard the i64 boundary
--                 at the Rust layer — see SqliteCredentialStore).
-- * timestamps  — INTEGER milliseconds-since-epoch (UTC). Lexicographic
--                 ordering of RFC-3339 text is fragile across chrono versions
--                 (conditional fractional seconds, ±HH:MM vs Z suffix); integer
--                 ordering is unambiguous for expiry predicates and ordering.
-- * `owner_id`  — extracted from metadata["owner_id"] at write time; NULL for
--                 admin/global credentials.
-- * `name`      — user-facing label; UNIQUE per owner enforced by partial index.
-- * `metadata`  — JSON blob for display_name, icon, sharing, tags; never
--                 queried by this store (the EncryptionLayer/CacheLayer sit
--                 above us and do their own reads).
CREATE TABLE credentials (
    id              TEXT    NOT NULL PRIMARY KEY,
    name            TEXT,
    owner_id        TEXT,
    credential_key  TEXT    NOT NULL,
    state_kind      TEXT    NOT NULL,
    state_version   INTEGER NOT NULL,
    data            BLOB    NOT NULL,
    version         INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    expires_at      INTEGER,
    reauth_required INTEGER NOT NULL DEFAULT 0,
    metadata        TEXT    NOT NULL DEFAULT '{}'
);

-- Partial unique index: two credentials belonging to the same owner may not
-- share a name, but unnamed credentials (NULL) are unconstrained.
-- NULL != NULL in SQL, so rows with name IS NULL are excluded from uniqueness
-- checks without this WHERE clause — the WHERE makes the intent explicit.
CREATE UNIQUE INDEX idx_credentials_owner_name
    ON credentials(owner_id, name)
    WHERE name IS NOT NULL;

CREATE INDEX idx_credentials_state_kind
    ON credentials(state_kind);

-- Sparse index: only rows with an expiry participate, keeping the index small.
CREATE INDEX idx_credentials_expiring
    ON credentials(expires_at)
    WHERE expires_at IS NOT NULL;
