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

## Schema parity

This directory and `../sqlite/` must define logically identical tables.
Types differ by dialect; table/column names and constraints must match.

See `docs/plans/2026-04-15-arch-specs/16-storage-schema.md` for the authoritative spec.
