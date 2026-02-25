-- ============================================================
-- 002: Users
-- ============================================================

CREATE TABLE users (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email           VARCHAR(320) NOT NULL UNIQUE,
    username        VARCHAR(64) NOT NULL UNIQUE,
    display_name    VARCHAR(255),
    avatar_url      TEXT,
    password_hash   TEXT,                                  -- NULL for SSO-only users
    role            VARCHAR(32) NOT NULL DEFAULT 'user',   -- user | admin | superadmin
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    is_verified     BOOLEAN NOT NULL DEFAULT FALSE,
    last_login_at   TIMESTAMPTZ,
    settings        JSONB NOT NULL DEFAULT '{}',           -- UI preferences, locale, theme
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_role ON users(role);
