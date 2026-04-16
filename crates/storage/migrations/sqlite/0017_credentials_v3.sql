-- 0017: Credential system v3 extensions
-- Spec: 22 (credential-system)

ALTER TABLE credentials ADD COLUMN envelope TEXT;        -- JSON: {kek_id, encrypted_dek, algorithm, nonce, aad_digest}
ALTER TABLE credentials ADD COLUMN state_kind TEXT NOT NULL DEFAULT 'active';
ALTER TABLE credentials ADD COLUMN lease_id TEXT;
ALTER TABLE credentials ADD COLUMN expires_at TEXT;

CREATE INDEX idx_credentials_expiring
    ON credentials (expires_at)
    WHERE expires_at IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE pending_credentials (
    id              BLOB PRIMARY KEY,
    org_id          BLOB NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    workspace_id    BLOB REFERENCES workspaces(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    state_encrypted BLOB NOT NULL,
    initiated_by    BLOB NOT NULL,
    created_at      TEXT NOT NULL,
    expires_at      TEXT NOT NULL
);

CREATE INDEX idx_pending_credentials_cleanup
    ON pending_credentials (expires_at);

CREATE TABLE credential_audit (
    id              BLOB PRIMARY KEY,
    org_id          BLOB NOT NULL,
    credential_id   BLOB NOT NULL,
    seq             INTEGER NOT NULL,
    principal_kind  TEXT NOT NULL,
    principal_id    BLOB,
    operation       TEXT NOT NULL,
    result          TEXT NOT NULL,
    detail          TEXT,                            -- JSON
    prev_hmac       BLOB,
    self_hmac       BLOB NOT NULL,
    emitted_at      TEXT NOT NULL
);

CREATE INDEX idx_credential_audit_by_cred
    ON credential_audit (credential_id, seq);

CREATE INDEX idx_credential_audit_by_org
    ON credential_audit (org_id, emitted_at DESC);
