-- ============================================================
-- 012: SSO / Identity Providers & SCIM Provisioning
-- ============================================================

-- ============================================================
-- SSO / IDENTITY PROVIDERS  (SAML 2.0 + OIDC)
-- ============================================================

CREATE TYPE sso_provider_type AS ENUM ('saml', 'oidc', 'ldap');

CREATE TABLE sso_providers (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            VARCHAR(128) NOT NULL,                  -- 'Okta Production', 'Azure AD'
    provider_type   sso_provider_type NOT NULL,
    is_active       BOOLEAN NOT NULL DEFAULT FALSE,         -- must be explicitly activated
    is_default      BOOLEAN NOT NULL DEFAULT FALSE,         -- redirect here if no hint

    -- OIDC config
    oidc_issuer_url     TEXT,
    oidc_client_id      TEXT,
    oidc_client_secret  TEXT,                              -- encrypted
    oidc_scopes         TEXT[] DEFAULT ARRAY['openid', 'email', 'profile'],
    oidc_extra_params   JSONB DEFAULT '{}',

    -- SAML config
    saml_entity_id      TEXT,
    saml_sso_url        TEXT,
    saml_certificate    TEXT,                              -- IdP public cert
    saml_sign_requests  BOOLEAN DEFAULT FALSE,

    -- LDAP config
    ldap_url            TEXT,
    ldap_bind_dn        TEXT,
    ldap_bind_password  TEXT,                              -- encrypted
    ldap_user_search_base TEXT,
    ldap_user_search_filter TEXT DEFAULT '(uid={username})',
    ldap_group_search_base TEXT,

    -- Attribute mapping (IdP claim -> Nebula field)
    attr_email      VARCHAR(128) DEFAULT 'email',
    attr_username   VARCHAR(128) DEFAULT 'preferred_username',
    attr_name       VARCHAR(128) DEFAULT 'name',
    attr_groups     VARCHAR(128) DEFAULT 'groups',          -- for group -> team sync

    -- Auto-provisioning
    auto_provision_users    BOOLEAN NOT NULL DEFAULT FALSE,
    default_org_role        VARCHAR(32) DEFAULT 'viewer',
    jit_project_mappings    JSONB DEFAULT '{}',            -- {"IdP-group": "project-id:role"}

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (organization_id, name)
);

CREATE INDEX idx_sso_providers_org ON sso_providers(organization_id);
CREATE UNIQUE INDEX idx_sso_default_per_org ON sso_providers(organization_id) WHERE is_default = TRUE;

-- SSO sessions (track active federation sessions for logout)
CREATE TABLE sso_sessions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    provider_id     UUID NOT NULL REFERENCES sso_providers(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id      UUID NOT NULL REFERENCES user_sessions(id) ON DELETE CASCADE,
    -- IdP-side identifiers for SLO (Single Logout)
    idp_session_id  TEXT,
    name_id         TEXT,                                   -- SAML NameID
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_sso_sessions_user ON sso_sessions(user_id);
CREATE INDEX idx_sso_sessions_idp ON sso_sessions(idp_session_id) WHERE idp_session_id IS NOT NULL;

-- ============================================================
-- SCIM PROVISIONING  (automated user sync from enterprise IdP)
-- ============================================================

CREATE TABLE scim_tokens (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    token_hash      TEXT NOT NULL UNIQUE,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    last_sync_at    TIMESTAMPTZ,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_scim_tokens_org ON scim_tokens(organization_id);

-- Track externally provisioned identities (IdP user <-> Nebula user mapping)
CREATE TABLE external_identities (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_id     UUID REFERENCES sso_providers(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    -- External IDs from IdP
    external_id     TEXT NOT NULL,                          -- IdP's stable subject ID
    external_email  TEXT,
    external_groups TEXT[] DEFAULT '{}',                    -- raw IdP groups for sync
    raw_attributes  JSONB DEFAULT '{}',                     -- full IdP claims (for debugging)
    last_synced_at  TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, external_id)
);

CREATE INDEX idx_ext_identities_user ON external_identities(user_id);
CREATE INDEX idx_ext_identities_org ON external_identities(organization_id);

-- SCIM group <-> Nebula team mapping (for automated team sync)
CREATE TABLE scim_group_mappings (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    external_group  TEXT NOT NULL,                          -- IdP group name or ID
    team_id         UUID REFERENCES teams(id) ON DELETE CASCADE,
    project_id      UUID REFERENCES projects(id) ON DELETE CASCADE,
    project_role    VARCHAR(64),                            -- auto-assign role when syncing
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (organization_id, external_group)
);

CREATE INDEX idx_scim_mappings_org ON scim_group_mappings(organization_id);
