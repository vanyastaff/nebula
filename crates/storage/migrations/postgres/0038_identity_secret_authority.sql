-- Plane-A identity authorities are persisted only as one-way lookup digests
-- (browser sessions) or authenticated-encryption envelopes (TOTP seeds).
--
-- Session rows from earlier builds contain the raw cookie bearer in `id`.
-- They cannot be transformed without retaining that authority, so this
-- intentional pre-1.0 breaking migration invalidates every existing session.
-- Users authenticate again and receive a fresh 256-bit cookie whose digest is
-- the only value written to this table.

TRUNCATE TABLE sessions;

ALTER TABLE sessions
    RENAME COLUMN id TO token_digest;

ALTER TABLE sessions
    ADD CONSTRAINT chk_sessions_token_digest_length
    CHECK (octet_length(token_digest) = 32);

COMMENT ON COLUMN sessions.token_digest IS
    'SHA-256("nebula:plane-a:session-cookie:v1\\0" || presented cookie token); plaintext bearer is never persisted';

-- Existing non-NULL values are legacy Base32 TOTP seeds. The first-party
-- composition root must run the bounded, advisory-lock-serialized identity
-- secret migrator before exposing the PG auth backend. It converts these
-- bytes in place to nebula-crypto EncryptedData v1, verifies every existing
-- envelope under user-bound Active AAD, and fails boot unless the legacy
-- count reaches zero. Keeping one column makes the conversion crash-resumable
-- without runtime DDL or a second plaintext-bearing staging column.

ALTER TABLE users
    RENAME COLUMN mfa_secret TO mfa_secret_envelope;

-- These bounds are compatible with both the historical canonical 32-byte
-- Base32 seed and the new JSON envelope, so all DDL remains in the numbered
-- migration while startup data conversion stays memory-bounded.
ALTER TABLE users
    ADD CONSTRAINT chk_users_identity_id_length
        CHECK (octet_length(id) = 16),
    ADD CONSTRAINT chk_users_mfa_secret_envelope_bounds
        CHECK (
            mfa_secret_envelope IS NULL
            OR octet_length(mfa_secret_envelope) BETWEEN 1 AND 4096
        );

ALTER TABLE mfa_enrollment_candidates
    ADD CONSTRAINT chk_mfa_enrollment_user_id_length
        CHECK (octet_length(user_id) = 16),
    ADD CONSTRAINT chk_mfa_enrollment_secret_envelope_bounds
        CHECK (octet_length(secret_envelope) BETWEEN 1 AND 4096);

COMMENT ON COLUMN users.mfa_secret_envelope IS
    'EncryptedData v1 AES-256-GCM TOTP seed; AAD binds active-factor purpose and users.id; startup refuses residual legacy plaintext';

COMMENT ON COLUMN mfa_enrollment_candidates.secret_envelope IS
    'EncryptedData v1 AES-256-GCM pending TOTP seed; AAD binds enrollment-candidate purpose and users.id';
