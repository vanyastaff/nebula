-- 025: Multi-Factor Authentication

CREATE TYPE mfa_method_type AS ENUM ('totp', 'webauthn', 'backup_code');

CREATE TABLE user_mfa_methods (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    method_type     mfa_method_type NOT NULL,
    name            VARCHAR(255),                           -- 'Personal YubiKey', 'Google Auth'
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    -- TOTP
    totp_secret     TEXT,                                   -- encrypted TOTP seed
    -- WebAuthn (Passkey)
    webauthn_credential_id  BYTEA,
    webauthn_public_key     BYTEA,
    webauthn_sign_count     BIGINT DEFAULT 0,
    webauthn_aaguid         UUID,
    webauthn_transports     TEXT[],
    -- Backup codes (hashed)
    backup_codes_hashes     TEXT[],
    backup_codes_used       INTEGER DEFAULT 0,
    last_used_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_mfa_methods_user ON user_mfa_methods(user_id);
