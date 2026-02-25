-- ============================================================
-- 008: Resources
-- ============================================================

CREATE TYPE resource_lifecycle AS ENUM ('global', 'workflow', 'execution', 'action');
CREATE TYPE resource_status AS ENUM ('healthy', 'degraded', 'unhealthy', 'unknown');

-- ============================================================
-- RESOURCE DEFINITIONS
-- ============================================================

CREATE TABLE resources (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    credential_id   UUID REFERENCES credentials(id) ON DELETE SET NULL,
    resource_type   VARCHAR(128) NOT NULL,                 -- 'database', 'cache', 'message_queue', etc.
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    lifecycle       resource_lifecycle NOT NULL DEFAULT 'global',
    config          JSONB NOT NULL DEFAULT '{}',           -- pool size, timeouts, etc.

    -- Connection pool config
    pool_min_size   INTEGER NOT NULL DEFAULT 1,
    pool_max_size   INTEGER NOT NULL DEFAULT 10,
    connect_timeout_ms  INTEGER NOT NULL DEFAULT 5000,
    idle_timeout_ms     INTEGER NOT NULL DEFAULT 60000,

    -- Current state
    status          resource_status NOT NULL DEFAULT 'unknown',
    last_health_check_at TIMESTAMPTZ,
    health_error    TEXT,

    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (tenant_id, name)
);

CREATE INDEX idx_resources_tenant ON resources(tenant_id);
CREATE INDEX idx_resources_type ON resources(resource_type);
CREATE INDEX idx_resources_status ON resources(status);

-- ============================================================
-- RESOURCE HEALTH CHECKS (history)
-- ============================================================

CREATE TABLE resource_health_checks (
    id              BIGSERIAL PRIMARY KEY,
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    status          resource_status NOT NULL,
    latency_ms      INTEGER,
    error_message   TEXT,
    checked_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_health_checks_resource ON resource_health_checks(resource_id);
CREATE INDEX idx_health_checks_checked ON resource_health_checks(checked_at DESC);

-- Retain only 7 days of health check history
CREATE OR REPLACE FUNCTION cleanup_old_health_checks() RETURNS void AS $$
BEGIN
    DELETE FROM resource_health_checks
    WHERE checked_at < NOW() - INTERVAL '7 days';
END;
$$ LANGUAGE plpgsql;
