-- Credentials: the encrypted credential records a workspace owns, the claims
-- that fence a provider operation (refresh, revoke) on one credential across
-- replicas, the incidents an operation that crashed past the provider
-- boundary leaves for reconciliation, and the encrypted state of interactive
-- credential flows that have not produced a credential yet.
--
-- Transition migration of the database standard (docs/database-standard.md):
-- replaces credentials, credential_refresh_claims, credential_sentinel_events
-- (now credential_refresh_incidents) and credential_pending_states. The
-- opaque owner partition becomes the workspace's (org_id, workspace_id).
-- Rate limits are PostgreSQL-only and move with runtime control.
-- Instants are INTEGER microseconds since the Unix epoch.
--
-- Credentials live in the deployment database beside tenancy: a credential and
-- a pending flow belong to their workspace and are purged with it, and
-- everything beneath a credential cascades from it.

DROP TABLE IF EXISTS credential_pending_states;
DROP TABLE IF EXISTS credential_sentinel_events;
DROP TABLE IF EXISTS credential_refresh_claims;
DROP TABLE IF EXISTS credentials;

-- ── Credentials ──────────────────────────────────────────────────────────
--
-- A credential belongs to its workspace and is purged with it. Ids are
-- server-generated and unique across tenants, so the key is the id and
-- (org_id, workspace_id, id) is the target children reference.
--
-- `data` is the ciphertext the encryption layer above the adapter seals
-- (key id and AAD binding live inside it); storage never inspects it.
-- `version` is the CAS counter; its last value is reserved for the
-- tombstone. `material_epoch` advances with every material-authority
-- change and `admission_epoch` with every write that closes credential use.
-- A revocation tombstone (`record_state = 'tombstoned'`) is terminal and
-- keeps only the identity physical binding reads need; the archive is
-- `deleted_at`, which hides the row from every read and write.
-- `refresh_retry_*` is the structural refresh-retry gate, separate from
-- metadata and from claim TTL.
CREATE TABLE credentials (
    org_id                        TEXT    NOT NULL,
    workspace_id                  TEXT    NOT NULL,
    id                            TEXT    NOT NULL,
    name                          TEXT,
    credential_key                TEXT    NOT NULL,
    state_kind                    TEXT    NOT NULL,
    state_version                 INTEGER NOT NULL,
    data                          BLOB    NOT NULL,
    version                       INTEGER NOT NULL,
    material_epoch                INTEGER NOT NULL,
    admission_epoch               INTEGER NOT NULL,
    created_at                    INTEGER NOT NULL,
    updated_at                    INTEGER NOT NULL,
    expires_at                    INTEGER,
    reauth_required               INTEGER NOT NULL,
    metadata                      TEXT    NOT NULL,
    record_state                  TEXT    NOT NULL,
    tombstoned_at                 INTEGER,
    refresh_retry_mode            TEXT,
    refresh_retry_not_before      INTEGER,
    refresh_retry_phase           TEXT,
    refresh_retry_kind            TEXT,
    refresh_retry_diagnostic_code TEXT,
    deleted_at                    INTEGER,
    CONSTRAINT pk_credentials PRIMARY KEY (id),
    CONSTRAINT uq_credentials__org_id_workspace_id_id UNIQUE (org_id, workspace_id, id),
    CONSTRAINT fk_credentials__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_credentials__state_version
        CHECK (typeof(state_version) = 'integer' AND state_version BETWEEN 0 AND 4294967295),
    CONSTRAINT ck_credentials__data_blob CHECK (typeof(data) = 'blob'),
    CONSTRAINT ck_credentials__version CHECK (typeof(version) = 'integer' AND version >= 1),
    CONSTRAINT ck_credentials__material_epoch
        CHECK (typeof(material_epoch) = 'integer' AND material_epoch >= 1),
    CONSTRAINT ck_credentials__admission_epoch
        CHECK (typeof(admission_epoch) = 'integer' AND admission_epoch >= 1),
    CONSTRAINT ck_credentials__reauth_required
        CHECK (typeof(reauth_required) = 'integer' AND reauth_required IN (0, 1)),
    CONSTRAINT ck_credentials__metadata_json CHECK (json_valid(metadata)),
    CONSTRAINT ck_credentials__metadata_object CHECK (json_type(metadata) = 'object'),
    CONSTRAINT ck_credentials__record_state CHECK (record_state IN ('live', 'tombstoned')),
    -- A live credential's `name` is the projection of its display name.
    CONSTRAINT ck_credentials__name_projection CHECK (
        record_state = 'tombstoned'
        OR (name IS NULL
            AND coalesce(json_type(metadata, '$.display.display_name'), 'null') = 'null')
        OR (name IS NOT NULL
            AND json_type(metadata, '$.display.display_name') = 'text'
            AND name = json_extract(metadata, '$.display.display_name'))
    ),
    CONSTRAINT ck_credentials__refresh_retry_gate CHECK (
        (refresh_retry_mode IS NULL
            AND refresh_retry_not_before IS NULL
            AND refresh_retry_phase IS NULL
            AND refresh_retry_kind IS NULL
            AND refresh_retry_diagnostic_code IS NULL)
        OR (record_state = 'live'
            AND refresh_retry_phase IN ('before_dispatch', 'provider_confirmed_not_applied')
            AND refresh_retry_kind IN ('transient_network', 'provider_unavailable', 'protocol_error')
            AND (refresh_retry_diagnostic_code IS NULL
                OR (typeof(refresh_retry_diagnostic_code) = 'text'
                    AND length(refresh_retry_diagnostic_code) BETWEEN 1 AND 64
                    AND refresh_retry_diagnostic_code NOT GLOB '*[^A-Za-z0-9_.:-]*'))
            AND ((refresh_retry_mode = 'never' AND refresh_retry_not_before IS NULL)
                OR (refresh_retry_mode = 'not_before'
                    AND typeof(refresh_retry_not_before) = 'integer')))
    ),
    -- A live row keeps one version for its tombstone; a tombstone holds no
    -- material, name, metadata, expiry, reauthentication or retry gate.
    CONSTRAINT ck_credentials__record_shape CHECK (
        (record_state = 'live'
            AND tombstoned_at IS NULL
            AND version <= 9223372036854775806)
        OR (record_state = 'tombstoned'
            AND tombstoned_at IS NOT NULL
            AND length(data) = 0
            AND name IS NULL
            AND expires_at IS NULL
            AND reauth_required = 0
            AND metadata = '{}')
    )
);
CREATE UNIQUE INDEX uq_credentials__org_id_workspace_id_name__live
    ON credentials (org_id, workspace_id, name) WHERE deleted_at IS NULL;
CREATE INDEX ix_credentials__org_id_workspace_id_state_kind
    ON credentials (org_id, workspace_id, state_kind);
-- The due-refresh scan walks live, expiring credentials in expiry order.
CREATE INDEX ix_credentials__expires_at__expiring
    ON credentials (expires_at) WHERE expires_at IS NOT NULL AND deleted_at IS NULL;

-- ── Provider-operation claims ────────────────────────────────────────────
--
-- At most one claim per credential, purged with it. A claim is won by
-- inserting it or by replacing an expired claim that never crossed the
-- provider boundary; `sentinel` marks a claim whose holder crossed it, and
-- such a claim, once expired, is durable poison until reconciled.
-- `generation` fences every acknowledgement (never decremented). A revoke
-- claim records the material epoch it observed.
CREATE TABLE credential_refresh_claims (
    org_id                  TEXT    NOT NULL,
    workspace_id            TEXT    NOT NULL,
    credential_id           TEXT    NOT NULL,
    claim_id                TEXT    NOT NULL,
    generation              INTEGER NOT NULL,
    holder_replica_id       TEXT    NOT NULL,
    acquired_at             INTEGER NOT NULL,
    expires_at              INTEGER NOT NULL,
    sentinel                INTEGER NOT NULL,
    operation_kind          TEXT    NOT NULL,
    observed_material_epoch INTEGER,
    CONSTRAINT pk_credential_refresh_claims PRIMARY KEY (org_id, workspace_id, credential_id),
    CONSTRAINT fk_credential_refresh_claims__credentials
        FOREIGN KEY (org_id, workspace_id, credential_id)
        REFERENCES credentials (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_credential_refresh_claims__generation CHECK (generation >= 0),
    CONSTRAINT ck_credential_refresh_claims__sentinel CHECK (sentinel IN (0, 1)),
    CONSTRAINT ck_credential_refresh_claims__operation_kind
        CHECK (operation_kind IN ('refresh', 'revoke', 'legacy_unclassified')),
    CONSTRAINT ck_credential_refresh_claims__observed_material_epoch CHECK (
        (operation_kind = 'revoke' AND observed_material_epoch >= 1)
        OR (operation_kind <> 'revoke' AND observed_material_epoch IS NULL)
    )
);
CREATE INDEX ix_credential_refresh_claims__expires_at
    ON credential_refresh_claims (expires_at);

-- ── Operation incidents ──────────────────────────────────────────────────
--
-- One row per claim that expired after crossing the provider boundary,
-- keyed by that claim's globally unique id and purged with its credential.
-- `detected_at` places it in the escalation window; the adjudication columns
-- are the operator's recorded provider outcome, all present or all absent.
CREATE TABLE credential_refresh_incidents (
    org_id                       TEXT    NOT NULL,
    workspace_id                 TEXT    NOT NULL,
    credential_id                TEXT    NOT NULL,
    claim_id                     TEXT    NOT NULL,
    detected_at                  INTEGER NOT NULL,
    crashed_holder               TEXT    NOT NULL,
    generation                   INTEGER NOT NULL,
    operation_kind               TEXT    NOT NULL,
    observed_material_epoch      INTEGER,
    adjudicated_at               INTEGER,
    adjudication_decision        TEXT,
    adjudication_evidence        TEXT,
    adjudication_evidence_digest BLOB,
    CONSTRAINT pk_credential_refresh_incidents PRIMARY KEY (claim_id),
    CONSTRAINT fk_credential_refresh_incidents__credentials
        FOREIGN KEY (org_id, workspace_id, credential_id)
        REFERENCES credentials (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_credential_refresh_incidents__generation CHECK (generation >= 0),
    CONSTRAINT ck_credential_refresh_incidents__operation_kind
        CHECK (operation_kind IN ('refresh', 'revoke', 'legacy_unclassified')),
    CONSTRAINT ck_credential_refresh_incidents__observed_material_epoch CHECK (
        (operation_kind = 'revoke' AND observed_material_epoch >= 1)
        OR (operation_kind <> 'revoke' AND observed_material_epoch IS NULL)
    ),
    CONSTRAINT ck_credential_refresh_incidents__adjudication CHECK (
        (adjudicated_at IS NULL
            AND adjudication_decision IS NULL
            AND adjudication_evidence IS NULL
            AND adjudication_evidence_digest IS NULL)
        OR (adjudicated_at IS NOT NULL
            AND adjudication_decision IN ('provider_applied', 'provider_not_applied',
                'provider_revoked', 'provider_not_revoked')
            AND adjudication_evidence IS NOT NULL
            AND typeof(adjudication_evidence_digest) = 'blob'
            AND length(adjudication_evidence_digest) = 32)
    )
);
CREATE INDEX ix_credential_refresh_incidents__credential_id_detected_at
    ON credential_refresh_incidents (org_id, workspace_id, credential_id, detected_at);

-- ── Pending interactive state ────────────────────────────────────────────
--
-- Encrypted state of an interactive credential flow (an OAuth authorization
-- in progress), owned by the workspace the credential will belong to and
-- purged with it. Only the SHA-256 digest of the bearer token is stored; the
-- state is sealed with AAD binding the token digest, kind, owner and
-- session, so a row moved to another binding does not decrypt.
CREATE TABLE credential_pending_states (
    org_id          TEXT    NOT NULL,
    workspace_id    TEXT    NOT NULL,
    token_digest    BLOB    NOT NULL,
    credential_kind TEXT    NOT NULL,
    session_id      TEXT    NOT NULL,
    state_encrypted BLOB    NOT NULL,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    CONSTRAINT pk_credential_pending_states PRIMARY KEY (token_digest),
    CONSTRAINT fk_credential_pending_states__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_credential_pending_states__token_digest_length
        CHECK (typeof(token_digest) = 'blob' AND length(token_digest) = 32),
    CONSTRAINT ck_credential_pending_states__expires_at CHECK (expires_at >= created_at)
);
CREATE INDEX ix_credential_pending_states__expires_at
    ON credential_pending_states (expires_at);
CREATE INDEX ix_credential_pending_states__org_id_workspace_id
    ON credential_pending_states (org_id, workspace_id);
