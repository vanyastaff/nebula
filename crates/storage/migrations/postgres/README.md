# PostgreSQL Migrations

Spec-16 compliant schema for Nebula's PostgreSQL backend.

## Dialect notes

- IDs: `BYTEA` (16-byte ULID, prefixed on wire)
- JSON: `JSONB`
- Timestamps: `TIMESTAMPTZ`
- IP addresses: `INET`
- Arrays: native `BYTEA[]`
- Booleans: `BOOLEAN`
- CAS: `BIGINT` version column on all mutable entities

## Migration order

| # | File | Layer | Tables |
|---|------|-------|--------|
| 0001 | `users` | Identity | `users` |
| 0002 | `user_auth` | Identity | `oauth_links`, `sessions`, `personal_access_tokens`, `verification_tokens` |
| 0003 | `orgs` | Tenancy | `orgs` |
| 0004 | `workspaces` | Tenancy | `workspaces` |
| 0005 | `memberships` | Tenancy | `org_members`, `workspace_members`, `service_accounts` |
| 0006 | `workflows` | Workflow | `workflows` |
| 0007 | `workflow_versions` | Workflow | `workflow_versions` + FK on `workflows` |
| 0008 | `credentials` | Credentials | `credentials` |
| 0009 | `resources` | Resources | `resources` |
| 0010 | `triggers` | Triggers | `triggers`, `trigger_events`, `cron_fire_slots` |
| 0011 | `executions` | Execution | `executions` |
| 0012 | `execution_nodes` | Execution | `execution_nodes`, `pending_signals` |
| 0013 | `execution_lifecycle` | Execution | `execution_journal`, `execution_control_queue` |
| 0014 | `quotas` | Quotas | `org_quotas`, `org_quota_usage`, `workspace_quota_usage`, `workspace_dispatch_state` |
| 0015 | `audit` | Audit | `slug_history`, `audit_log` |
| 0016 | `executions_observability` | Execution | `executions` — adds trace correlation and takeover count |
| 0017 | `credentials_v3` | Credentials | legacy credential envelope/lifecycle, pending-flow, and audit extensions |
| 0018 | `webhook_paths` | Triggers | `triggers` — adds indexed active webhook path |
| 0019 | `blobs` | Storage | `blobs` — out-of-row binary payloads and retention indexes |
| 0020 | `add_resume_result_persistence` | Execution | `execution_nodes` — adds `result_schema_version`, `result_kind`, `result` (ADR-0009) |
| 0021 | `add_control_queue_reclaim_count` | Execution | `execution_control_queue` — adds `reclaim_count` (ADR-0017 / ADR-0008 B1) |
| 0022 | `credential_refresh_claims` | Credentials | `credential_refresh_claims` — cross-replica refresh ownership |
| 0023 | `credential_sentinel_events` | Credentials | `credential_sentinel_events` — durable refresh-crash evidence |
| 0024 | `add_idempotency_dedup` | API | `api_idempotency_dedup` — bounded response-replay records |
| 0025 | `triggers_config_namespace_comment` | Triggers | documents the kind-namespaced `triggers.config` contract |
| 0026 | `execution_control_queue_w3c_trace_context` | Execution | `execution_control_queue` — nullable `w3c_trace_context` (M3.5 W3C carrier) |
| 0027 | `port_adapter_schema` | Storage port | the `port_*` tables — execution / journal / control-queue / idempotency / webhook-activation / workflow / workflow-version, and the identity zoo (`port_users` … `port_blobs`) |
| 0028 | `plane_a_oauth_state` | Identity | `plane_a_oauth_states` — single-use PKCE callback authority |
| 0029 | `external_identities` | Identity | `external_identities` — authoritative provider-subject links |
| 0030 | `credentials_store` | Credentials | replaces the unused legacy model with the durable credential store |
| 0031 | `job_dispatch_and_trigger_dedup` | Execution | `port_job_dispatch_queue`, `port_trigger_dedup_inbox` |
| 0032 | `webhook_activation_fields` | Storage port | activation workflow, mode, and token-digest fields |
| 0033 | `webhook_activation_spec_link` | Storage port | activation-to-trigger-spec link |
| 0034 | `port_control_queue_resume_target` | Execution | durable resume target on `port_control_queue` |
| 0035 | `port_resume_tokens` | Execution | one-way-digested, mint-on-park resume authorities |
| 0036 | `plane_a_oauth_state_cleanup_index` | Identity | complete expiry-cleanup index for consumed and live state |
| 0037 | `mfa_enrollment_candidates` | Identity | expiring, single-use MFA replacement candidates, separate from the active user factor |
| 0038 | `identity_secret_authority` | Identity | digest-only sessions and authenticated-encryption envelopes for active/pending TOTP |
| 0039 | `credentials_owner_and_record_state` | Credentials | owner-bound structural live/tombstone lifecycle |
| 0040 | `credential_refresh_retry_gate` | Credentials | durable structural refresh-retry admission gate |
| 0041 | `port_plan_flavor_revision_catalog` | Runtime control | dormant immutable plan/flavor records and exact execution revision references |

## Storage-port adapter schema (0027)

`0027_port_adapter_schema.sql` is the historical migration that introduced
the spec-16 port tables. `nebula_storage::postgres::init_schema`, credential
readiness, clean database setup, and upgrades all run this exact ordered
migration tree; no separate embedded schema snapshot exists. General
`init_schema` validates catalog facts only; credential readiness adds
credential relation/row admission under the same advisory lock and session.
The spec-16 port
(`PgExecutionStore` + the atomic
`TransitionBatch`, the durable control-queue outbox, idempotency,
webhook activations, workflows/versions, and the identity stores)
persists through these `port_*` tables. Applied migration files are immutable;
schema changes always append a new numbered migration.

Migration `0041` is deliberately dormant DDL. It enforces byte lengths,
closed lifecycle/reference sums, restrictive foreign keys, and blocker
indexes, but provides no SQL adapter, activation path, payload-hash proof, or
exactly-once guarantee.

## Rebuilding the local dev database

`task db:reset` **drops and recreates the database** (then uses raw SQLx to run
every migration in this directory, `0027` included). It destroys all local dev
data. `task db:migrate` is the non-destructive path and uses the server-owned
admitted operator: catalog-only setup first, credential-owner deep admission
only after a typed configuration rejection. Numbered history is immutable and
forward-only; no revert task is supported.

## Schema parity

This directory and `../sqlite/` must define logically identical tables.
Types differ by dialect; table/column names and constraints must match.

See the maintainers' private design vault for the authoritative spec.
