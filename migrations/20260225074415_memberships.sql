-- ============================================================
-- 003: Memberships, API Keys, Sessions
-- ============================================================

-- ============================================================
-- ORGANIZATION MEMBERS
-- ============================================================

CREATE TYPE org_member_role AS ENUM ('owner', 'admin', 'editor', 'viewer');

CREATE TABLE organization_members (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role            org_member_role NOT NULL DEFAULT 'viewer',
    invited_by      UUID REFERENCES users(id),
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, user_id)
);

CREATE INDEX idx_org_members_org ON organization_members(organization_id);
CREATE INDEX idx_org_members_user ON organization_members(user_id);

-- ============================================================
-- API KEYS
-- ============================================================

CREATE TABLE api_keys (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id         UUID REFERENCES users(id) ON DELETE SET NULL,
    name            VARCHAR(255) NOT NULL,
    key_hash        TEXT NOT NULL UNIQUE,                  -- bcrypt/sha256 hash
    key_prefix      VARCHAR(16) NOT NULL,                  -- first 8 chars for display: "nbk_xxxx"
    scopes          TEXT[] NOT NULL DEFAULT '{}',          -- ['workflows:read', 'executions:write']
    last_used_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_api_keys_org ON api_keys(organization_id);
CREATE INDEX idx_api_keys_hash ON api_keys(key_hash);

-- ============================================================
-- USER SESSIONS
-- ============================================================

CREATE TABLE user_sessions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash      TEXT NOT NULL UNIQUE,
    user_agent      TEXT,
    ip_address      INET,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sessions_user ON user_sessions(user_id);
CREATE INDEX idx_sessions_token ON user_sessions(token_hash);
CREATE INDEX idx_sessions_expires ON user_sessions(expires_at);
