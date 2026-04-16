-- 0017: Credential system v3 extensions
-- Spec: 22 (credential-system)
--
-- Adds envelope encryption metadata, credential lifecycle state,
-- dynamic secret lease tracking, and pending interactive flows.

-- ── Envelope encryption metadata ───────────────────────────
-- Stored alongside encrypted_secret; allows key rotation
-- and algorithm migration without re-encrypting all secrets at once.

ALTER TABLE credentials ADD COLUMN envelope JSONB;       -- {kek_id, encrypted_dek, algorithm, nonce, aad_digest}

-- ── Credential lifecycle state ─────────────────────────────
-- Tracks whether a credential is active, refreshing, expired, etc.

ALTER TABLE credentials ADD COLUMN state_kind TEXT NOT NULL DEFAULT 'active';  -- 'active'/'refreshing'/'expired'/'revoked'/'suspended'

-- ── Dynamic secrets ────────────────────────────────────────
-- For credentials that produce time-limited leased material
-- (e.g., Vault dynamic DB creds, AWS STS tokens).

ALTER TABLE credentials ADD COLUMN lease_id TEXT;         -- external lease identifier
ALTER TABLE credentials ADD COLUMN expires_at TIMESTAMPTZ; -- lease expiry

CREATE INDEX idx_credentials_expiring
    ON credentials (expires_at)
    WHERE expires_at IS NOT NULL AND deleted_at IS NULL;

-- ── Pending credential flows ───────────────────────────────
-- Stores in-progress interactive OAuth2/OIDC flows so they
-- survive process restarts. Cleaned up after completion or timeout.

CREATE TABLE pending_credentials (
    id              BYTEA PRIMARY KEY,               -- ULID
    org_id          BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id    BYTEA REFERENCES workspaces(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,                   -- credential type being created
    state_encrypted BYTEA NOT NULL,                  -- encrypted PendingState (PKCE, nonce, etc.)
    initiated_by    BYTEA NOT NULL,                  -- user who started the flow
    created_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL             -- auto-cleanup after timeout
);

CREATE INDEX idx_pending_credentials_cleanup
    ON pending_credentials (expires_at);

-- ── Credential audit trail (tamper-evident) ────────────────
-- Append-only log with HMAC hash chain for compliance.
-- Each row includes the HMAC of the previous row, making
-- tampering detectable.

CREATE TABLE credential_audit (
    id              BYTEA PRIMARY KEY,               -- ULID
    org_id          BYTEA NOT NULL,
    credential_id   BYTEA NOT NULL,                  -- may reference deleted credential
    seq             BIGINT NOT NULL,                 -- per-credential monotonic
    principal_kind  TEXT NOT NULL,
    principal_id    BYTEA,
    operation       TEXT NOT NULL,                   -- 'created'/'rotated'/'refreshed'/'revoked'/'accessed'/'deleted'
    result          TEXT NOT NULL,                   -- 'success'/'failure'
    detail          JSONB,
    prev_hmac       BYTEA,                           -- HMAC of previous entry (NULL for first)
    self_hmac       BYTEA NOT NULL,                  -- HMAC of this entry (hash chain anchor)
    emitted_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_credential_audit_by_cred
    ON credential_audit (credential_id, seq);

CREATE INDEX idx_credential_audit_by_org
    ON credential_audit (org_id, emitted_at DESC);
