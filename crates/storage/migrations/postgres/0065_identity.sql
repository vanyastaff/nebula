-- Identity: user accounts and their credentials of access — browser sessions,
-- personal access tokens, single-use verification tokens, OAuth login state,
-- linked external identities and pending MFA enrollments.
--
-- Transition migration of the database standard (docs/database-standard.md): replaces the
-- previous identity tables and the dead legacy tables that referenced users.

DROP TABLE IF EXISTS oauth_links, service_accounts CASCADE;
DROP TABLE IF EXISTS mfa_enrollment_candidates, external_identities, verification_tokens,
    sessions, personal_access_tokens, plane_a_oauth_states, users CASCADE;

CREATE TABLE users (
    id                  BYTEA       NOT NULL,
    email               TEXT        NOT NULL,
    email_verified_at   TIMESTAMPTZ,
    display_name        TEXT        NOT NULL,
    avatar_url          TEXT,
    password_hash       TEXT,
    created_at          TIMESTAMPTZ NOT NULL,
    last_login_at       TIMESTAMPTZ,
    locked_until        TIMESTAMPTZ,
    failed_login_count  INTEGER     NOT NULL DEFAULT 0,
    mfa_enabled         BOOLEAN     NOT NULL DEFAULT FALSE,
    mfa_secret_envelope BYTEA,
    version             BIGINT      NOT NULL DEFAULT 0,
    deleted_at          TIMESTAMPTZ,
    CONSTRAINT pk_users PRIMARY KEY (id),
    CONSTRAINT ck_users__id_length CHECK (octet_length(id) = 16),
    CONSTRAINT ck_users__failed_login_count CHECK (failed_login_count >= 0),
    CONSTRAINT ck_users__version CHECK (version >= 0),
    CONSTRAINT ck_users__mfa_secret_envelope_length
        CHECK (mfa_secret_envelope IS NULL OR octet_length(mfa_secret_envelope) BETWEEN 1 AND 4096)
);
-- An email identifies one active account, case-insensitively.
CREATE UNIQUE INDEX uq_users__email__live ON users (lower(email)) WHERE deleted_at IS NULL;
CREATE INDEX ix_users__locked_until__locked ON users (locked_until) WHERE locked_until IS NOT NULL;

-- Browser sessions: only the SHA-256 digest of the cookie token is stored.
CREATE TABLE sessions (
    token_digest   BYTEA       NOT NULL,
    user_id        BYTEA       NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    last_active_at TIMESTAMPTZ NOT NULL,
    expires_at     TIMESTAMPTZ NOT NULL,
    ip_address     INET,
    user_agent     TEXT,
    revoked_at     TIMESTAMPTZ,
    CONSTRAINT pk_sessions PRIMARY KEY (token_digest),
    CONSTRAINT fk_sessions__users FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT ck_sessions__token_digest_length CHECK (octet_length(token_digest) = 32)
);
CREATE INDEX ix_sessions__user_id__unrevoked ON sessions (user_id) WHERE revoked_at IS NULL;
CREATE INDEX ix_sessions__expires_at__unrevoked ON sessions (expires_at) WHERE revoked_at IS NULL;

-- Personal access tokens. The principal is a user or a service account, so it
-- is not a foreign key.
CREATE TABLE personal_access_tokens (
    id             BYTEA       NOT NULL,
    principal_kind TEXT        NOT NULL,
    principal_id   BYTEA       NOT NULL,
    name           TEXT        NOT NULL,
    prefix         TEXT        NOT NULL,
    hash           BYTEA       NOT NULL,
    scopes         JSONB       NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    last_used_at   TIMESTAMPTZ,
    expires_at     TIMESTAMPTZ,
    revoked_at     TIMESTAMPTZ,
    CONSTRAINT pk_personal_access_tokens PRIMARY KEY (id)
);
CREATE INDEX ix_personal_access_tokens__hash__unrevoked ON personal_access_tokens (hash)
    WHERE revoked_at IS NULL;
CREATE INDEX ix_personal_access_tokens__principal_kind_principal_id
    ON personal_access_tokens (principal_kind, principal_id);

-- Single-use email verification and password-reset tokens (digest only).
CREATE TABLE verification_tokens (
    token_hash  BYTEA       NOT NULL,
    user_id     BYTEA       NOT NULL,
    kind        TEXT        NOT NULL,
    payload     JSONB,
    created_at  TIMESTAMPTZ NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    CONSTRAINT pk_verification_tokens PRIMARY KEY (token_hash),
    CONSTRAINT fk_verification_tokens__users
        FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
);
CREATE INDEX ix_verification_tokens__user_id_kind__unconsumed ON verification_tokens (user_id, kind)
    WHERE consumed_at IS NULL;
CREATE INDEX ix_verification_tokens__expires_at__unconsumed ON verification_tokens (expires_at)
    WHERE consumed_at IS NULL;

-- Single-use PKCE state of an OAuth login between redirect and callback.
CREATE TABLE oauth_states (
    state         TEXT        NOT NULL,
    provider      TEXT        NOT NULL,
    code_verifier TEXT        NOT NULL,
    redirect_uri  TEXT,
    created_at    TIMESTAMPTZ NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    consumed_at   TIMESTAMPTZ,
    CONSTRAINT pk_oauth_states PRIMARY KEY (state)
);
CREATE INDEX ix_oauth_states__expires_at ON oauth_states (expires_at);

-- The authoritative link from an OAuth provider subject to a user.
CREATE TABLE external_identities (
    provider  TEXT        NOT NULL,
    subject   TEXT        NOT NULL,
    user_id   BYTEA       NOT NULL,
    email     TEXT,
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT pk_external_identities PRIMARY KEY (provider, subject),
    CONSTRAINT fk_external_identities__users
        FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
);
CREATE INDEX ix_external_identities__user_id ON external_identities (user_id);

-- At most one expiring MFA enrollment candidate per user, separate from the
-- active factor on `users`.
CREATE TABLE mfa_enrollment_candidates (
    user_id         BYTEA       NOT NULL,
    enrollment_id   BYTEA       NOT NULL,
    secret_envelope BYTEA       NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    CONSTRAINT pk_mfa_enrollment_candidates PRIMARY KEY (user_id),
    CONSTRAINT uq_mfa_enrollment_candidates__enrollment_id UNIQUE (enrollment_id),
    CONSTRAINT fk_mfa_enrollment_candidates__users
        FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT ck_mfa_enrollment_candidates__user_id_length CHECK (octet_length(user_id) = 16),
    CONSTRAINT ck_mfa_enrollment_candidates__enrollment_id_length
        CHECK (octet_length(enrollment_id) = 32),
    CONSTRAINT ck_mfa_enrollment_candidates__secret_envelope_length
        CHECK (octet_length(secret_envelope) BETWEEN 1 AND 4096),
    CONSTRAINT ck_mfa_enrollment_candidates__expiry CHECK (created_at < expires_at)
);
CREATE INDEX ix_mfa_enrollment_candidates__expires_at ON mfa_enrollment_candidates (expires_at);
