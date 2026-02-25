-- 023: SCIM Provisioning
-- ============================================================

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
