-- ============================================================
-- 016: Cluster (Nodes, Workers, Locks, Queue)
-- ============================================================

CREATE TYPE node_role AS ENUM ('leader', 'follower', 'candidate');
CREATE TYPE worker_status AS ENUM ('active', 'draining', 'offline', 'error');

-- ============================================================
-- CLUSTER NODES
-- ============================================================

CREATE TABLE cluster_nodes (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    hostname        VARCHAR(255) NOT NULL,
    ip_address      INET NOT NULL,
    port            INTEGER NOT NULL DEFAULT 7700,
    role            node_role NOT NULL DEFAULT 'follower',
    status          worker_status NOT NULL DEFAULT 'active',
    version         VARCHAR(32),                           -- nebula-engine version

    -- Capabilities
    tags            TEXT[] NOT NULL DEFAULT '{}',          -- for targeted execution routing
    max_workers     INTEGER NOT NULL DEFAULT 4,
    current_load    FLOAT NOT NULL DEFAULT 0.0,            -- 0.0 to 1.0

    -- Raft state
    raft_term       BIGINT NOT NULL DEFAULT 0,
    last_log_index  BIGINT NOT NULL DEFAULT 0,

    -- Health
    last_heartbeat  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (hostname, port)
);

CREATE INDEX idx_cluster_nodes_status ON cluster_nodes(status);
CREATE INDEX idx_cluster_nodes_role ON cluster_nodes(role);
CREATE INDEX idx_cluster_nodes_heartbeat ON cluster_nodes(last_heartbeat DESC);

-- ============================================================
-- WORKER INSTANCES (ephemeral, registered at startup)
-- ============================================================

CREATE TABLE workers (
    id              UUID PRIMARY KEY,                      -- worker's own UUID, set at boot
    cluster_node_id UUID NOT NULL REFERENCES cluster_nodes(id) ON DELETE CASCADE,
    status          worker_status NOT NULL DEFAULT 'active',

    -- Concurrency
    max_concurrent  INTEGER NOT NULL DEFAULT 4,
    active_count    INTEGER NOT NULL DEFAULT 0,

    -- Routing
    tenant_affinity UUID[] DEFAULT '{}',                   -- preferred tenants (empty = any)
    tags            TEXT[] NOT NULL DEFAULT '{}',

    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_workers_node ON workers(cluster_node_id);
CREATE INDEX idx_workers_status ON workers(status);
CREATE INDEX idx_workers_heartbeat ON workers(last_heartbeat DESC) WHERE status = 'active';

-- ============================================================
-- DISTRIBUTED LOCKS (leader election, single-execution guarantees)
-- ============================================================

CREATE TABLE distributed_locks (
    resource        VARCHAR(512) PRIMARY KEY,
    holder_id       UUID NOT NULL,                         -- worker or cluster node id
    acquired_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at      TIMESTAMPTZ NOT NULL,
    metadata        JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_locks_expires ON distributed_locks(expires_at);

-- ============================================================
-- WORK QUEUE (persisted queue for reliable delivery)
-- ============================================================

CREATE TYPE queue_item_status AS ENUM ('pending', 'claimed', 'done', 'failed');

CREATE TABLE work_queue (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    execution_id    UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    priority        SMALLINT NOT NULL DEFAULT 5,           -- 1 (highest) to 10
    status          queue_item_status NOT NULL DEFAULT 'pending',
    worker_id       UUID REFERENCES workers(id) ON DELETE SET NULL,
    claimed_at      TIMESTAMPTZ,
    done_at         TIMESTAMPTZ,
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    scheduled_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),    -- for delayed execution
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_work_queue_pending
    ON work_queue(priority ASC, scheduled_at ASC)
    WHERE status = 'pending';
CREATE INDEX idx_work_queue_worker ON work_queue(worker_id) WHERE status = 'claimed';
CREATE INDEX idx_work_queue_execution ON work_queue(execution_id);

-- Fast lookup for heartbeat-based stale detection
CREATE INDEX idx_cluster_nodes_stale ON cluster_nodes(last_heartbeat)
    WHERE status = 'active';

CREATE INDEX idx_workers_stale ON workers(last_heartbeat)
    WHERE status = 'active';
