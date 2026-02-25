-- ============================================================
-- 013: Tags, MFA, Invitations, Project Variables
-- ============================================================

-- ============================================================
-- TAGS  (instance-scoped labels, NOT RBAC'd — like n8n tags)
-- ============================================================

CREATE TABLE tags (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            VARCHAR(128) NOT NULL,
    color           VARCHAR(16),
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, name)
);

CREATE TABLE workflow_tags (
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    tag_id      UUID NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (workflow_id, tag_id)
);

CREATE INDEX idx_workflow_tags_tag ON workflow_tags(tag_id);

-- ============================================================
-- MFA — TOTP + WebAuthn  (enterprise security)
-- ============================================================

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

-- ============================================================
-- INVITATIONS  (pending email invites)
-- ============================================================

CREATE TYPE invitation_type AS ENUM ('organization', 'project', 'team');

CREATE TABLE invitations (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    invited_email   VARCHAR(320) NOT NULL,
    invited_user_id UUID REFERENCES users(id) ON DELETE SET NULL,  -- NULL if user doesn't exist yet
    invitation_type invitation_type NOT NULL DEFAULT 'organization',
    -- For project/team invites
    project_id      UUID REFERENCES projects(id) ON DELETE CASCADE,
    team_id         UUID REFERENCES teams(id) ON DELETE CASCADE,
    -- Role to assign on accept
    org_role        VARCHAR(32),
    project_role    VARCHAR(64),
    custom_role_id  UUID REFERENCES roles(id) ON DELETE SET NULL,
    -- Invite lifecycle
    token           TEXT NOT NULL UNIQUE,                   -- secure random token
    invited_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    accepted_at     TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '7 days',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_invitations_org ON invitations(organization_id);
CREATE INDEX idx_invitations_email ON invitations(invited_email);
CREATE INDEX idx_invitations_token ON invitations(token);
CREATE INDEX idx_invitations_pending ON invitations(expires_at)
    WHERE accepted_at IS NULL;

-- ============================================================
-- PROJECT VARIABLES  (per-project, separate from tenant-level vars)
-- ============================================================

CREATE TABLE project_variables (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key             VARCHAR(255) NOT NULL,
    value           TEXT NOT NULL,
    is_secret       BOOLEAN NOT NULL DEFAULT FALSE,
    description     TEXT,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_id, key)
);

CREATE INDEX idx_project_vars_project ON project_variables(project_id);
