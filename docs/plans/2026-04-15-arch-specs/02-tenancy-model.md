# Spec 02 — Tenancy model

> **Status:** draft
> **Canon target:** §5 (scope table), §11.8 (new)
> **Depends on:** spec 01 (positioning determines tenant requirements)
> **Depended on by:** 03 (auth), 04 (RBAC), 05 (routing), 16 (storage schema)

## Problem

Nebula serves three distinct realities from the same codebase:

1. Solo developer on a VPS (one user, one «tenant»)
2. Cloud customers sharing infrastructure (many tenants, strong isolation)
3. Enterprise self-host with teams inside one installation (middle case)

Without tenant isolation built into the data model, cross-tenant data leaks are a typed impossibility only if enforced from day zero. Adding tenant awareness after the first customer is a multi-month migration that competitors have publicly suffered through (Temporal storage layer rewrites, n8n Cloud instance-per-tenant scaling pain).

## Decision

**Two-level hierarchy: Organization → Workspace**, threaded through every storage query, API route, event, metric label, and credential access. Self-host gets an implicit `default` org with `default` workspace; users never see the hierarchy until they invite a collaborator or create a second workspace.

## Data model

### Organization

```rust
// nebula-core (new types)
pub struct OrgId(Ulid);  // prefix "org_"

pub struct Organization {
    pub id: OrgId,
    pub slug: String,               // globally unique (§07)
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub created_by: UserId,         // owner
    pub plan: PlanTier,             // Free / Team / Business / Enterprise
    pub billing_email: Option<String>,
    pub settings: OrgSettings,
    pub version: u64,               // CAS
}

pub enum PlanTier {
    SelfHost,   // no quotas enforced
    Free,       // cloud free tier
    Team,
    Business,
    Enterprise,
}

pub struct OrgSettings {
    pub allow_cross_workspace_resource_sharing: bool,  // default true
    pub require_mfa: bool,                              // enforced on all members
    pub session_max_lifetime: Duration,                 // default 7 days
    pub credential_rotation_reminder_days: Option<u32>, // optional
}
```

### Workspace

```rust
pub struct WorkspaceId(Ulid);  // prefix "ws_"

pub struct Workspace {
    pub id: WorkspaceId,
    pub org_id: OrgId,
    pub slug: String,               // unique per org (§07)
    pub display_name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: UserId,
    pub is_default: bool,           // first workspace in org, cannot be deleted
    pub settings: WorkspaceSettings,
    pub version: u64,
}

pub struct WorkspaceSettings {
    pub default_workflow_timeout: Duration,
    pub default_action_timeout: Duration,
    pub allowed_trigger_types: Vec<TriggerType>,   // restrict which triggers can be used
    pub data_retention_days: Option<u32>,          // override org default
}
```

### SQL schema

```sql
CREATE TABLE orgs (
    id              BYTEA PRIMARY KEY,
    slug            TEXT NOT NULL UNIQUE,
    display_name    TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    created_by      BYTEA NOT NULL,           -- user_id
    plan            TEXT NOT NULL,            -- PlanTier as string
    billing_email   TEXT,
    settings        JSONB NOT NULL,
    version         BIGINT NOT NULL DEFAULT 0,
    deleted_at      TIMESTAMPTZ               -- soft delete
);

CREATE UNIQUE INDEX idx_orgs_slug_active
    ON orgs (slug)
    WHERE deleted_at IS NULL;

CREATE TABLE workspaces (
    id              BYTEA PRIMARY KEY,
    org_id          BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    slug            TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    description     TEXT,
    created_at      TIMESTAMPTZ NOT NULL,
    created_by      BYTEA NOT NULL,
    is_default      BOOLEAN NOT NULL DEFAULT FALSE,
    settings        JSONB NOT NULL,
    version         BIGINT NOT NULL DEFAULT 0,
    deleted_at      TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_workspaces_org_slug
    ON workspaces (org_id, slug)
    WHERE deleted_at IS NULL;

-- Only one default workspace per org
CREATE UNIQUE INDEX idx_workspaces_org_default
    ON workspaces (org_id)
    WHERE is_default = TRUE AND deleted_at IS NULL;
```

## Isolation invariants

**Every query that touches user-owned data MUST include `workspace_id` or `org_id` in the WHERE clause.** This is enforced through:

**1. Type-level tenant context.**

```rust
// nebula-core
pub struct TenantContext {
    pub org_id: OrgId,
    pub workspace_id: WorkspaceId,
    pub principal: Principal,       // User, ServiceAccount, or System
}

// Every storage method takes &TenantContext
impl WorkflowRepo {
    pub async fn find_by_slug(
        &self,
        ctx: &TenantContext,
        slug: &str,
    ) -> Result<Option<Workflow>> { ... }
}
```

**2. SQL query templates include the scope.** No `SELECT * FROM workflows WHERE slug = ?` is ever written — always `WHERE workspace_id = ? AND slug = ?`.

**3. Middleware extracts tenant from path** (see spec 05) and constructs `TenantContext` before any handler runs. Handler signature is `fn(State, TenantContext, Path<...>) -> Result<Response>`.

**4. Metric labels include tenant dimensions.** Every metric emitted has `org_id` and `workspace_id` as labels (subject to label allowlist in `nebula-metrics` to prevent cardinality explosion — spec 15 of session 2025).

**5. Log / trace span attributes include tenant.** `tracing` span opened at middleware level, all downstream spans inherit `org_id`, `workspace_id`, `execution_id`.

**Forbidden:** any `TODO: add tenant_id later`. Tenant context is a prerequisite, not a follow-up.

## Organization lifecycle

### Creation

**Self-host:** `default` org created on first run (idempotent), owned by first user who creates account.

**Cloud:** user signs up → creates org (slug chosen by user, subject to availability) → becomes `OrgOwner` → default workspace `default` created automatically.

```rust
pub async fn create_org(
    db: &Storage,
    creator: UserId,
    slug: String,
    display_name: String,
    plan: PlanTier,
) -> Result<OrgId, OrgError> {
    // 1. Validate slug (spec 07)
    validate_org_slug(&slug)?;
    
    // 2. Check reserved words
    if is_reserved_org_slug(&slug) {
        return Err(OrgError::SlugReserved);
    }
    
    // 3. Atomic insert + first workspace + owner membership
    let mut tx = db.begin().await?;
    
    let org_id = OrgId::new();
    tx.insert_org(Organization { id: org_id, slug, display_name, plan, ... }).await?;
    
    let ws_id = WorkspaceId::new();
    tx.insert_workspace(Workspace {
        id: ws_id,
        org_id,
        slug: "default".into(),
        display_name: "Default Workspace".into(),
        is_default: true,
        ...
    }).await?;
    
    tx.insert_org_member(creator, org_id, OrgRole::OrgOwner).await?;
    tx.insert_workspace_member(creator, ws_id, WorkspaceRole::WorkspaceAdmin).await?;
    
    tx.commit().await?;
    Ok(org_id)
}
```

### Deletion

**Organizations are soft-deleted.** `deleted_at` timestamp set, hard cleanup after retention period (default 30 days) by background job. Cascade to workspaces, workflows, executions, credentials, etc. — all get `deleted_at` set in same transaction.

**Protection:** `OrgOwner` cannot delete org if they are the only owner — must transfer ownership first. Billing concerns (active subscription) block deletion until billing cancelled.

**Hard delete after retention:** background job `DELETE FROM orgs WHERE deleted_at < NOW() - INTERVAL '30 days'`. Cascades through FK ON DELETE CASCADE.

## Workspace lifecycle

### Creation

Any `OrgOwner` or `OrgAdmin` can create workspaces within their org (up to plan quota). The `default` workspace is created automatically with the org and cannot be deleted.

### Deletion

`default` workspace cannot be deleted. Non-default workspaces soft-deleted, same retention as org.

## Flows

### Self-host first run

```
1. User downloads nebula, runs `nebula serve`
2. Storage empty, no orgs exist
3. HTTP server starts, /health returns ok
4. User opens http://localhost:8080
5. Middleware detects no orgs → redirects to /setup
6. /setup form: "Create your admin account" (email + password)
7. On submit:
   - Transaction begins
   - Insert user (first user)
   - Insert org "default" with slug "default"
   - Insert workspace "default" in that org
   - Insert org_member(user, default_org, OrgOwner)
   - Insert workspace_member(user, default_ws, WorkspaceAdmin)
   - Commit
8. User logged in, redirected to /default/default
9. Empty workspace, ready to create first workflow
```

### Cloud signup

```
1. User visits nebula.io, clicks Sign Up
2. Enters email, password (or OAuth Google/GitHub)
3. Email verification required (unless OAuth)
4. After verification → "Create your organization" form
5. Slug + display name input
6. On submit:
   - Same transaction as self-host but without first-run bypass
   - Insert user, org, workspace, memberships
7. Redirect to /{org_slug}/default
```

### Inviting a collaborator

```
1. OrgAdmin opens /orgs/{org}/members → "Invite"
2. Enters email + selects role (OrgAdmin / OrgMember)
3. Optionally: pre-assigns workspace(s) with specific roles
4. Invite record created: {email, org_id, role, workspace_assignments, token, expires_at}
5. Email sent with invite link containing one-time token
6. Invitee clicks link → if account exists, prompted to accept; if not, account creation form
7. On accept, transaction:
   - Insert user (if new)
   - Insert org_member
   - Insert workspace_member(s) per assignments
   - Mark invite as accepted
8. Invitee lands in org dashboard
```

## Edge cases

**Orphaned workspace after org delete:** prevented by `ON DELETE CASCADE` on FK. All workspaces get `deleted_at` set in the same transaction as org.

**Cross-org credential reference:** forbidden by construction. Credentials live in workspace or org; cross-org sharing requires explicit copy, not reference.

**Workspace slug reused in different orgs:** allowed. `production` can exist in `acme` and `example` simultaneously — uniqueness constraint is `(org_id, slug)`, not global.

**Org slug reused after deletion:** after 30-day retention, hard delete happens. Slug becomes available. If some bad actor races this — document as known behavior, not a security issue (slug reuse does not transfer data).

**User in org that gets deleted:** their `org_member` row deleted via cascade. If user has no other orgs, their login returns them to an empty state with option to create new org or join existing.

**Default workspace cannot be deleted but can be emptied.** User deletes all workflows, credentials, resources — workspace becomes empty but persists.

**Transferring org ownership.** `OrgOwner` promotes another member to `OrgOwner`, then optionally demotes themselves. There must always be at least one `OrgOwner`.

## Configuration surface

```toml
[tenancy]
# Self-host: auto-create default org on first run
auto_default_org = true          # default for self-host, false for cloud

# Cloud: soft-delete retention period
org_retention_days = 30          # hard delete after soft delete
workspace_retention_days = 30

# Max workspaces per org (plan-specific override)
[tenancy.quotas.self_host]
max_workspaces_per_org = 1000
max_members_per_org = 1000

[tenancy.quotas.free]
max_workspaces_per_org = 5
max_members_per_org = 3

[tenancy.quotas.team]
max_workspaces_per_org = 50
max_members_per_org = 50

[tenancy.quotas.enterprise]
max_workspaces_per_org = 10000
max_members_per_org = 10000
```

## Testing criteria

**Unit tests:**
- `validate_org_slug` rejects all reserved words
- `create_org` is atomic (partial insert rolls back)
- `delete_org` cascades to workspaces, workflows, executions
- Workspace `is_default` flag cannot be moved after creation

**Integration tests:**
- Full self-host first-run flow (empty DB → owner created → default org/ws exist)
- Full cloud signup flow with email verification
- Invite flow end-to-end
- Cross-org isolation: user in org A cannot see data in org B (try every read endpoint)
- Workspace deletion in one org does not affect other orgs
- Slug uniqueness constraints enforced at DB level
- Soft delete + retention + hard delete job

**Property tests:**
- Any workspace row always has valid `org_id` (FK integrity)
- No query result contains rows with different `org_id` than requested
- Metric labels always include `org_id`, `workspace_id`

**Security tests:**
- Direct database manipulation test: insert workflow with wrong `workspace_id` → verify user cannot access it through any endpoint
- Fuzz test on path parameters: `/api/v1/orgs/{org}/workspaces/{ws}/...` with various injection attempts

## Performance targets

- Org lookup by slug: **< 1 ms p99** (unique index)
- Workspace list by org: **< 5 ms p99** for orgs with ≤ 100 workspaces
- Tenant context construction on every request: **< 500 µs** (single indexed query)
- Max workspaces per org: **10 000** without query degradation

## Module boundaries

| Type / function | Crate |
|---|---|
| `OrgId`, `WorkspaceId`, `OrgRole`, `WorkspaceRole` | `nebula-core` |
| `Organization`, `Workspace`, `OrgSettings`, `WorkspaceSettings` | `nebula-core` |
| `TenantContext`, `Principal` | `nebula-core` |
| `OrgRepo`, `WorkspaceRepo` | `nebula-storage` |
| `create_org`, `delete_org` use cases | `nebula-api` (service layer) |
| Tenant middleware | `nebula-api` |

## Migration path

**Greenfield** — tables are new. No existing data to migrate.

**Fold-in constraint:** when adding tenant awareness to any existing crate, every public function taking a user-facing ID must also take `&TenantContext`. This is a workspace-wide refactor touching `workflows`, `credentials`, `resources`, `actions`, `executions`, API handlers, storage.

**Validation gate:** `cargo clippy -D warnings` with custom lint (or manual review) that rejects any function named `find_by_*` / `get_*` / `list_*` / `delete_*` in storage layer that does not accept `&TenantContext`.

## Open questions

- **Sub-organizations** (3-level hierarchy: Account → Org → Workspace) — GCP and Datadog style. Deferred until first enterprise customer asks.
- **Org transfer between billing accounts** — separation of billing entity from org entity. Deferred.
- **Workspace templates** — clone a workspace with its workflows. Nice-to-have for v2.
- **Cross-org workflow export/import** — possible but not designed. Marked planned.
