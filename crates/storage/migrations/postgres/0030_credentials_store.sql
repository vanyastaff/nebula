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
-- * `data`      — BYTEA, byte-exact ciphertext from EncryptionLayer; the store
--                 never inspects or decrypts it.
-- * `version`   — CAS counter (u64 stored as BIGINT; guard the i64 boundary
--                 at the Rust layer — see PgCredentialStore).
-- * timestamps  — native TIMESTAMPTZ (Postgres has a proper instant type, so
--                 the SQLite millis-INTEGER workaround is unnecessary here).
-- * `owner_id`  — extracted from metadata["owner_id"] at write time; NULL for
--                 admin/global credentials.
-- * `name`      — user-facing label; UNIQUE per owner enforced by partial index.
-- * `metadata`  — JSON text for display_name, icon, sharing, tags; never queried
--                 by this store (the EncryptionLayer/CacheLayer sit above us).
CREATE TABLE credentials (
    id              TEXT        NOT NULL PRIMARY KEY,
    name            TEXT,
    owner_id        TEXT,
    credential_key  TEXT        NOT NULL,
    state_kind      TEXT        NOT NULL,
    state_version   BIGINT      NOT NULL,
    data            BYTEA       NOT NULL,
    version         BIGINT      NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ,
    reauth_required BOOLEAN     NOT NULL DEFAULT FALSE,
    metadata        TEXT        NOT NULL DEFAULT '{}'
);

-- Partial unique index: two credentials belonging to the same owner may not
-- share a name, but unnamed credentials (NULL) are unconstrained.
CREATE UNIQUE INDEX idx_credentials_owner_name
    ON credentials(owner_id, name)
    WHERE name IS NOT NULL;

CREATE INDEX idx_credentials_state_kind
    ON credentials(state_kind);

-- Sparse index: only rows with an expiry participate, keeping the index small.
CREATE INDEX idx_credentials_expiring
    ON credentials(expires_at)
    WHERE expires_at IS NOT NULL;
