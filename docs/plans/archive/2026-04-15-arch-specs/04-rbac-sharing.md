# Spec 04 — RBAC, sharing, service accounts

> **Status:** draft
> **Canon target:** §11.9 (new)
> **Depends on:** 02 (tenancy), 03 (identity)
> **Depended on by:** 05 (API routing), 11 (triggers — scheduled run identity)

## Problem

Every API handler must answer three questions:
1. Is this principal authenticated? (spec 03)
2. Does this principal belong to the requested tenant? (spec 02)
3. **Does this principal have permission for this specific action?** (this spec)

Without a clear RBAC model, permission checks get sprinkled throughout handlers ad-hoc, inconsistencies accumulate, and enforcement becomes swiss cheese. n8n went through this exact problem and rewrote their RBAC twice.

## Decision

**Fixed-role RBAC with hierarchical inheritance**: 4 org-level roles × 4 workspace-level roles, with org roles implying minimum workspace roles. No custom roles in v1. Credentials have role-based visibility (not workflow-derived). Service accounts are first-class non-human principals.

## Data model

### Role enums

```rust
// nebula-core
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
pub enum OrgRole {
    OrgMember   = 0,   // default invited member
    OrgBilling  = 1,   // billing access only (can see invoices, upgrade plan)
    OrgAdmin    = 2,   // manage workspaces, members, credentials
    OrgOwner    = 3,   // everything including org delete, billing
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
pub enum WorkspaceRole {
    WorkspaceViewer  = 0,  // read-only access to workflows, executions
    WorkspaceRunner  = 1,  // can trigger manual executions, use existing workflows
    WorkspaceEditor  = 2,  // create/edit workflows, credentials, resources
    WorkspaceAdmin   = 3,  // manage workspace members, settings, delete workflows
}
```

Roles are ordered — `Admin >= Editor >= Runner >= Viewer`. Permission check becomes `user_role >= required_role`.

### Membership

```rust
pub struct OrgMember {
    pub org_id: OrgId,
    pub principal_id: PrincipalId,     // User or ServiceAccount
    pub role: OrgRole,
    pub invited_at: DateTime<Utc>,
    pub invited_by: Option<UserId>,    // None for org creator
    pub accepted_at: Option<DateTime<Utc>>,  // None while invite pending
}

pub struct WorkspaceMember {
    pub workspace_id: WorkspaceId,
    pub principal_id: PrincipalId,
    pub role: WorkspaceRole,
    pub added_at: DateTime<Utc>,
    pub added_by: UserId,
}
```

### Permission

```rust
pub enum Permission {
    // Workflow
    WorkflowRead,
    WorkflowWrite,
    WorkflowDelete,
    WorkflowExecute,     // trigger manual run
    WorkflowActivate,    // enable/disable triggers
    
    // Execution
    ExecutionRead,
    ExecutionCancel,
    ExecutionTerminate,  // force, Admin only
    ExecutionRestart,
    
    // Credentials
    CredentialRead,      // list, see metadata (NOT the secret)
    CredentialUse,       // use via workflow execution (still no secret visibility)
    CredentialWrite,     // create, update, rotate
    CredentialDelete,
    
    // Resources
    ResourceRead,
    ResourceWrite,
    ResourceDelete,
    
    // Workspace
    WorkspaceRead,
    WorkspaceUpdate,
    WorkspaceDelete,
    WorkspaceManageMembers,
    
    // Organization
    OrgRead,
    OrgUpdate,
    OrgDelete,
    OrgManageMembers,
    OrgManageBilling,
    OrgManageServiceAccounts,
}
```

### Role → permission matrix

**Workspace roles:**

| Permission | Viewer | Runner | Editor | Admin |
|---|:---:|:---:|:---:|:---:|
| `WorkflowRead` | ✅ | ✅ | ✅ | ✅ |
| `WorkflowExecute` | — | ✅ | ✅ | ✅ |
| `WorkflowWrite` | — | — | ✅ | ✅ |
| `WorkflowDelete` | — | — | — | ✅ |
| `WorkflowActivate` | — | — | ✅ | ✅ |
| `ExecutionRead` | ✅ | ✅ | ✅ | ✅ |
| `ExecutionCancel` | — | ✅ | ✅ | ✅ |
| `ExecutionTerminate` | — | — | — | ✅ |
| `ExecutionRestart` | — | — | ✅ | ✅ |
| `CredentialRead` (metadata) | ✅ | ✅ | ✅ | ✅ |
| `CredentialUse` (in exec) | — | ✅ | ✅ | ✅ |
| `CredentialWrite` | — | — | ✅ | ✅ |
| `CredentialDelete` | — | — | — | ✅ |
| `ResourceRead` | ✅ | ✅ | ✅ | ✅ |
| `ResourceWrite` | — | — | ✅ | ✅ |
| `ResourceDelete` | — | — | — | ✅ |
| `WorkspaceRead` | ✅ | ✅ | ✅ | ✅ |
| `WorkspaceUpdate` | — | — | — | ✅ |
| `WorkspaceManageMembers` | — | — | — | ✅ |
| `WorkspaceDelete` | — | — | — | ✅ (org-scoped, see inheritance) |

**Org roles (implied workspace roles across all workspaces in org):**

| Org role | Implied min workspace role | Org-level permissions |
|---|---|---|
| `OrgMember` | none (must be explicitly added to workspace) | `OrgRead` |
| `OrgBilling` | none | `OrgRead`, `OrgManageBilling` |
| `OrgAdmin` | `WorkspaceAdmin` in ALL workspaces | all workspace perms, `OrgRead`, `OrgUpdate`, `OrgManageMembers`, `OrgManageServiceAccounts` |
| `OrgOwner` | `WorkspaceAdmin` in ALL workspaces | everything including `OrgDelete`, `OrgManageBilling` |

### Effective role computation

```rust
pub fn effective_workspace_role(
    org_role: Option<OrgRole>,
    explicit_ws_role: Option<WorkspaceRole>,
) -> Option<WorkspaceRole> {
    let implied = match org_role {
        Some(OrgRole::OrgOwner) | Some(OrgRole::OrgAdmin) => Some(WorkspaceRole::WorkspaceAdmin),
        Some(OrgRole::OrgMember) | Some(OrgRole::OrgBilling) | None => None,
    };
    match (implied, explicit_ws_role) {
        (Some(a), Some(b)) => Some(a.max(b)),  // take the higher
        (Some(r), None) | (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}
```

### SQL schema

```sql
CREATE TABLE org_members (
    org_id             BYTEA NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,    -- 'user' or 'service_account'
    principal_id       BYTEA NOT NULL,
    role               TEXT NOT NULL,    -- OrgRole as string
    invited_at         TIMESTAMPTZ NOT NULL,
    invited_by         BYTEA,
    accepted_at        TIMESTAMPTZ,
    PRIMARY KEY (org_id, principal_kind, principal_id)
);

CREATE INDEX idx_org_members_principal ON org_members (principal_kind, principal_id);

CREATE TABLE workspace_members (
    workspace_id       BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    principal_kind     TEXT NOT NULL,
    principal_id       BYTEA NOT NULL,
    role               TEXT NOT NULL,
    added_at           TIMESTAMPTZ NOT NULL,
    added_by           BYTEA NOT NULL,
    PRIMARY KEY (workspace_id, principal_kind, principal_id)
);

CREATE INDEX idx_workspace_members_principal
    ON workspace_members (principal_kind, principal_id);
```

## Permission check API

```rust
// nebula-core
pub trait PermissionCheck {
    fn check(&self, principal: &Principal, permission: Permission) -> Result<(), AuthorizationError>;
}

pub struct TenantContext {
    pub org_id: OrgId,
    pub workspace_id: WorkspaceId,
    pub principal: Principal,
    pub org_role: Option<OrgRole>,        // cached from middleware
    pub workspace_role: Option<WorkspaceRole>,  // cached, already includes org implication
}

impl TenantContext {
    pub fn require(&self, perm: Permission) -> Result<(), AuthorizationError> {
        let required = required_role_for(perm);
        match required {
            Required::Workspace(role) => {
                if self.workspace_role.map_or(false, |r| r >= role) {
                    Ok(())
                } else {
                    Err(AuthorizationError::InsufficientRole {
                        required: format!("{:?}", role),
                        actual: format!("{:?}", self.workspace_role),
                    })
                }
            }
            Required::Org(role) => {
                if self.org_role.map_or(false, |r| r >= role) {
                    Ok(())
                } else {
                    Err(AuthorizationError::InsufficientRole { /* ... */ })
                }
            }
        }
    }
}

// Static mapping of permission → required role
fn required_role_for(perm: Permission) -> Required {
    use Permission::*;
    use Required::*;
    match perm {
        WorkflowRead | ExecutionRead | CredentialRead | ResourceRead | WorkspaceRead
            => Workspace(WorkspaceRole::WorkspaceViewer),
        WorkflowExecute | ExecutionCancel | CredentialUse
            => Workspace(WorkspaceRole::WorkspaceRunner),
        WorkflowWrite | WorkflowActivate | ExecutionRestart | CredentialWrite | ResourceWrite
            => Workspace(WorkspaceRole::WorkspaceEditor),
        WorkflowDelete | ExecutionTerminate | CredentialDelete | ResourceDelete
            | WorkspaceUpdate | WorkspaceManageMembers | WorkspaceDelete
            => Workspace(WorkspaceRole::WorkspaceAdmin),
        OrgRead => Org(OrgRole::OrgMember),
        OrgUpdate | OrgManageMembers | OrgManageServiceAccounts
            => Org(OrgRole::OrgAdmin),
        OrgManageBilling => Org(OrgRole::OrgBilling),  // or OrgOwner via ordering
        OrgDelete => Org(OrgRole::OrgOwner),
    }
}
```

### Handler pattern

```rust
// Every handler follows this shape
async fn list_workflows(
    State(app): State<AppState>,
    ctx: TenantContext,  // extracted by middleware, already validated
    Path((org_slug, ws_slug)): Path<(String, String)>,
) -> Result<Json<Vec<WorkflowSummary>>, ApiError> {
    ctx.require(Permission::WorkflowRead)?;
    
    let workflows = app.workflow_repo.list(&ctx).await?;
    Ok(Json(workflows))
}

async fn delete_workflow(
    State(app): State<AppState>,
    ctx: TenantContext,
    Path((org, ws, wf_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    ctx.require(Permission::WorkflowDelete)?;
    
    let wf_id = parse_workflow_id_or_slug(&wf_id)?;
    app.workflow_repo.delete(&ctx, wf_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

**Key property:** `ctx.require(perm)` is the **only** place permission checks live. Storage layer does not re-check — it trusts `TenantContext`. Middleware must populate `TenantContext` before handlers run.

## Sharing semantics

### Workflow sharing

Four operations in v1, two deferred:

**Supported in v1:**

1. **Transfer ownership** — move workflow from workspace A to workspace B (same org, requires `WorkspaceAdmin` in both)
2. **Fork / clone** — copy workflow definition to another workspace (source `Editor`, target `Editor`, same org or different)
3. **Export** — download workflow as JSON (`WorkspaceViewer` can export — no secret leak since credentials are references)
4. **Import** — upload JSON, create workflow in workspace (`WorkspaceEditor`)

**Not in v1:**

- Public read-only link (`planned v1.5`)
- Cross-workspace workflow reference (`not supported`)
- Sub-workflow invocation across workspaces (`not supported`)

### Credential sharing

Two scopes:

- **Workspace-local** (default) — credential lives in one workspace, only that workspace's members can use it
- **Org-level with allowlist** — credential lives in org, has explicit `allowed_workspaces: Vec<WorkspaceId>`

```rust
pub struct Credential {
    pub id: CredentialId,
    pub scope: CredentialScope,
    pub display_name: String,
    pub kind: CredentialKind,
    pub encrypted_secret: Vec<u8>,
    // ...
}

pub enum CredentialScope {
    Workspace(WorkspaceId),
    Org {
        org_id: OrgId,
        allowed_workspaces: Vec<WorkspaceId>,
    },
}
```

**Permission check for credential use:**

```rust
pub fn check_credential_access(
    ctx: &TenantContext,
    cred: &Credential,
    required: Permission,
) -> Result<(), AuthorizationError> {
    // Scope check first
    match &cred.scope {
        CredentialScope::Workspace(ws_id) => {
            if *ws_id != ctx.workspace_id {
                return Err(AuthorizationError::CredentialNotInWorkspace);
            }
        }
        CredentialScope::Org { org_id, allowed_workspaces } => {
            if *org_id != ctx.org_id {
                return Err(AuthorizationError::CredentialNotInOrg);
            }
            if !allowed_workspaces.contains(&ctx.workspace_id) {
                return Err(AuthorizationError::CredentialNotAllowedInWorkspace);
            }
        }
    }
    // Role check second
    ctx.require(required)
}
```

**Credential visibility rule (critical, do not break):**

`WorkspaceViewer` sees credential *metadata* — name, kind, last rotation time, last used, allowed workspaces. **Never the secret value.** `WorkspaceRunner` can *use* the credential (engine resolves it in action execution) but also cannot read the secret directly. Only `WorkspaceEditor` and above can rotate or update the secret, and even they never see the current secret plaintext through the API — they provide a new value.

This is the rule that n8n v0 violated — workflow editor could view credential tokens through workflow JSON. We do not repeat this mistake.

### Workspace sharing

Invite flow (from spec 03 — same mechanism, different assignment):

```
OrgAdmin or WorkspaceAdmin → /workspaces/{ws}/members → Add
→ Enter email + workspace role
→ Invite record with workspace assignment
→ Email with accept link
→ On accept: insert workspace_members row
```

**Removing a member:** `WorkspaceAdmin` can remove any member except themselves (last Admin cannot self-remove; transfer first). OrgAdmin can remove anyone including WorkspaceAdmins.

## Service accounts

### Purpose

Non-human identities for:
- CI/CD pipelines calling Nebula API
- Scheduled workflows (cron `source.principal = service_account`)
- Webhook receivers that need to attribute actions
- Cross-system integrations

### Lifecycle

```
1. OrgAdmin creates service account
   - name: "ci-deploy-bot"
   - display_name: "CI Deploy Bot"
   - org: current
2. Service account has no credentials yet — it exists but cannot authenticate
3. OrgAdmin creates PAT for the SA (spec 03 PAT flow, with principal_kind='service_account')
4. SA added as workspace_member with role (same flow as user)
5. CI/CD uses PAT as Bearer token in API calls
6. Request arrives → middleware sees PAT → loads SA as Principal → normal permission checks apply
```

### Scheduled execution identity

When cron trigger fires, execution `source` is:

```rust
ExecutionSource::Cron {
    trigger_id: TriggerId,
    fire_slot: DateTime<Utc>,
    running_as: ServiceAccountId,  // explicit
}
```

**Workflow author configures which service account runs scheduled executions.** Default: a synthetic `sa_cron_default` per workspace, created automatically. Author can override to use a different SA with different permissions — useful for «this cron needs credentials X, but interactive runs don't».

**Critical rule:** scheduled executions run as the **service account**, not as the workflow author. If Alice creates a cron that uses Stripe credentials, the cron runs as SA (not Alice). When Alice leaves the company and her user is deleted, the cron keeps running — it was never tied to her identity.

## Flows

### Permission check flow for API request

```
1. HTTP request arrives with Authorization header
2. Auth middleware extracts token (session or PAT)
3. Auth middleware loads Principal (user or SA)
4. Tenancy middleware parses path: /api/v1/orgs/{org_slug}/workspaces/{ws_slug}/...
5. Tenancy middleware resolves slug → IDs, constructs TenantContext skeleton
6. RBAC middleware loads org_role and workspace_role for principal:
   - SELECT role FROM org_members WHERE org_id=? AND principal_id=?
   - SELECT role FROM workspace_members WHERE workspace_id=? AND principal_id=?
   - Compute effective_workspace_role considering org role implication
7. If principal has no access to org OR workspace → return 404 (not 403, to avoid enumeration)
8. Populate TenantContext.{org_role, workspace_role}
9. Handler runs, calls ctx.require(Permission::X) as needed
10. If permission denied → return 403 with structured error
```

**Note: 404 vs 403.** We return **404 for missing access to tenant**, **403 for insufficient role within tenant the user has access to**. This follows GitHub's convention and prevents enumeration attacks (attacker cannot distinguish «workspace exists but I don't have access» from «workspace doesn't exist»).

### Credential use in execution

```
1. Execution running, action needs credential X
2. Runtime calls CredentialAccessor::get(credential_id)
3. Accessor looks up credential
4. Accessor checks scope:
   - Workspace-local: matches execution.workspace_id?
   - Org-level: matches execution.org_id AND workspace_id in allowed_workspaces?
5. If scope mismatch → ActionError::CredentialNotAvailable
6. Credential decrypted with master key
7. Credential material returned to action
8. Action uses it (e.g., HTTP header)
9. Action completes, credential material dropped (Zeroize)
10. Audit entry: credential_used(id, execution_id, timestamp)
```

**Role check here is not repeated** — the execution is running under the permissions of its starting principal, which was already checked. The scope check is a defense-in-depth for credential misuse via workflow authoring errors, not a primary permission check.

## Edge cases

**Last admin self-removal:** attempt to remove last `OrgOwner` rejected. Attempt to remove last `WorkspaceAdmin` allowed only if some `OrgAdmin`/`OrgOwner` exists in the org (they have implicit Admin via org role).

**Role downgrade with active session:** user's role reduced from `Editor` to `Runner` — their current session continues but next permission check fails for editing operations. No session invalidation.

**Deleted user in `created_by` fields:** handled by `ON DELETE SET NULL`. History shows «[deleted user]».

**Service account creator deleted:** SA persists. Audit shows original creator in immutable audit log, current SA has no live creator reference.

**Credential moved from workspace-local to org-level:** requires re-scoping; need `WorkspaceAdmin` in source + `OrgAdmin` to make org-level. Transaction updates scope and allowed_workspaces in one step.

**Transitive credential access via fork:** if Alice forks a workflow using credential X into workspace B where X is not available, the fork references a non-existent credential. Action execution fails with `CredentialNotAvailable`. Alice must either add X to workspace B or change the fork to use a different credential.

**Permission check during cascading delete:** `WorkspaceAdmin` delete workspace → all workflows, executions, credentials in it deleted. Only checked once at the top — cascade is unconditional.

## Configuration surface

```toml
[rbac]
# No custom roles in v1, but keep the flag for v2
allow_custom_roles = false

# Session invalidation on role change (defense-in-depth)
invalidate_sessions_on_role_downgrade = false  # default false, opt-in stricter

# Service account defaults
[rbac.service_accounts]
auto_create_cron_sa_per_workspace = true
cron_sa_default_role = "WorkspaceRunner"
max_per_org = 100  # plan-specific override
```

## Testing criteria

**Unit tests:**
- Role ordering (`OrgOwner > OrgAdmin > OrgMember`)
- `effective_workspace_role` computation for all org/ws combinations
- `required_role_for(Permission)` is total (every permission has a mapping)
- Credential scope check logic

**Integration tests:**
- User with `WorkspaceViewer` trying to delete workflow → 403
- User with no access to workspace → 404 (not 403)
- User with `OrgAdmin` auto-has `WorkspaceAdmin` on all workspaces
- PAT with restricted scopes honored
- Credential metadata visible to `Viewer`, secret never returned in any API
- Fork with missing credential fails execution cleanly
- Cron execution runs as service account, not author
- Author deletion does not break scheduled workflows
- Last `OrgOwner` cannot remove themselves
- Workspace member removal by admin
- Transfer ownership flow

**Security tests:**
- No endpoint leaks credential secret values in any response
- 404 vs 403 consistency (enumeration prevention)
- Permission check bypass attempts (direct DB, storage layer, gRPC if added)
- Service account without PAT cannot authenticate
- Disabled service account cannot authenticate even with valid PAT

**Property tests:**
- For any (role, permission) pair, check is deterministic (no randomness)
- Adding more roles to a principal never removes permissions
- Role ordering is total (no cycles)

## Performance targets

- Permission check (`ctx.require`): **< 50 µs** (pure in-memory once context loaded)
- Middleware loading of roles: **< 5 ms p99** (two indexed queries)
- Org member list: **< 10 ms p99** for orgs with ≤ 1000 members

## Module boundaries

| Component | Crate |
|---|---|
| `OrgRole`, `WorkspaceRole`, `Permission`, `PrincipalId` | `nebula-core` |
| `OrgMember`, `WorkspaceMember`, `ServiceAccount` types | `nebula-core` |
| `TenantContext`, `effective_workspace_role`, `required_role_for` | `nebula-core` |
| `MembershipRepo`, `ServiceAccountRepo` | `nebula-storage` |
| RBAC middleware (context loading) | `nebula-api` |
| Invite flow handlers | `nebula-api` |
| `CredentialAccessor` (engine side, already exists) | `nebula-engine` |
| Credential scope enforcement | `nebula-credential` |

## Migration path

**Greenfield** — tables new, enum variants new. No existing RBAC to migrate.

**Storage migration discipline:** when modifying `OrgRole` or `WorkspaceRole` enums, migration must:
1. Add new variant first (old code forward-compatible since `#[non_exhaustive]`)
2. Deploy code that handles new variant
3. Migrate existing rows (if needed)
4. Remove old variant in separate release

## Open questions

- **Custom roles** — planned v2, when enterprise customer asks. Design should accommodate adding `CustomRoleId` variant to `OrgRole`/`WorkspaceRole` enums (or switching to registry pattern).
- **ABAC / conditional permissions** — «Alice can edit workflows X, Y but not Z» — deferred. RBAC covers 95%.
- **Deny policies** — GCP-style «explicitly deny even if allowed» — overkill for v1, deferred.
- **Resource-level ACL** — per-workflow or per-credential ACL overriding role — adds significant complexity, deferred until strong need.
- **Impersonation** — org admin acting as another user for debugging — useful for support, deferred with audit log requirement.
- **Session revocation on role change** — defense in depth, currently opt-in via config. Decide whether to make default.
