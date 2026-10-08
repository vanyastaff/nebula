-- Historical acceptance belongs to provisioning, not to the mutable tenant.
-- No parent FK: purge must not reopen a completed creation command.
CREATE TABLE tenant_provisioning_receipts (
    org_id               TEXT    NOT NULL,
    initial_workspace_id TEXT,
    request_version      INTEGER,
    request_digest       BLOB,
    recorded_at          INTEGER NOT NULL,
    CONSTRAINT pk_tenant_provisioning_receipts PRIMARY KEY (org_id),
    CONSTRAINT uq_tenant_provisioning_receipts__initial_workspace_id
        UNIQUE (initial_workspace_id),
    CONSTRAINT ck_tenant_provisioning_receipts__request CHECK (
        (request_version IS NULL AND request_digest IS NULL AND initial_workspace_id IS NULL)
        OR (request_version IS NOT NULL AND request_version = 1
            AND request_digest IS NOT NULL AND typeof(request_digest) = 'blob'
            AND length(request_digest) = 32 AND initial_workspace_id IS NOT NULL)
    )
);

-- Existing rows prove identity occupation, not the original creation command.
-- Do not infer a digest or an initial workspace from today's mutable records.
INSERT INTO tenant_provisioning_receipts (org_id, recorded_at)
SELECT id, CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER) FROM orgs;
