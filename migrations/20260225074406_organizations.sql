-- ============================================================
-- 001: Organizations
-- ============================================================

CREATE TABLE organizations (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    slug            VARCHAR(64) NOT NULL UNIQUE,
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    avatar_url      TEXT,
    plan            VARCHAR(32) NOT NULL DEFAULT 'free',  -- free | pro | enterprise
    settings        JSONB NOT NULL DEFAULT '{}',
    max_workflows   INTEGER NOT NULL DEFAULT 10,
    max_executions  INTEGER NOT NULL DEFAULT 1000,        -- per month
    max_members     INTEGER NOT NULL DEFAULT 5,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_organizations_slug ON organizations(slug);
CREATE INDEX idx_organizations_plan ON organizations(plan);
