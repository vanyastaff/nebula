# Spec 07 — Slug contract

> **Status:** draft
> **Canon target:** §12.11 (new)
> **Depends on:** 06 (IDs)
> **Depended on by:** 02 (tenancy uses slugs), 05 (API routing), 13 (workflow versioning)

## Problem

Every user-visible resource has a slug — a human-readable handle that appears in URLs, UI, CLI. Slugs have properties IDs don't:

- They change (rename)
- They collide (two users want «production»)
- They must be safe in URLs, filenames, SQL, JSON
- They must not accidentally match reserved paths (`/api`, `/admin`)
- They must not allow trademark squatting at scale

Every platform that ignored these has paid for it: GitHub rewrote its username rules twice, GitLab had `group.json` route conflict, Slack had case-sensitivity migration, Vercel customer CI pipelines broke on rename.

## Decision

**ASCII slug regex `^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$`, nickname model (not primary key), reserved word list enforced, rename with grace-period history table.** Display name is a separate field allowing Unicode. ULID is always the canonical reference; slug is a user-facing alias.

## Rules

### Character set

```
allowed:     [a-z] [0-9] [-]
not allowed: uppercase, underscore, dot, slash, whitespace, Unicode, special chars

examples:
  ok: "my-workflow", "production", "ci-deploy", "v2-migration", "stripe-prod"
  no: "My Workflow" (space, uppercase)
  no: "my_workflow" (underscore)
  no: "my.workflow" (dot — conflicts with file-extension parsing)
  no: "мой-воркфлоу" (Unicode — use display_name)
  no: "-prod" (leading hyphen)
  no: "prod-" (trailing hyphen)
  no: "my--workflow" (double hyphen)
```

### Structural rules

- **Must start with `[a-z0-9]`** — not with hyphen
- **Must end with `[a-z0-9]`** — not with hyphen
- **No double hyphens** `--` — prevents look-alikes
- **Not all digits** — slug `123` is allowed but discouraged by UI warning (reserved for potential future ID-like semantics)

### Length limits per level

| Level | Min | Max | Rationale |
|---|---|---|---|
| Org slug | 3 | 39 | Matches GitHub username range. UI space, brand-like feel. |
| Workspace slug | 1 | 50 | More flexible, teams use `prod`, `staging`, `default`. |
| Workflow slug | 1 | 63 | Descriptive allowed: `onboard-new-customer-from-stripe`. |
| Credential slug | 1 | 63 | `slack-oauth-marketing`, `postgres-analytics-ro`. |
| Resource slug | 1 | 63 | `redis-cache-east`, `http-client-default`. |
| Service account slug | 3 | 63 | `ci-deploy-bot`, `cron-runner-nightly`. |
| Trigger slug | 1 | 63 | Used in webhook URLs `/hooks/{org}/{ws}/{trigger}`. |

**Hard maximum: 63.** Matches DNS label limit. Keeps URLs reasonable, avoids filesystem issues on any platform.

### Display name (separate field)

- Any Unicode
- Length up to 100 chars
- Can contain spaces, punctuation
- Used in UI rendering, not in URLs or routing
- Example: slug `onboard-user` ↔ display_name `Customer Onboarding (v2)`

```rust
pub struct WorkflowMeta {
    pub id: WorkflowId,
    pub slug: Slug,                    // ASCII validated
    pub display_name: String,          // Unicode allowed, up to 100 chars
    // ...
}
```

## Uniqueness scope

Which scope a slug must be unique in:

| Slug | Unique in |
|---|---|
| Org slug | **global** (`orgs.slug`) |
| Workspace slug | per org (`(org_id, slug)`) |
| Workflow slug | per workspace (`(workspace_id, slug)`) |
| Workflow version number | per workflow (see spec 13) |
| Credential slug (workspace-scoped) | per workspace (`(workspace_id, slug)` with filter on scope) |
| Credential slug (org-scoped) | per org (`(org_id, slug)` with filter on scope) |
| Resource slug | per workspace |
| Service account slug | per org |
| Trigger slug | per workspace |

### SQL enforcement

```sql
-- Org: globally unique, case-insensitive
CREATE UNIQUE INDEX idx_orgs_slug
    ON orgs (LOWER(slug))
    WHERE deleted_at IS NULL;

-- Workspace: unique per org
CREATE UNIQUE INDEX idx_workspaces_org_slug
    ON workspaces (org_id, LOWER(slug))
    WHERE deleted_at IS NULL;

-- Workflow: unique per workspace
CREATE UNIQUE INDEX idx_workflows_workspace_slug
    ON workflows (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

-- ... and so on for each entity
```

**`LOWER(slug)` in index** — case-insensitive uniqueness. But slug regex already forbids uppercase, so this is defense-in-depth for migration scenarios or hand-edited rows.

## Nickname model (S2 strategy)

Slug is **NOT the primary key.** ULID is.

```rust
// Primary key — never changes
pub struct Workflow {
    pub id: WorkflowId,           // wf_01J9... — immutable forever
    pub slug: String,             // current slug — can change
    pub display_name: String,
    // ...
}

// Historical slugs for redirect
pub struct SlugHistoryEntry {
    pub kind: SlugKind,           // Org / Workspace / Workflow / ...
    pub scope_id: Option<Ulid>,   // parent ID (org_id for workspace slug, ws_id for workflow slug)
    pub old_slug: String,
    pub resource_id: Ulid,        // target entity
    pub renamed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
```

**Consequence:** squatting is less severe.

- If Alice takes `acme` as org slug first, real ACME Inc can complain → admin transfers slug, Alice's old slug goes into history with 90-day redirect, then becomes available
- Rename is cheap (update one row + insert history row), not a data migration
- Legal takeovers are a one-update operation, not a multi-day infra project

Without nickname model: slug rename would require cascading updates across every table with FK to slug (which is why we don't FK on slug).

## Reserved words

Must be enforced **on every insert**, not relied on to appear naturally. List lives in `nebula-core::reserved_slugs`:

```rust
// nebula-core/src/reserved_slugs.rs
use std::collections::HashSet;
use std::sync::OnceLock;

static RESERVED: OnceLock<HashSet<&'static str>> = OnceLock::new();

pub fn is_reserved(slug: &str) -> bool {
    let set = RESERVED.get_or_init(|| {
        let words = include_str!("reserved_slugs.txt");
        words.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    });
    set.contains(slug.to_lowercase().as_str())
}

pub fn validate_slug(slug: &str, max_len: usize) -> Result<(), SlugError> {
    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if slug.len() > max_len {
        return Err(SlugError::TooLong { max: max_len, actual: slug.len() });
    }
    if !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(SlugError::InvalidChars);
    }
    if !slug.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err(SlugError::LeadingNonAlphanum);
    }
    if !slug.ends_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err(SlugError::TrailingNonAlphanum);
    }
    if slug.contains("--") {
        return Err(SlugError::DoubleHyphen);
    }
    if is_reserved(slug) {
        return Err(SlugError::Reserved);
    }
    Ok(())
}
```

### `reserved_slugs.txt` contents

```
# Technical / routing
admin
administrator
api
app
apps
auth
billing
cdn
cloud
console
dashboard
dev
docs
documentation
download
download
files
health
help
home
host
hooks
internal
login
logout
me
metrics
new
null
oauth
oauth2
panel
privacy
public
ready
register
root
search
settings
setup
signin
signout
signup
static
status
support
sys
system
terms
test
tests
undefined
user
users
v1
v2
v3
v4
web
webhook
webhooks
www

# ID prefix collision prevention
org
ws
user
sa
sess
pat
wf
wfv
exec
node
cred
res
action
plugin
job
nbl
trig
evt
inv

# Short brand protection (top 100)
amazon
apple
anthropic
facebook
github
gitlab
google
meta
microsoft
netflix
nebula
nvidia
openai
slack
stripe
tesla
# ... etc — document the full list separately

# Product-future-proofing
about
blog
careers
community
contact
faq
features
landing
news
partners
pricing
product
team
teams
```

This file lives in the repo, tracked by git, updated through PRs. ~300 entries total.

**Case-insensitive matching.** User tries `Admin` → `admin.to_lowercase() == "admin"` → rejected.

**Only org slugs get global brand protection.** Workspace / workflow slugs don't need it — they're scoped.

## Rename semantics

### Grace period per level

```rust
pub fn rename_grace_period(kind: SlugKind) -> Duration {
    match kind {
        SlugKind::Org => Duration::from_days(90),
        SlugKind::Workspace => Duration::from_days(30),
        SlugKind::Workflow => Duration::from_days(7),
        SlugKind::Credential => Duration::from_days(7),
        SlugKind::Resource => Duration::from_days(7),
        SlugKind::ServiceAccount => Duration::from_days(30),
        SlugKind::Trigger => Duration::from_days(7),
    }
}
```

**Why per level:**

- **Org** (90 days) — most impactful. Old slug shows in OAuth callback URLs, shared links, CI pipelines, invoices. Long grace gives customers time to update everything.
- **Workspace** (30 days) — medium impact. Most references are internal to the company.
- **Workflow / resource / credential** (7 days) — short impact. References are internal, renaming is rare.

### Rename flow

```
User clicks "Rename workflow" in UI
  ↓
Enter new slug (validated live as user types)
  ↓
POST /api/v1/orgs/{org}/workspaces/{ws}/workflows/{wf_id}
  Body: { "slug": "new-slug" }
  ↓
Server:
  1. Validate new slug (regex, length, reserved)
  2. Check uniqueness in scope (workspace_id, new_slug)
  3. Begin transaction
  4. Insert into slug_history:
     (kind, scope_id=workspace_id, old_slug, resource_id=wf_id, expires_at=NOW()+7d)
  5. Update workflows.slug = new_slug
  6. Commit
  ↓
Response: 200 OK, workflow resource with new slug
  ↓
Audit log: "user X renamed workflow Y from A to B"
```

### Resolution flow with history

```
GET /api/v1/orgs/{org}/workspaces/{ws}/workflows/{slug_or_id}
  ↓
Resolution middleware:
  1. If starts with "wf_" → parse as ULID, look up by id (no history needed)
  2. Else → look up current slug in workflows WHERE workspace_id=? AND slug=?
     - Found: use this workflow, continue
     - Not found: look up in slug_history WHERE kind='workflow' AND scope_id=workspace_id AND old_slug=? AND expires_at > NOW()
       - Found: return 301 Moved Permanently with Location header to canonical URL with current slug
       - Not found: return 404
```

**301 redirect behavior:**
- Applied by API middleware before handler runs
- Response includes `Location: /api/v1/orgs/{org}/workspaces/{ws}/workflows/{new_slug}`
- Client automatically follows (browsers, HTTP libraries)
- CI scripts that bookmark old slug continue to work for grace period
- After grace period, slug returns 404 (or is available for new entity)

### Multiple renames in sequence

```
Day 0: created as "old-name"
Day 10: renamed to "middle-name" → slug_history(old="old-name", target=id, expires=day 17)
Day 15: renamed to "new-name" → slug_history(old="middle-name", target=id, expires=day 22)
                                 → slug_history(old="old-name", target=id, expires=day 17) still exists
Day 16: GET .../old-name → 301 to /new-name (chain through history)
Day 18: GET .../old-name → 404 (expired)
Day 18: GET .../middle-name → 301 to /new-name (still in grace period)
Day 23: GET .../middle-name → 404
```

Resolution algorithm handles chains naturally — each lookup in history returns a target ID, then we resolve that target's current slug.

### Cleanup job

```sql
-- Background job runs daily
DELETE FROM slug_history
WHERE expires_at < NOW() - INTERVAL '1 day';
```

Plus: if a cleanup run leaves a slug available, it can be taken by a new entity (subject to reservation rules).

## Auto-generation from display name

When user enters display name but no slug, UI suggests a slug:

```rust
pub fn slugify(display_name: &str, max_len: usize) -> String {
    // 1. Lowercase
    let lower = display_name.to_lowercase();
    
    // 2. Transliterate Unicode → ASCII via deunicode crate
    let ascii = deunicode::deunicode(&lower);
    
    // 3. Replace non-allowed chars with hyphen
    let mut result: String = ascii.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    
    // 4. Collapse consecutive hyphens
    while result.contains("--") {
        result = result.replace("--", "-");
    }
    
    // 5. Trim leading/trailing hyphens
    result = result.trim_matches('-').to_string();
    
    // 6. Truncate to max length, re-trim in case truncation landed on hyphen
    if result.len() > max_len {
        result.truncate(max_len);
        result = result.trim_end_matches('-').to_string();
    }
    
    // 7. Empty result → fallback to "untitled"
    if result.is_empty() {
        result = "untitled".to_string();
    }
    
    result
}
```

### Examples

```
"My Workflow"                → "my-workflow"
"Customer Onboarding (v2)"   → "customer-onboarding-v2"
"Мой Воркфлоу"               → "moj-vorkflou" (via deunicode)
"漢字テスト"                  → "han-zi-tesuto"
"  leading spaces"           → "leading-spaces"
"---bare-hyphens---"         → "bare-hyphens"
"!@#$%"                      → "untitled"
"👋 emoji"                   → "emoji"
"production"                 → "production" (but check reserved list!)
```

### Collision resolution

If `slugify` produces a slug that already exists:

```rust
pub async fn unique_slug(
    base: String,
    scope: impl SlugScope,
    max_len: usize,
) -> Result<String, SlugError> {
    if !scope.exists(&base).await? && !is_reserved(&base) {
        return Ok(base);
    }
    
    for n in 2..=99 {
        let candidate = format!("{}-{}", base, n);
        if candidate.len() > max_len {
            return Err(SlugError::AutoGenFailed);
        }
        if !scope.exists(&candidate).await? && !is_reserved(&candidate) {
            return Ok(candidate);
        }
    }
    
    // Fall back to ULID suffix if 99 candidates taken
    let ulid_suffix: String = Ulid::new().to_string().chars().take(6).collect();
    let candidate = format!("{}-{}", base, ulid_suffix.to_lowercase());
    Ok(candidate)
}
```

User sees the suggested slug in the form, can accept or override.

## Data model

```sql
-- Slug history table
CREATE TABLE slug_history (
    kind            TEXT NOT NULL,           -- 'org' / 'workspace' / 'workflow' / ...
    scope_id        BYTEA,                   -- NULL for org, otherwise parent id
    old_slug        TEXT NOT NULL,
    resource_id     BYTEA NOT NULL,          -- target entity
    renamed_at      TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (kind, scope_id, old_slug)
);

CREATE INDEX idx_slug_history_expiry
    ON slug_history (expires_at)
    WHERE expires_at > NOW();

-- Partial index to fast-find active entries
CREATE INDEX idx_slug_history_lookup
    ON slug_history (kind, scope_id, LOWER(old_slug))
    WHERE expires_at > NOW();
```

`scope_id` nullable because org slugs have no parent (global scope). Workspace slugs have `scope_id = org_id`, workflow slugs have `scope_id = workspace_id`.

## Edge cases

**Renaming to a reserved word.** Validation rejects early. User sees clear error.

**Renaming to a taken slug within the same scope.** Unique constraint rejects transaction. User sees «slug already in use».

**Renaming to a slug that's in own history.** Allowed — we can take back our previously-used slug. Transaction:
1. Delete our own history entry for that old slug (if any)
2. Insert new history entry for current slug
3. Update slug

**Slug was taken by another entity after grace expired.** Once `expires_at` passes, the slug is free. If another entity takes it, the history entry has already been cleaned up. Historical 301 no longer works. User sees 404.

**Slug collision during auto-generation race.** Two concurrent inserts with same auto-generated slug — one wins via unique constraint, other gets error, client retries with new suffix. Rare but handled.

**Org slug transfer (admin action for trademark disputes).** Admin UI action that:
1. Transfers slug atomically to target org
2. Original org gets a default replacement slug (e.g., `org-{ulid_suffix}`)
3. Original org's old slug enters slug_history with 90-day grace period
4. All audit logged

Only `platform admin` role (new, cloud only) can do this. Self-host: operator can do it via CLI with explicit confirmation.

**Hard-deleted org slug.** After 30-day retention + hard delete (spec 02), slug is available. If history entry exists, it's also cleaned up via FK cascade.

**Backup/restore scenario.** Slugs in backup may conflict with current live slugs after restore to different env. Operator's problem — backups are operator data, conflicts must be resolved manually.

**URL encoding.** Slugs are ASCII, no URL encoding issues. This is part of why we don't allow Unicode.

## Configuration surface

```toml
[slugs]
# Case-insensitive unique check always on
case_insensitive_unique = true   # always true, not user-configurable, listed for clarity

# Grace periods for renames (override defaults above if needed)
[slugs.rename_grace]
org = "90d"
workspace = "30d"
workflow = "7d"
credential = "7d"
resource = "7d"
service_account = "30d"
trigger = "7d"

# Auto-suggestion behavior
[slugs.auto_gen]
max_collision_attempts = 99   # before falling back to ULID suffix
```

## Testing criteria

**Unit tests:**

```rust
#[test]
fn valid_slugs() {
    assert!(validate_slug("my-workflow", 63).is_ok());
    assert!(validate_slug("a", 63).is_ok());
    assert!(validate_slug("a1", 63).is_ok());
    assert!(validate_slug("v2-migration", 63).is_ok());
}

#[test]
fn invalid_slugs() {
    assert!(validate_slug("", 63).is_err());
    assert!(validate_slug("-leading", 63).is_err());
    assert!(validate_slug("trailing-", 63).is_err());
    assert!(validate_slug("double--hyphen", 63).is_err());
    assert!(validate_slug("UPPERCASE", 63).is_err());
    assert!(validate_slug("under_score", 63).is_err());
    assert!(validate_slug("with space", 63).is_err());
    assert!(validate_slug("мой", 63).is_err());  // Unicode
    assert!(validate_slug("with.dot", 63).is_err());
}

#[test]
fn reserved_rejected() {
    assert!(validate_slug("admin", 63).is_err());
    assert!(validate_slug("api", 63).is_err());
    assert!(validate_slug("org", 63).is_err());  // ID prefix
}

#[test]
fn length_limits() {
    let too_long = "a".repeat(64);
    assert!(validate_slug(&too_long, 63).is_err());
    
    let ok_long = "a".repeat(63);
    assert!(validate_slug(&ok_long, 63).is_ok());
}

#[test]
fn slugify_examples() {
    assert_eq!(slugify("My Workflow", 63), "my-workflow");
    assert_eq!(slugify("Customer Onboarding (v2)", 63), "customer-onboarding-v2");
    assert_eq!(slugify("Мой Воркфлоу", 63), "moj-vorkflou");
    assert_eq!(slugify("!@#$%", 63), "untitled");
    assert_eq!(slugify("   ", 63), "untitled");
}
```

**Integration tests:**
- Create workflow with slug → create another with same slug in same workspace → conflict
- Create two workflows with same slug in different workspaces → both succeed
- Rename workflow → old slug returns 301
- Rename workflow → wait past grace → old slug returns 404
- Rename workflow to slug in own history → succeeds
- Auto-generation collision → appends `-2`, `-3`, etc.
- Chain of renames → final resolution works

**Security tests:**
- Reserved word injection (`admin`, `api`) rejected at all levels
- SQL injection attempts on slug parameter rejected by regex
- Path traversal attempts rejected by char set
- Case variation of reserved words rejected (`ADMIN`, `Api`)

**Property tests:**
- `validate_slug(slugify(x, len), len).is_ok()` for any Unicode string
- `slugify` output never contains invalid chars or double hyphens
- `slugify` output never starts or ends with hyphen
- Validated slug is stable: `validate(s) == validate(validate(s).unwrap())`

## Performance targets

- `validate_slug` — **< 1 µs** (single pass, no allocation for happy path)
- `slugify` — **< 50 µs** for typical inputs (dominated by deunicode)
- `is_reserved` — **< 500 ns** (HashSet lookup)
- Slug resolution with cache — **< 1 ms p99 cold**, **< 100 µs warm**
- Slug history lookup — **< 5 ms p99** (single indexed query)

## Module boundaries

| Component | Crate |
|---|---|
| `validate_slug`, `slugify`, `is_reserved` | `nebula-core` |
| `Slug` newtype wrapper (optional, could be `String`) | `nebula-core` |
| `reserved_slugs.txt` asset | `nebula-core/src/` |
| `SlugHistoryEntry`, `SlugKind` | `nebula-core` |
| `SlugHistoryRepo` | `nebula-storage` |
| Resolution middleware (slug → ID) | `nebula-api` |
| Cleanup job (expired history rows) | `nebula-storage` background worker |

## Open questions

- **Paid slug / verified brand** — should enterprise customers get first-come-first-served or priority on brand names? Deferred until legal request.
- **Profanity filter** — industry-standard reserved list for org slugs? Probably yes for cloud, tracked separately from technical reserved list. Community-maintained list via opt-in package.
- **Slug availability API for UI** — `GET /api/v1/orgs/availability/{slug}` to let signup form show "taken" live. Simple, add when signup flow is built.
- **Reverse lookup** — «what's the slug history for entity X» for audit. Nice to have, derivable from `slug_history` rows where `resource_id = X`.
