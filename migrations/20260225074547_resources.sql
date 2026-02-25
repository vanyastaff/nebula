-- ============================================================
-- Resources: Full Schema
-- Includes: resource definitions, pool config, health checks,
--           quarantine, dependencies, events, hooks, metrics
-- ============================================================

-- ============================================================
-- TYPES
-- ============================================================

CREATE TYPE resource_lifecycle AS ENUM (
    'global',
    'workflow',
    'execution',
    'action'
);

CREATE TYPE resource_status AS ENUM (
    'healthy',
    'degraded',
    'unhealthy',
    'unknown'
);

CREATE TYPE resource_pool_strategy AS ENUM (
    'fifo',
    'lifo'
);

CREATE TYPE resource_event_type AS ENUM (
    'acquired',
    'released',
    'pool_exhausted',
    'cleaned_up',
    'health_changed',
    'quarantined',
    'quarantine_released',
    'error'
);

CREATE TYPE resource_cleanup_reason AS ENUM (
    'expired',
    'shutdown',
    'recycle_failed',
    'evicted'
);

CREATE TYPE resource_hook_filter AS ENUM (
    'all',
    'resource',
    'prefix'
);

-- ============================================================
-- RESOURCE DEFINITIONS
-- ============================================================

CREATE TABLE resources (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    credential_id   UUID REFERENCES credentials(id) ON DELETE SET NULL,

    -- Identity
    resource_type   VARCHAR(128) NOT NULL,  -- 'database', 'cache', 'http', 'storage', etc.
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    lifecycle       resource_lifecycle NOT NULL DEFAULT 'global',

    -- Arbitrary connection config (host, port, db name, etc.)
    config          JSONB NOT NULL DEFAULT '{}',

    -- ── Pool configuration ──────────────────────────────────
    pool_strategy           resource_pool_strategy NOT NULL DEFAULT 'fifo',
    pool_min_size           INTEGER NOT NULL DEFAULT 1  CHECK (pool_min_size >= 0),
    pool_max_size           INTEGER NOT NULL DEFAULT 10 CHECK (pool_max_size > 0),
    acquire_timeout_ms      INTEGER NOT NULL DEFAULT 5000,
    connect_timeout_ms      INTEGER NOT NULL DEFAULT 5000,
    idle_timeout_ms         INTEGER NOT NULL DEFAULT 60000,
    max_lifetime_ms         INTEGER NOT NULL DEFAULT 1800000,
    maintenance_interval_ms INTEGER,                      -- NULL = disabled

    -- ── Health check configuration ──────────────────────────
    check_interval_ms       INTEGER NOT NULL DEFAULT 30000,
    check_timeout_ms        INTEGER NOT NULL DEFAULT 5000,
    failure_threshold       INTEGER NOT NULL DEFAULT 3,

    -- ── Current runtime state (updated by worker) ───────────
    status                  resource_status NOT NULL DEFAULT 'unknown',
    performance_impact      NUMERIC(4,3),                 -- 0.0–1.0, set when Degraded
    health_error            TEXT,
    consecutive_failures    INTEGER NOT NULL DEFAULT 0,
    last_health_check_at    TIMESTAMPTZ,
    pool_active             INTEGER NOT NULL DEFAULT 0,
    pool_idle               INTEGER NOT NULL DEFAULT 0,

    -- ── Lifetime statistics (monotonically increasing) ──────
    acquire_count           BIGINT NOT NULL DEFAULT 0,
    release_count           BIGINT NOT NULL DEFAULT 0,
    exhausted_count         BIGINT NOT NULL DEFAULT 0,
    error_count             BIGINT NOT NULL DEFAULT 0,
    avg_usage_duration_ms   INTEGER,

    -- ── Meta ────────────────────────────────────────────────
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (tenant_id, name),
    CHECK (pool_min_size <= pool_max_size)
);

CREATE INDEX idx_resources_tenant   ON resources(tenant_id);
CREATE INDEX idx_resources_type     ON resources(resource_type);
CREATE INDEX idx_resources_status   ON resources(status);
CREATE INDEX idx_resources_lifecycle ON resources(lifecycle);
CREATE INDEX idx_resources_active   ON resources(tenant_id, is_active) WHERE is_active = TRUE;

-- ============================================================
-- RESOURCE DEPENDENCIES
-- Used for: Topology page, health cascade (Unhealthy → Degraded)
-- ============================================================

CREATE TABLE resource_dependencies (
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    depends_on_id   UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    PRIMARY KEY (resource_id, depends_on_id),
    CHECK (resource_id != depends_on_id)
);

-- Both directions indexed: "what does X depend on" and "who depends on X"
CREATE INDEX idx_resource_deps_resource ON resource_dependencies(resource_id);
CREATE INDEX idx_resource_deps_on       ON resource_dependencies(depends_on_id);

-- ============================================================
-- RESOURCE HEALTH CHECKS (history)
-- Used for: Health → Check History sparkline (last 20), Health → Pipeline stages
-- Retained for 7 days.
-- ============================================================

CREATE TABLE resource_health_checks (
    id              BIGSERIAL PRIMARY KEY,
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,

    status          resource_status NOT NULL,
    latency_ms      INTEGER,
    error_message   TEXT,

    -- Per-pipeline-stage breakdown
    -- NULL means the whole check result, not a specific stage
    stage                   VARCHAR(64),        -- 'ConnectivityStage' | 'PerformanceStage' | 'is_valid'
    performance_impact      NUMERIC(4,3),       -- 0.0–1.0, populated for Degraded results
    consecutive_failures_at INTEGER,            -- value of consecutive_failures at check time

    checked_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_health_checks_resource ON resource_health_checks(resource_id, checked_at DESC);
CREATE INDEX idx_health_checks_checked  ON resource_health_checks(checked_at DESC);

-- ============================================================
-- RESOURCE QUARANTINE
-- Used for: Health → Quarantine block, quarantine badge on cards
-- One active quarantine record per resource (released_at IS NULL = active).
-- ============================================================

CREATE TABLE resource_quarantine (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,

    -- Why it was quarantined
    reason          TEXT NOT NULL,              -- 'HealthCheckFailed (5 consecutive)'
    quarantine_type VARCHAR(8) NOT NULL DEFAULT 'auto'
                    CHECK (quarantine_type IN ('auto', 'manual')),
    recoverable     BOOLEAN NOT NULL DEFAULT TRUE,

    -- Recovery backoff state
    recovery_attempts       INTEGER NOT NULL DEFAULT 0,
    max_recovery_attempts   INTEGER NOT NULL DEFAULT 5,
    next_recovery_at        TIMESTAMPTZ,
    base_delay_ms           INTEGER NOT NULL DEFAULT 1000,
    max_delay_ms            INTEGER NOT NULL DEFAULT 60000,
    multiplier              NUMERIC(4,2) NOT NULL DEFAULT 2.0,

    quarantined_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    released_at     TIMESTAMPTZ             -- NULL = still quarantined
);

-- Fast lookup: "is this resource quarantined right now?"
CREATE UNIQUE INDEX idx_quarantine_active
    ON resource_quarantine(resource_id)
    WHERE released_at IS NULL;

CREATE INDEX idx_quarantine_resource   ON resource_quarantine(resource_id);
CREATE INDEX idx_quarantine_recovery   ON resource_quarantine(next_recovery_at)
    WHERE released_at IS NULL;

-- ============================================================
-- RESOURCE EVENTS
-- Used for: Event Stream sidebar, Events page + filters
-- High-volume table — retain 7 days, partition by day if needed.
-- ============================================================

CREATE TABLE resource_events (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,

    event_type      resource_event_type NOT NULL,

    -- Execution context (shown as "exec-7f3a · wait 0.2ms" in the UI)
    execution_id    UUID,
    workflow_id     UUID,

    -- Type-specific payload fields
    -- acquired:         wait_duration_ms
    -- released:         usage_duration_ms
    -- cleaned_up:       cleanup_reason
    -- health_changed:   health_state_from, health_state_to
    -- error:            error_message
    -- pool_exhausted:   wait_duration_ms (time waited before giving up)
    usage_duration_ms   INTEGER,
    wait_duration_ms    INTEGER,
    cleanup_reason      resource_cleanup_reason,
    health_state_from   resource_status,
    health_state_to     resource_status,
    error_message       TEXT,

    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_resource_events_resource ON resource_events(resource_id, occurred_at DESC);
CREATE INDEX idx_resource_events_tenant   ON resource_events(tenant_id,   occurred_at DESC);
CREATE INDEX idx_resource_events_type     ON resource_events(tenant_id, event_type, occurred_at DESC);
CREATE INDEX idx_resource_events_exec     ON resource_events(execution_id)
    WHERE execution_id IS NOT NULL;

-- ============================================================
-- RESOURCE HOOKS
-- Used for: Hooks tab (list of hooks, priority, events, filter)
-- resource_id NULL = tenant-wide hook applied to all resources.
-- ============================================================

CREATE TABLE resource_hooks (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    resource_id     UUID REFERENCES resources(id) ON DELETE CASCADE,  -- NULL = all resources

    hook_name       VARCHAR(128) NOT NULL,      -- 'AuditHook', 'SlowAcquireHook'
    priority        INTEGER NOT NULL DEFAULT 50 CHECK (priority >= 0),
    events          TEXT[]  NOT NULL,           -- {'acquire','release','cleaned_up','error'}
    filter_type     resource_hook_filter NOT NULL DEFAULT 'all',
    filter_value    VARCHAR(255),               -- NULL for 'all'; id or prefix string otherwise

    -- Hook-specific settings, e.g. {"threshold_ms": 2000} for SlowAcquireHook
    config          JSONB NOT NULL DEFAULT '{}',

    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- filter_value required when filter_type is not 'all'
    CHECK (filter_type = 'all' OR filter_value IS NOT NULL)
);

CREATE INDEX idx_resource_hooks_tenant   ON resource_hooks(tenant_id);
CREATE INDEX idx_resource_hooks_resource ON resource_hooks(resource_id);
CREATE INDEX idx_resource_hooks_active   ON resource_hooks(tenant_id, is_active) WHERE is_active = TRUE;

-- ============================================================
-- RESOURCE POOL METRICS (time-series samples)
-- Used for: Pool → Active Connections Trend, Pool → Wait Time sparklines
-- Sampled every N seconds by worker. Retain 24 hours.
-- ============================================================

CREATE TABLE resource_pool_metrics (
    id              BIGSERIAL PRIMARY KEY,
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    sampled_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    active_count    INTEGER NOT NULL DEFAULT 0,
    idle_count      INTEGER NOT NULL DEFAULT 0,
    waiting_count   INTEGER NOT NULL DEFAULT 0,     -- connections waiting for a permit
    avg_wait_ms     NUMERIC(10,2),
    avg_usage_ms    NUMERIC(10,2)
);

CREATE INDEX idx_pool_metrics_resource ON resource_pool_metrics(resource_id, sampled_at DESC);

-- ============================================================
-- MAINTENANCE FUNCTIONS
-- ============================================================

-- Purge health check history older than 7 days
CREATE OR REPLACE FUNCTION cleanup_old_health_checks() RETURNS void AS $$
BEGIN
    DELETE FROM resource_health_checks
    WHERE checked_at < NOW() - INTERVAL '7 days';
END;
$$ LANGUAGE plpgsql;

-- Purge event log older than 7 days
CREATE OR REPLACE FUNCTION cleanup_old_resource_events() RETURNS void AS $$
BEGIN
    DELETE FROM resource_events
    WHERE occurred_at < NOW() - INTERVAL '7 days';
END;
$$ LANGUAGE plpgsql;

-- Purge pool metric samples older than 24 hours
CREATE OR REPLACE FUNCTION cleanup_old_pool_metrics() RETURNS void AS $$
BEGIN
    DELETE FROM resource_pool_metrics
    WHERE sampled_at < NOW() - INTERVAL '24 hours';
END;
$$ LANGUAGE plpgsql;

-- ============================================================
-- UPDATED_AT TRIGGER
-- ============================================================

CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_resources_updated_at
    BEFORE UPDATE ON resources
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER trg_resource_hooks_updated_at
    BEFORE UPDATE ON resource_hooks
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
