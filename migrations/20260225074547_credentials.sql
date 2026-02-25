-- ============================================================
-- 007: Credentials
-- ============================================================

-- ============================================================
-- CREDENTIAL TYPES (built-in + custom schemas)
-- ============================================================

CREATE TABLE credential_types (
    id              VARCHAR(128) PRIMARY KEY,              -- 'http_basic_auth', 'aws_iam', 'oauth2'
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    icon_url        TEXT,
    schema          JSONB NOT NULL DEFAULT '{}',           -- JSON Schema for the credential fields
    is_builtin      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO credential_types (id, name, description, is_builtin, schema) VALUES
    ('http_basic_auth', 'HTTP Basic Auth', 'Username and password', TRUE,
     '{"properties": {"username": {"type": "string"}, "password": {"type": "string", "secret": true}}}'),
    ('bearer_token', 'Bearer Token', 'API token / Bearer authorization', TRUE,
     '{"properties": {"token": {"type": "string", "secret": true}}}'),
    ('oauth2', 'OAuth 2.0', 'OAuth2 client credentials flow', TRUE,
     '{"properties": {"client_id": {"type": "string"}, "client_secret": {"type": "string", "secret": true}, "token_url": {"type": "string"}}}'),
    ('aws_iam', 'AWS IAM', 'AWS access key credentials', TRUE,
     '{"properties": {"access_key_id": {"type": "string"}, "secret_access_key": {"type": "string", "secret": true}, "region": {"type": "string"}}}'),
    ('ssh_key', 'SSH Private Key', 'SSH key pair for remote access', TRUE,
     '{"properties": {"private_key": {"type": "string", "secret": true}, "passphrase": {"type": "string", "secret": true}}}'),
    ('postgres', 'PostgreSQL', 'PostgreSQL database connection', TRUE,
     '{"properties": {"host": {"type": "string"}, "port": {"type": "integer"}, "database": {"type": "string"}, "username": {"type": "string"}, "password": {"type": "string", "secret": true}, "ssl": {"type": "boolean"}}}'),
    ('smtp', 'SMTP Email', 'SMTP server for sending emails', TRUE,
     '{"properties": {"host": {"type": "string"}, "port": {"type": "integer"}, "username": {"type": "string"}, "password": {"type": "string", "secret": true}, "use_tls": {"type": "boolean"}}}');

-- ============================================================
-- CREDENTIALS (encrypted storage)
-- ============================================================

CREATE TABLE credentials (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    credential_type VARCHAR(128) NOT NULL REFERENCES credential_types(id),
    name            VARCHAR(255) NOT NULL,
    description     TEXT,

    -- Encrypted credential data (AES-256-GCM via nebula-credential / ring)
    data_encrypted  BYTEA NOT NULL,
    data_iv         BYTEA NOT NULL,                        -- Initialization vector
    key_id          VARCHAR(128),                          -- KMS key reference if using external KMS

    -- Metadata (non-sensitive, stored plaintext for search/display)
    metadata        JSONB NOT NULL DEFAULT '{}',           -- e.g. {"username": "john", "host": "db.example.com"}

    is_shared       BOOLEAN NOT NULL DEFAULT FALSE,        -- shared across tenant or private
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_credentials_tenant ON credentials(tenant_id);
CREATE INDEX idx_credentials_type ON credentials(credential_type);
CREATE UNIQUE INDEX idx_credentials_tenant_name ON credentials(tenant_id, name);
