-- ============================================================
-- 015: Registry (Actions, Nodes, Packages)
-- ============================================================

-- ============================================================
-- ACTION DEFINITIONS (atomic operations)
-- ============================================================

CREATE TABLE action_definitions (
    id              VARCHAR(255) PRIMARY KEY,              -- 'http.request', 'postgres.query'
    node_id         VARCHAR(255),                          -- grouping node: 'http', 'postgres'
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    category        VARCHAR(128) NOT NULL,                 -- 'network', 'database', 'transform'
    icon_url        TEXT,
    version         VARCHAR(32) NOT NULL DEFAULT '1.0.0',  -- semver
    min_engine_version VARCHAR(32),                        -- minimum nebula-engine version

    -- Schemas (JSON Schema for UI form generation)
    input_schema    JSONB NOT NULL DEFAULT '{}',
    output_schema   JSONB NOT NULL DEFAULT '{}',
    credential_types TEXT[],                               -- required credential types

    -- Properties
    is_builtin      BOOLEAN NOT NULL DEFAULT FALSE,
    is_deprecated   BOOLEAN NOT NULL DEFAULT FALSE,
    deprecated_message TEXT,
    is_async        BOOLEAN NOT NULL DEFAULT TRUE,
    is_retriable    BOOLEAN NOT NULL DEFAULT TRUE,
    default_timeout_ms INTEGER NOT NULL DEFAULT 30000,

    -- Full Rust metadata (serialized ActionMetadata)
    metadata        JSONB NOT NULL DEFAULT '{}',

    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_action_defs_node ON action_definitions(node_id);
CREATE INDEX idx_action_defs_category ON action_definitions(category);

-- ============================================================
-- NODE DEFINITIONS (grouped actions, like n8n nodes)
-- ============================================================

CREATE TABLE node_definitions (
    id              VARCHAR(255) PRIMARY KEY,              -- 'http', 'postgres', 'slack'
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    category        VARCHAR(128) NOT NULL,
    subcategory     VARCHAR(128),
    icon_url        TEXT,
    version         VARCHAR(32) NOT NULL DEFAULT '1.0.0',
    credential_types TEXT[],                               -- can be overridden per action
    is_builtin      BOOLEAN NOT NULL DEFAULT FALSE,
    is_deprecated   BOOLEAN NOT NULL DEFAULT FALSE,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_node_defs_category ON node_definitions(category);

-- Backfill FK
ALTER TABLE action_definitions
    ADD CONSTRAINT fk_action_node
    FOREIGN KEY (node_id)
    REFERENCES node_definitions(id)
    ON DELETE SET NULL;

-- ============================================================
-- PACKAGES (community/enterprise installable packs)
-- ============================================================

CREATE TYPE package_status AS ENUM ('available', 'installed', 'disabled', 'error');

CREATE TABLE packages (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    package_name    VARCHAR(255) NOT NULL UNIQUE,          -- 'nebula-node-stripe'
    display_name    VARCHAR(255) NOT NULL,
    description     TEXT,
    author          VARCHAR(255),
    repository_url  TEXT,
    icon_url        TEXT,
    version         VARCHAR(32) NOT NULL,
    min_engine_version VARCHAR(32),

    -- Checksums for integrity
    checksum_sha256 TEXT NOT NULL,
    download_url    TEXT,

    status          package_status NOT NULL DEFAULT 'available',
    installed_at    TIMESTAMPTZ,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================
-- TENANT INSTALLED PACKAGES
-- ============================================================

CREATE TABLE tenant_packages (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    package_id      UUID NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    installed_version VARCHAR(32) NOT NULL,
    installed_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    installed_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    UNIQUE (tenant_id, package_id)
);

CREATE INDEX idx_tenant_packages_tenant ON tenant_packages(tenant_id);
