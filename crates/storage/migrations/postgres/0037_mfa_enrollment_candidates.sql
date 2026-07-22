-- MFA enrollment candidates are intentionally separate from the active user factor.
-- Starting/replacing enrollment cannot weaken an already active factor; only
-- the storage-owned atomic install operation may promote a live candidate.

CREATE TABLE IF NOT EXISTS mfa_enrollment_candidates (
    user_id          BYTEA PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    enrollment_id    BYTEA NOT NULL UNIQUE,
    secret_envelope  BYTEA NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,
    CONSTRAINT chk_mfa_enrollment_id_length CHECK (octet_length(enrollment_id) = 32),
    CONSTRAINT chk_mfa_enrollment_secret_nonempty CHECK (octet_length(secret_envelope) > 0),
    CONSTRAINT chk_mfa_enrollment_expiry CHECK (created_at < expires_at)
);

CREATE INDEX IF NOT EXISTS idx_mfa_enrollment_candidates_expiry
    ON mfa_enrollment_candidates (expires_at);

COMMENT ON TABLE mfa_enrollment_candidates IS
    'Expiring, single-use Plane-A MFA candidates; never the active factor authority';
COMMENT ON COLUMN mfa_enrollment_candidates.secret_envelope IS
    'Opaque identity-secret envelope bytes; encoding is owned above the storage contract';
