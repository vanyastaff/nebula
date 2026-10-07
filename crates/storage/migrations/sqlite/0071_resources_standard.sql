-- Resources: the resource definitions a workspace stores, the runtime status
-- workers publish for them, and the shared-resource runtime that accepts one
-- resource's events, fans each out to the resource's subscriptions and hands
-- every delivered event to the execution it starts.
--
-- Transition migration of the database standard (docs/database-standard.md):
-- replaces port_resources, port_resource_status (now resource_status_snapshots),
-- port_worker_heartbeats (now resource_status_heartbeats), port_shared_resources,
-- port_resource_subscriptions, port_resource_source_leases, port_resource_events,
-- port_resource_deliveries and port_resource_execution_handoffs. Rate limits
-- are PostgreSQL-only and move with runtime control.
-- Instants are INTEGER microseconds since the Unix epoch.
--
-- A stored resource and a shared resource belong to their workspace and are
-- purged with it; everything beneath either cascades from it.

DROP TABLE IF EXISTS port_resource_execution_handoffs;
DROP TABLE IF EXISTS port_resource_deliveries;
DROP TABLE IF EXISTS port_resource_events;
DROP TABLE IF EXISTS port_resource_source_leases;
DROP TABLE IF EXISTS port_resource_subscriptions;
DROP TABLE IF EXISTS port_shared_resources;
DROP TABLE IF EXISTS port_resource_status;
DROP TABLE IF EXISTS port_worker_heartbeats;
DROP TABLE IF EXISTS port_resources;

-- ── Stored resources ─────────────────────────────────────────────────────
--
-- A resource definition belongs to its workspace. `config` is the kind's own
-- document; `credential_bindings` maps the kind's credential slots to
-- credential selectors, kept apart from `config` so credential authority is
-- never smuggled through kind-specific values; `topology` and
-- `resilience_override` are operator runtime settings (NULL: the kind's
-- defaults).
CREATE TABLE resources (
    org_id              TEXT    NOT NULL,
    workspace_id        TEXT    NOT NULL,
    id                  TEXT    NOT NULL,
    slug                TEXT    NOT NULL,
    display_name        TEXT    NOT NULL,
    kind                TEXT    NOT NULL,
    config              TEXT    NOT NULL,
    credential_bindings TEXT    NOT NULL,
    topology            TEXT,
    resilience_override TEXT,
    created_at          INTEGER NOT NULL,
    created_by          TEXT    NOT NULL,
    version             INTEGER NOT NULL DEFAULT 0,
    deleted_at          INTEGER,
    CONSTRAINT pk_resources PRIMARY KEY (org_id, workspace_id, id),
    CONSTRAINT fk_resources__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resources__config_json CHECK (json_valid(config)),
    CONSTRAINT ck_resources__credential_bindings_json CHECK (json_valid(credential_bindings)),
    CONSTRAINT ck_resources__credential_bindings_object
        CHECK (json_type(credential_bindings) = 'object'),
    CONSTRAINT ck_resources__topology_json CHECK (json_valid(topology)),
    CONSTRAINT ck_resources__resilience_override_json CHECK (json_valid(resilience_override)),
    CONSTRAINT ck_resources__version CHECK (version >= 0)
);
CREATE UNIQUE INDEX uq_resources__org_id_workspace_id_slug__live
    ON resources (org_id, workspace_id, slug) WHERE deleted_at IS NULL;

-- ── Runtime status ───────────────────────────────────────────────────────
--
-- Workers renew a liveness heartbeat and publish one lifecycle snapshot per
-- stored resource they run; readers trust a snapshot only while its worker's
-- heartbeat is unexpired, so a crashed worker's status disappears without
-- anyone deleting it. Snapshots carry lifecycle state only, never config or
-- credential material.

-- A worker's liveness lease. A long-expired heartbeat is pruned together with
-- the worker's snapshots.
CREATE TABLE resource_status_heartbeats (
    worker_id  TEXT    NOT NULL,
    expires_at INTEGER NOT NULL,
    CONSTRAINT pk_resource_status_heartbeats PRIMARY KEY (worker_id),
    CONSTRAINT ck_resource_status_heartbeats__worker_id_length
        CHECK (length(CAST(worker_id AS BLOB)) BETWEEN 1 AND 128)
);

-- One worker's view of one stored resource, purged with the resource.
-- `worker_id` names no heartbeat row: a snapshot may be published before
-- its worker's first heartbeat and becomes visible with it.
CREATE TABLE resource_status_snapshots (
    org_id       TEXT    NOT NULL,
    workspace_id TEXT    NOT NULL,
    resource_id  TEXT    NOT NULL,
    worker_id    TEXT    NOT NULL,
    phase        TEXT    NOT NULL,
    healthy      INTEGER NOT NULL,
    accepting    INTEGER NOT NULL,
    row_version  INTEGER NOT NULL,
    CONSTRAINT pk_resource_status_snapshots
        PRIMARY KEY (org_id, workspace_id, resource_id, worker_id),
    CONSTRAINT fk_resource_status_snapshots__resources
        FOREIGN KEY (org_id, workspace_id, resource_id)
        REFERENCES resources (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resource_status_snapshots__worker_id_length
        CHECK (length(CAST(worker_id AS BLOB)) BETWEEN 1 AND 128),
    CONSTRAINT ck_resource_status_snapshots__phase CHECK (phase IN ('initializing', 'ready',
        'reloading', 'draining', 'shutting_down', 'failed', 'unknown')),
    CONSTRAINT ck_resource_status_snapshots__healthy CHECK (healthy IN (0, 1)),
    CONSTRAINT ck_resource_status_snapshots__accepting CHECK (accepting IN (0, 1)),
    CONSTRAINT ck_resource_status_snapshots__row_version CHECK (row_version >= 0)
);
CREATE INDEX ix_resource_status_snapshots__worker_id
    ON resource_status_snapshots (worker_id);

-- ── Shared resources ─────────────────────────────────────────────────────
--
-- One physical resource of a workspace, identified exactly by its kind,
-- compatibility version, configuration identity and slot identity, purged
-- with its workspace. Ids are raw 16-byte UUIDs; `sequence` is the stable
-- reconciliation order. The identity columns exceed a PostgreSQL B-tree key
-- (64 KiB), so `identity_digest` (SHA-256 of the length-framed identity)
-- locates a candidate and the adapter compares the exact identity inside its
-- write transaction; the database does not enforce identity uniqueness.
CREATE TABLE shared_resources (
    org_id                 TEXT    NOT NULL,
    workspace_id           TEXT    NOT NULL,
    id                     BLOB    NOT NULL,
    sequence               INTEGER NOT NULL
        CONSTRAINT pk_shared_resources PRIMARY KEY AUTOINCREMENT,
    kind                   TEXT    NOT NULL,
    compatibility_version  INTEGER NOT NULL,
    configuration_identity BLOB    NOT NULL,
    slot_identity          BLOB    NOT NULL,
    identity_digest        BLOB    NOT NULL,
    CONSTRAINT uq_shared_resources__org_id_workspace_id_id UNIQUE (org_id, workspace_id, id),
    CONSTRAINT fk_shared_resources__workspaces
        FOREIGN KEY (org_id, workspace_id) REFERENCES workspaces (org_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_shared_resources__id_length CHECK (typeof(id) = 'blob' AND length(id) = 16),
    CONSTRAINT ck_shared_resources__kind_length
        CHECK (length(CAST(kind AS BLOB)) BETWEEN 1 AND 128),
    CONSTRAINT ck_shared_resources__compatibility_version
        CHECK (compatibility_version BETWEEN 0 AND 4294967295),
    CONSTRAINT ck_shared_resources__configuration_identity_length CHECK (
        typeof(configuration_identity) = 'blob'
        AND length(configuration_identity) BETWEEN 1 AND 65536
    ),
    CONSTRAINT ck_shared_resources__slot_identity_length
        CHECK (typeof(slot_identity) = 'blob' AND length(slot_identity) BETWEEN 0 AND 65536),
    CONSTRAINT ck_shared_resources__identity_digest_length
        CHECK (typeof(identity_digest) = 'blob' AND length(identity_digest) = 32)
);
CREATE INDEX ix_shared_resources__org_id_workspace_id_identity_digest
    ON shared_resources (org_id, workspace_id, identity_digest);
CREATE INDEX ix_shared_resources__org_id_workspace_id_sequence
    ON shared_resources (org_id, workspace_id, sequence);

-- A consumer's subscription to a shared resource's events, purged with the
-- resource. A subscription is never deleted: `tombstoned` is its terminal
-- state. `version` is the CAS counter of its state transitions.
CREATE TABLE resource_subscriptions (
    org_id            TEXT    NOT NULL,
    workspace_id      TEXT    NOT NULL,
    resource_id       BLOB    NOT NULL,
    id                BLOB    NOT NULL,
    sequence          INTEGER NOT NULL
        CONSTRAINT pk_resource_subscriptions PRIMARY KEY AUTOINCREMENT,
    consumer_kind     TEXT    NOT NULL,
    consumer_identity BLOB    NOT NULL,
    state             TEXT    NOT NULL,
    version           INTEGER NOT NULL,
    CONSTRAINT uq_resource_subscriptions__org_id_workspace_id_id
        UNIQUE (org_id, workspace_id, id),
    CONSTRAINT uq_resource_subscriptions__org_id_workspace_id_resource_id_id
        UNIQUE (org_id, workspace_id, resource_id, id),
    CONSTRAINT uq_resource_subscriptions__resource
        UNIQUE (org_id, workspace_id, resource_id, consumer_kind, consumer_identity),
    CONSTRAINT fk_resource_subscriptions__shared_resources
        FOREIGN KEY (org_id, workspace_id, resource_id)
        REFERENCES shared_resources (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resource_subscriptions__resource_id_length
        CHECK (typeof(resource_id) = 'blob' AND length(resource_id) = 16),
    CONSTRAINT ck_resource_subscriptions__id_length CHECK (typeof(id) = 'blob' AND length(id) = 16),
    CONSTRAINT ck_resource_subscriptions__consumer_kind_length
        CHECK (length(CAST(consumer_kind AS BLOB)) BETWEEN 1 AND 64),
    CONSTRAINT ck_resource_subscriptions__consumer_identity_length CHECK (
        typeof(consumer_identity) = 'blob' AND length(consumer_identity) BETWEEN 1 AND 512
    ),
    CONSTRAINT ck_resource_subscriptions__state
        CHECK (state IN ('active', 'disabled', 'tombstoned')),
    CONSTRAINT ck_resource_subscriptions__version CHECK (version >= 0)
);
CREATE INDEX ix_resource_subscriptions__resource_id_state_sequence
    ON resource_subscriptions (org_id, workspace_id, resource_id, state, sequence);
CREATE INDEX ix_resource_subscriptions__org_id_workspace_id_sequence
    ON resource_subscriptions (org_id, workspace_id, sequence);

-- The lease of the one source that may accept a shared resource's events,
-- purged with the resource. `generation` fences every accepted event (never
-- decremented); a released lease keeps its generation and expires at once.
CREATE TABLE resource_source_leases (
    org_id       TEXT    NOT NULL,
    workspace_id TEXT    NOT NULL,
    resource_id  BLOB    NOT NULL,
    holder       TEXT    NOT NULL,
    claim_id     BLOB    NOT NULL,
    generation   INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    CONSTRAINT pk_resource_source_leases PRIMARY KEY (org_id, workspace_id, resource_id),
    CONSTRAINT fk_resource_source_leases__shared_resources
        FOREIGN KEY (org_id, workspace_id, resource_id)
        REFERENCES shared_resources (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resource_source_leases__resource_id_length
        CHECK (typeof(resource_id) = 'blob' AND length(resource_id) = 16),
    CONSTRAINT ck_resource_source_leases__holder_length
        CHECK (length(CAST(holder AS BLOB)) BETWEEN 1 AND 256),
    CONSTRAINT ck_resource_source_leases__claim_id_length
        CHECK (typeof(claim_id) = 'blob' AND length(claim_id) = 16),
    CONSTRAINT ck_resource_source_leases__generation CHECK (generation >= 0)
);

-- ── Event fan-out ────────────────────────────────────────────────────────
--
-- An event a shared resource's source accepted, purged with the resource.
-- Its occurrence identity (namespace, key) is unique per resource, so a
-- redelivered occurrence replays or conflicts instead of fanning out twice.
-- `accepted_at` and `source_generation` record the source lease that
-- accepted it; `complete` once no delivery is pending.
CREATE TABLE resource_events (
    org_id               TEXT    NOT NULL,
    workspace_id         TEXT    NOT NULL,
    resource_id          BLOB    NOT NULL,
    id                   BLOB    NOT NULL,
    occurrence_namespace TEXT    NOT NULL,
    occurrence_key       BLOB    NOT NULL,
    schema_version       INTEGER NOT NULL,
    canonical_payload    BLOB    NOT NULL,
    envelope_digest      BLOB    NOT NULL,
    accepted_at          INTEGER NOT NULL,
    source_generation    INTEGER NOT NULL,
    state                TEXT    NOT NULL,
    CONSTRAINT pk_resource_events PRIMARY KEY (org_id, workspace_id, id),
    CONSTRAINT uq_resource_events__org_id_workspace_id_resource_id_id
        UNIQUE (org_id, workspace_id, resource_id, id),
    CONSTRAINT uq_resource_events__resource
        UNIQUE (org_id, workspace_id, resource_id, occurrence_namespace, occurrence_key),
    CONSTRAINT fk_resource_events__shared_resources
        FOREIGN KEY (org_id, workspace_id, resource_id)
        REFERENCES shared_resources (org_id, workspace_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resource_events__resource_id_length
        CHECK (typeof(resource_id) = 'blob' AND length(resource_id) = 16),
    CONSTRAINT ck_resource_events__id_length CHECK (typeof(id) = 'blob' AND length(id) = 16),
    CONSTRAINT ck_resource_events__occurrence_namespace_length
        CHECK (length(CAST(occurrence_namespace AS BLOB)) BETWEEN 1 AND 128),
    CONSTRAINT ck_resource_events__occurrence_key_length
        CHECK (typeof(occurrence_key) = 'blob' AND length(occurrence_key) BETWEEN 1 AND 1024),
    CONSTRAINT ck_resource_events__schema_version CHECK (schema_version BETWEEN 0 AND 4294967295),
    CONSTRAINT ck_resource_events__canonical_payload_length CHECK (
        typeof(canonical_payload) = 'blob' AND length(canonical_payload) BETWEEN 1 AND 1048576
    ),
    CONSTRAINT ck_resource_events__envelope_digest_length
        CHECK (typeof(envelope_digest) = 'blob' AND length(envelope_digest) = 32),
    CONSTRAINT ck_resource_events__source_generation CHECK (source_generation >= 0),
    CONSTRAINT ck_resource_events__state CHECK (state IN ('pending', 'complete'))
);

-- One event's delivery to one subscription of the same resource, purged with
-- either. A claim mints `claim_generation`, which fences every
-- acknowledgement (never decremented); a terminal delivery keeps the claim
-- that completed it and the completion it requested, so a replayed
-- completion is recognised.
CREATE TABLE resource_deliveries (
    org_id                    TEXT    NOT NULL,
    workspace_id              TEXT    NOT NULL,
    resource_id               BLOB    NOT NULL,
    id                        BLOB    NOT NULL,
    sequence                  INTEGER NOT NULL
        CONSTRAINT pk_resource_deliveries PRIMARY KEY AUTOINCREMENT,
    event_id                  BLOB    NOT NULL,
    subscription_id           BLOB    NOT NULL,
    status                    TEXT    NOT NULL,
    terminal_reason           TEXT,
    requested_status          TEXT,
    requested_terminal_reason TEXT,
    claim_holder              TEXT,
    claim_id                  BLOB,
    claim_generation          INTEGER NOT NULL,
    claim_expires_at          INTEGER,
    terminal_claim_id         BLOB,
    terminal_claim_generation INTEGER,
    CONSTRAINT uq_resource_deliveries__org_id_workspace_id_id UNIQUE (org_id, workspace_id, id),
    CONSTRAINT uq_resource_deliveries__org_id_workspace_id_resource_id_id
        UNIQUE (org_id, workspace_id, resource_id, id),
    CONSTRAINT uq_resource_deliveries__event_id_subscription_id
        UNIQUE (org_id, workspace_id, event_id, subscription_id),
    CONSTRAINT fk_resource_deliveries__resource_events
        FOREIGN KEY (org_id, workspace_id, resource_id, event_id)
        REFERENCES resource_events (org_id, workspace_id, resource_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_resource_deliveries__resource_subscriptions
        FOREIGN KEY (org_id, workspace_id, resource_id, subscription_id)
        REFERENCES resource_subscriptions (org_id, workspace_id, resource_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resource_deliveries__resource_id_length
        CHECK (typeof(resource_id) = 'blob' AND length(resource_id) = 16),
    CONSTRAINT ck_resource_deliveries__id_length CHECK (typeof(id) = 'blob' AND length(id) = 16),
    CONSTRAINT ck_resource_deliveries__event_id_length
        CHECK (typeof(event_id) = 'blob' AND length(event_id) = 16),
    CONSTRAINT ck_resource_deliveries__subscription_id_length
        CHECK (typeof(subscription_id) = 'blob' AND length(subscription_id) = 16),
    CONSTRAINT ck_resource_deliveries__status
        CHECK (status IN ('pending', 'delivered', 'ineligible')),
    CONSTRAINT ck_resource_deliveries__terminal_reason CHECK (terminal_reason IN (
        'subscription_disabled', 'subscription_tombstoned', 'consumer_unavailable',
        'unsupported_envelope_schema')),
    CONSTRAINT ck_resource_deliveries__requested_status
        CHECK (requested_status IN ('delivered', 'ineligible')),
    CONSTRAINT ck_resource_deliveries__requested_terminal_reason
        CHECK (requested_terminal_reason IN ('subscription_disabled', 'subscription_tombstoned',
            'consumer_unavailable', 'unsupported_envelope_schema')),
    CONSTRAINT ck_resource_deliveries__claim_holder_length
        CHECK (length(CAST(claim_holder AS BLOB)) BETWEEN 1 AND 256),
    CONSTRAINT ck_resource_deliveries__claim_id_length
        CHECK (typeof(claim_id) IN ('null', 'blob') AND length(claim_id) = 16),
    CONSTRAINT ck_resource_deliveries__claim_generation CHECK (claim_generation >= 0),
    CONSTRAINT ck_resource_deliveries__terminal_claim_id_length CHECK (
        typeof(terminal_claim_id) IN ('null', 'blob') AND length(terminal_claim_id) = 16
    ),
    CONSTRAINT ck_resource_deliveries__terminal_claim_generation
        CHECK (terminal_claim_generation >= 0),
    -- A live claim has a holder, an id and an expiry, or none of them.
    CONSTRAINT ck_resource_deliveries__claim CHECK (
        (claim_holder IS NULL) = (claim_id IS NULL)
        AND (claim_id IS NULL) = (claim_expires_at IS NULL)
    ),
    -- Pending until completed; a terminal delivery holds no live claim, keeps
    -- the claim that completed it, and only an ineligible one has a reason.
    CONSTRAINT ck_resource_deliveries__terminal CHECK (
        (status = 'ineligible') = (terminal_reason IS NOT NULL)
        AND (terminal_claim_id IS NULL) = (terminal_claim_generation IS NULL)
        AND (status = 'pending') = (terminal_claim_id IS NULL)
        AND (status = 'pending' OR claim_id IS NULL)
    ),
    -- A terminal delivery records the completion its claimant requested.
    CONSTRAINT ck_resource_deliveries__requested_completion CHECK (
        (status = 'pending' AND requested_status IS NULL AND requested_terminal_reason IS NULL)
        OR (status <> 'pending' AND requested_status IS NOT NULL AND (
            (requested_status = 'delivered' AND requested_terminal_reason IS NULL)
            OR (requested_status = 'ineligible' AND requested_terminal_reason IS NOT NULL)))
    )
);
CREATE INDEX ix_resource_deliveries__sequence_claim_expires_at__pending
    ON resource_deliveries (org_id, workspace_id, sequence, claim_expires_at)
    WHERE status = 'pending';
CREATE INDEX ix_resource_deliveries__org_id_workspace_id_event_id__pending
    ON resource_deliveries (org_id, workspace_id, event_id)
    WHERE status = 'pending';
CREATE INDEX ix_resource_deliveries__org_id_workspace_id_subscription_id
    ON resource_deliveries (org_id, workspace_id, subscription_id);

-- The recoverable hand-off of one delivered event to the execution it starts,
-- keyed by its delivery and purged with it. A claim fences the start exactly
-- as a delivery claim does; `acknowledged` once the start is recorded.
CREATE TABLE resource_execution_handoffs (
    org_id                    TEXT    NOT NULL,
    workspace_id              TEXT    NOT NULL,
    resource_id               BLOB    NOT NULL,
    delivery_id               BLOB    NOT NULL,
    sequence                  INTEGER NOT NULL
        CONSTRAINT pk_resource_execution_handoffs PRIMARY KEY AUTOINCREMENT,
    event_id                  BLOB    NOT NULL,
    subscription_id           BLOB    NOT NULL,
    status                    TEXT    NOT NULL,
    claim_holder              TEXT,
    claim_id                  BLOB,
    claim_generation          INTEGER NOT NULL,
    claim_expires_at          INTEGER,
    terminal_claim_id         BLOB,
    terminal_claim_generation INTEGER,
    CONSTRAINT uq_resource_execution_handoffs__org_id_workspace_id_delivery_id
        UNIQUE (org_id, workspace_id, delivery_id),
    CONSTRAINT fk_resource_execution_handoffs__resource_deliveries
        FOREIGN KEY (org_id, workspace_id, resource_id, delivery_id)
        REFERENCES resource_deliveries (org_id, workspace_id, resource_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_resource_execution_handoffs__resource_events
        FOREIGN KEY (org_id, workspace_id, resource_id, event_id)
        REFERENCES resource_events (org_id, workspace_id, resource_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_resource_execution_handoffs__resource_subscriptions
        FOREIGN KEY (org_id, workspace_id, resource_id, subscription_id)
        REFERENCES resource_subscriptions (org_id, workspace_id, resource_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_resource_execution_handoffs__resource_id_length
        CHECK (typeof(resource_id) = 'blob' AND length(resource_id) = 16),
    CONSTRAINT ck_resource_execution_handoffs__delivery_id_length
        CHECK (typeof(delivery_id) = 'blob' AND length(delivery_id) = 16),
    CONSTRAINT ck_resource_execution_handoffs__event_id_length
        CHECK (typeof(event_id) = 'blob' AND length(event_id) = 16),
    CONSTRAINT ck_resource_execution_handoffs__subscription_id_length
        CHECK (typeof(subscription_id) = 'blob' AND length(subscription_id) = 16),
    CONSTRAINT ck_resource_execution_handoffs__status CHECK (status IN ('pending', 'acknowledged')),
    CONSTRAINT ck_resource_execution_handoffs__claim_holder_length
        CHECK (length(CAST(claim_holder AS BLOB)) BETWEEN 1 AND 256),
    CONSTRAINT ck_resource_execution_handoffs__claim_id_length
        CHECK (typeof(claim_id) IN ('null', 'blob') AND length(claim_id) = 16),
    CONSTRAINT ck_resource_execution_handoffs__claim_generation CHECK (claim_generation >= 0),
    CONSTRAINT ck_resource_execution_handoffs__terminal_claim_id_length CHECK (
        typeof(terminal_claim_id) IN ('null', 'blob') AND length(terminal_claim_id) = 16
    ),
    CONSTRAINT ck_resource_execution_handoffs__terminal_claim_generation
        CHECK (terminal_claim_generation >= 0),
    CONSTRAINT ck_resource_execution_handoffs__claim CHECK (
        (claim_holder IS NULL) = (claim_id IS NULL)
        AND (claim_id IS NULL) = (claim_expires_at IS NULL)
    ),
    CONSTRAINT ck_resource_execution_handoffs__terminal CHECK (
        (terminal_claim_id IS NULL) = (terminal_claim_generation IS NULL)
        AND (status = 'pending') = (terminal_claim_id IS NULL)
        AND (status = 'pending' OR claim_id IS NULL)
    )
);
CREATE INDEX ix_resource_execution_handoffs__sequence__pending
    ON resource_execution_handoffs (org_id, workspace_id, sequence, claim_expires_at)
    WHERE status = 'pending';
CREATE INDEX ix_resource_execution_handoffs__org_id_workspace_id_event_id
    ON resource_execution_handoffs (org_id, workspace_id, event_id);
CREATE INDEX ix_resource_execution_handoffs__subscription_id
    ON resource_execution_handoffs (org_id, workspace_id, subscription_id);
