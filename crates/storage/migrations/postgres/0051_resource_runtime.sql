CREATE TABLE port_shared_resources (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id BYTEA NOT NULL CHECK (octet_length(id) = 16),
    workspace_id TEXT NOT NULL,
    org_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (octet_length(kind) BETWEEN 1 AND 128),
    compatibility_version BIGINT NOT NULL CHECK (compatibility_version BETWEEN 0 AND 4294967295),
    configuration_identity BYTEA NOT NULL CHECK (octet_length(configuration_identity) BETWEEN 1 AND 65536),
    slot_identity BYTEA NOT NULL CHECK (octet_length(slot_identity) BETWEEN 0 AND 65536),
    identity_digest BYTEA NOT NULL CHECK (octet_length(identity_digest) = 32),
    UNIQUE (workspace_id, org_id, id)
);
CREATE INDEX port_shared_resources_identity_digest ON port_shared_resources (workspace_id, org_id, identity_digest);
CREATE INDEX port_shared_resources_reconciliation ON port_shared_resources (workspace_id, org_id, sequence);

CREATE TABLE port_resource_subscriptions (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id BYTEA NOT NULL CHECK (octet_length(id) = 16), workspace_id TEXT NOT NULL, org_id TEXT NOT NULL,
    resource_id BYTEA NOT NULL CHECK (octet_length(resource_id) = 16),
    consumer_kind TEXT NOT NULL CHECK (octet_length(consumer_kind) BETWEEN 1 AND 64),
    consumer_identity BYTEA NOT NULL CHECK (octet_length(consumer_identity) BETWEEN 1 AND 512),
    state TEXT NOT NULL CHECK (state IN ('active', 'disabled', 'tombstoned')),
    version BYTEA NOT NULL CHECK (octet_length(version) = 8),
    UNIQUE (workspace_id, org_id, id),
    UNIQUE (workspace_id, org_id, resource_id, id),
    UNIQUE (workspace_id, org_id, resource_id, consumer_kind, consumer_identity),
    FOREIGN KEY (workspace_id, org_id, resource_id) REFERENCES port_shared_resources (workspace_id, org_id, id)
);
CREATE INDEX port_resource_subscriptions_active ON port_resource_subscriptions (workspace_id, org_id, resource_id, state, sequence);
CREATE INDEX port_resource_subscriptions_reconciliation ON port_resource_subscriptions (workspace_id, org_id, sequence);

CREATE TABLE port_resource_source_leases (
    workspace_id TEXT NOT NULL, org_id TEXT NOT NULL,
    resource_id BYTEA NOT NULL CHECK (octet_length(resource_id) = 16),
    holder TEXT NOT NULL CHECK (octet_length(holder) BETWEEN 1 AND 256),
    claim_id BYTEA NOT NULL CHECK (octet_length(claim_id) = 16),
    generation BYTEA NOT NULL CHECK (octet_length(generation) = 8), expires_at_ms BIGINT NOT NULL,
    PRIMARY KEY (workspace_id, org_id, resource_id),
    FOREIGN KEY (workspace_id, org_id, resource_id) REFERENCES port_shared_resources (workspace_id, org_id, id)
);

CREATE TABLE port_resource_events (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id BYTEA NOT NULL CHECK (octet_length(id) = 16), workspace_id TEXT NOT NULL, org_id TEXT NOT NULL,
    resource_id BYTEA NOT NULL CHECK (octet_length(resource_id) = 16),
    occurrence_namespace TEXT NOT NULL CHECK (octet_length(occurrence_namespace) BETWEEN 1 AND 128),
    occurrence_key BYTEA NOT NULL CHECK (octet_length(occurrence_key) BETWEEN 1 AND 1024),
    occurrence_digest BYTEA NOT NULL CHECK (octet_length(occurrence_digest) = 32),
    schema_version BIGINT NOT NULL CHECK (schema_version BETWEEN 0 AND 4294967295),
    canonical_payload BYTEA NOT NULL CHECK (octet_length(canonical_payload) BETWEEN 1 AND 1048576),
    envelope_digest BYTEA NOT NULL CHECK (octet_length(envelope_digest) = 32),
    accepted_at_ms BIGINT NOT NULL,
    source_generation BYTEA NOT NULL CHECK (octet_length(source_generation) = 8),
    state TEXT NOT NULL CHECK (state IN ('pending', 'complete')),
    UNIQUE (workspace_id, org_id, id),
    UNIQUE (workspace_id, org_id, resource_id, id),
    FOREIGN KEY (workspace_id, org_id, resource_id) REFERENCES port_shared_resources (workspace_id, org_id, id)
);
CREATE INDEX port_resource_events_occurrence ON port_resource_events (workspace_id, org_id, resource_id, occurrence_digest);

CREATE TABLE port_resource_deliveries (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id BYTEA NOT NULL CHECK (octet_length(id) = 16), workspace_id TEXT NOT NULL, org_id TEXT NOT NULL,
    resource_id BYTEA NOT NULL CHECK (octet_length(resource_id) = 16),
    event_id BYTEA NOT NULL CHECK (octet_length(event_id) = 16),
    subscription_id BYTEA NOT NULL CHECK (octet_length(subscription_id) = 16),
    status TEXT NOT NULL CHECK (status IN ('pending', 'delivered', 'ineligible')),
    terminal_reason TEXT CHECK (terminal_reason IS NULL OR terminal_reason IN ('subscription_disabled', 'subscription_tombstoned', 'consumer_unavailable', 'unsupported_envelope_schema')),
    requested_status TEXT CHECK (requested_status IS NULL OR requested_status IN ('delivered', 'ineligible')),
    requested_terminal_reason TEXT CHECK (requested_terminal_reason IS NULL OR requested_terminal_reason IN ('subscription_disabled', 'subscription_tombstoned', 'consumer_unavailable', 'unsupported_envelope_schema')),
    claim_holder TEXT CHECK (claim_holder IS NULL OR octet_length(claim_holder) BETWEEN 1 AND 256),
    claim_id BYTEA CHECK (claim_id IS NULL OR octet_length(claim_id) = 16),
    claim_generation BYTEA NOT NULL CHECK (octet_length(claim_generation) = 8), claim_expires_at_ms BIGINT,
    terminal_claim_id BYTEA CHECK (terminal_claim_id IS NULL OR octet_length(terminal_claim_id) = 16),
    terminal_claim_generation BYTEA CHECK (terminal_claim_generation IS NULL OR octet_length(terminal_claim_generation) = 8),
    UNIQUE (workspace_id, org_id, id), UNIQUE (workspace_id, org_id, resource_id, id),
    UNIQUE (workspace_id, org_id, event_id, subscription_id),
    CHECK ((claim_holder IS NULL) = (claim_id IS NULL) AND (claim_id IS NULL) = (claim_expires_at_ms IS NULL)),
    CHECK ((status = 'ineligible') = (terminal_reason IS NOT NULL)),
    CHECK ((terminal_claim_id IS NULL) = (terminal_claim_generation IS NULL)),
    CHECK ((status = 'pending') = (terminal_claim_id IS NULL)),
    CHECK (status = 'pending' OR claim_id IS NULL),
    CHECK (
        (status = 'pending' AND requested_status IS NULL AND requested_terminal_reason IS NULL)
        OR (status <> 'pending' AND requested_status IS NOT NULL AND (
            (requested_status = 'delivered' AND requested_terminal_reason IS NULL)
            OR (requested_status = 'ineligible' AND requested_terminal_reason IS NOT NULL)
        ))
    ),
    FOREIGN KEY (workspace_id, org_id, resource_id, event_id) REFERENCES port_resource_events (workspace_id, org_id, resource_id, id),
    FOREIGN KEY (workspace_id, org_id, resource_id, subscription_id) REFERENCES port_resource_subscriptions (workspace_id, org_id, resource_id, id)
);
CREATE INDEX port_resource_deliveries_claimable ON port_resource_deliveries (workspace_id, org_id, sequence, claim_expires_at_ms) WHERE status = 'pending';
CREATE INDEX port_resource_deliveries_event_pending ON port_resource_deliveries (workspace_id, org_id, event_id) WHERE status = 'pending';

CREATE TABLE port_resource_execution_handoffs (
    sequence BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workspace_id TEXT NOT NULL, org_id TEXT NOT NULL,
    resource_id BYTEA NOT NULL CHECK (octet_length(resource_id) = 16),
    delivery_id BYTEA NOT NULL CHECK (octet_length(delivery_id) = 16),
    event_id BYTEA NOT NULL CHECK (octet_length(event_id) = 16),
    subscription_id BYTEA NOT NULL CHECK (octet_length(subscription_id) = 16),
    status TEXT NOT NULL CHECK (status IN ('pending', 'acknowledged')),
    claim_holder TEXT CHECK (claim_holder IS NULL OR octet_length(claim_holder) BETWEEN 1 AND 256),
    claim_id BYTEA CHECK (claim_id IS NULL OR octet_length(claim_id) = 16),
    claim_generation BYTEA NOT NULL CHECK (octet_length(claim_generation) = 8),
    claim_expires_at_ms BIGINT,
    terminal_claim_id BYTEA CHECK (terminal_claim_id IS NULL OR octet_length(terminal_claim_id) = 16),
    terminal_claim_generation BYTEA CHECK (terminal_claim_generation IS NULL OR octet_length(terminal_claim_generation) = 8),
    UNIQUE (workspace_id, org_id, delivery_id),
    CHECK ((claim_holder IS NULL) = (claim_id IS NULL) AND (claim_id IS NULL) = (claim_expires_at_ms IS NULL)),
    CHECK ((terminal_claim_id IS NULL) = (terminal_claim_generation IS NULL)),
    CHECK ((status = 'pending') = (terminal_claim_id IS NULL)),
    CHECK (status = 'pending' OR claim_id IS NULL),
    FOREIGN KEY (workspace_id, org_id, resource_id, delivery_id) REFERENCES port_resource_deliveries (workspace_id, org_id, resource_id, id),
    FOREIGN KEY (workspace_id, org_id, resource_id, event_id) REFERENCES port_resource_events (workspace_id, org_id, resource_id, id),
    FOREIGN KEY (workspace_id, org_id, resource_id, subscription_id) REFERENCES port_resource_subscriptions (workspace_id, org_id, resource_id, id)
);
CREATE INDEX port_resource_execution_handoffs_claimable ON port_resource_execution_handoffs (workspace_id, org_id, sequence, claim_expires_at_ms) WHERE status = 'pending';
