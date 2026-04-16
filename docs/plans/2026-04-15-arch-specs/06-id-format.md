# Spec 06 — ID format (Prefixed ULID)

> **Status:** draft
> **Canon target:** §3.10 (extend), `GLOSSARY.md` new ID reference table
> **Depends on:** —
> **Depended on by:** everything that has IDs (nearly all specs)

## Problem

Every entity in Nebula needs an identifier. The choice affects:
- Database storage size and index performance
- Log / trace readability and debugging
- URL length and shareability
- Type safety at compile time
- Wire format stability across languages
- Sort order stability for pagination

Getting this wrong means either (a) UUIDs everywhere and `grep` becomes useless, (b) auto-increment integers and every bug becomes a privacy leak, or (c) custom schemes that nobody understands.

## Decision

**Prefixed ULID, Stripe style.** 16 bytes binary in storage, prefixed base32 string on wire (`wf_01J9XYZABCDEF0123456789XYZA`). Generated app-side through extension of existing `domain_key` infrastructure in `nebula-core`. Typed newtype per entity kind enforced through macro.

## Why ULID, not UUID v4 / v7 / KSUID / NanoID

| Format | Size | Sortable | Readable | Issue for Nebula |
|---|---|---|---|---|
| UUID v4 (random) | 16 B | ❌ | ugly | pagination by id broken, index fragmentation on insert |
| UUID v7 (timestamp) | 16 B | ✅ | ugly | correct but Rust ecosystem for v7 is newer; ULID crates more mature |
| **ULID** | 16 B | ✅ | ok | sortable, matches UUID column type, mature `ulid` crate |
| KSUID | 20 B | ✅ | ok | 20 bytes doesn't fit UUID column, requires `BYTEA` (slightly bigger) |
| NanoID | varies | ❌ | ok | not sortable — kills pagination |

ULID has 48 bits of timestamp + 80 bits of randomness, providing:
- **Sortability** — sort by ID ≈ sort by creation time (monotonic variant guarantees same-ms ordering)
- **Binary 16 bytes** — fits `UUID` column type in Postgres (stored as bytes), `BLOB` in SQLite, both efficient for index
- **Cardinality** — 2^80 random bits per millisecond, collision virtually impossible for any real workload
- **Human-ish** — base32 encoded 26 characters, readable enough in logs

## Why prefixes

Prefixes are not decoration. They serve four functional purposes:

1. **Type safety on the wire.** `DELETE /workflows/cred_01J9X...` → server rejects immediately with «wrong id kind: expected `wf_`, got `cred_`». Without prefix, this becomes a silent 404.

2. **Debuggability.** Log line `ERROR: not found: cred_01J9X` tells you it was a credential. Without prefix, you grep schema to figure out the table.

3. **Grep across audit logs.** `grep "cred_01J9X" logs/` finds every touch of that credential across the system. No collision with other entity IDs.

4. **Support tickets.** User pastes «my workflow `cred_01J9X...` isn't running» — you immediately know it's a credential, not a workflow. Saves 30 minutes of clarification.

Stripe considers this so important they have internal linting that blocks PRs introducing new entity types without a prefix.

## ID catalog

v1 prefixes. Add only through explicit update to this list.

| Prefix | Entity | Spec reference |
|---|---|---|
| `org_` | Organization | 02 |
| `ws_` | Workspace | 02 |
| `user_` | User | 03 |
| `sa_` | Service Account | 03, 04 |
| `sess_` | Session | 03 |
| `pat_` | Personal Access Token | 03 |
| `wf_` | Workflow | 13 |
| `wfv_` | Workflow Version | 13 |
| `exec_` | Execution | 16 |
| `node_` | Node Attempt (row in `execution_nodes`) | 16 |
| `cred_` | Credential | 04 |
| `res_` | Resource | — |
| `action_` | Action registration | 11 |
| `plugin_` | Plugin registration | — |
| `job_` | Background job / task | 10 |
| `nbl_` | Nebula instance (process) | 17 |
| `trig_` | Trigger | 11 |
| `evt_` | Trigger event (in inbox) | 11 |

**Reserved for future:**

- `inv_` — Invite token
- `api_` — API key (alternative to PAT)
- `audit_` — Audit log entry

## Data model

### Rust types

```rust
// nebula-core/src/ids.rs
use std::{fmt, str::FromStr};
use ulid::Ulid;
use serde::{Deserialize, Serialize};

/// Trait implemented by every prefixed ID.
pub trait PrefixedId: Copy + Eq + Ord + fmt::Display + FromStr + Serialize {
    const PREFIX: &'static str;

    fn new() -> Self;
    fn as_bytes(&self) -> [u8; 16];
    fn from_bytes(b: [u8; 16]) -> Self;
    fn created_at(&self) -> std::time::SystemTime;
}

/// Macro generates a PrefixedId type.
#[macro_export]
macro_rules! prefixed_id {
    ($name:ident, $prefix:literal $(, docs: $doc:literal)?) => {
        $(#[doc = $doc])?
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Ulid);

        impl $crate::ids::PrefixedId for $name {
            const PREFIX: &'static str = concat!($prefix, "_");

            fn new() -> Self {
                Self(Ulid::new())
            }

            fn as_bytes(&self) -> [u8; 16] {
                self.0.to_bytes()
            }

            fn from_bytes(b: [u8; 16]) -> Self {
                Self(Ulid::from_bytes(b))
            }

            fn created_at(&self) -> std::time::SystemTime {
                self.0.datetime()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", <Self as $crate::ids::PrefixedId>::PREFIX, self.0)
            }
        }

        impl FromStr for $name {
            type Err = $crate::ids::IdParseError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let prefix = <Self as $crate::ids::PrefixedId>::PREFIX;
                let body = s.strip_prefix(prefix).ok_or_else(|| {
                    $crate::ids::IdParseError::WrongPrefix {
                        expected: prefix,
                        got: s.chars().take_while(|c| *c != '_')
                            .chain(std::iter::once('_'))
                            .collect(),
                    }
                })?;
                Ulid::from_str(body)
                    .map(Self)
                    .map_err(|_| $crate::ids::IdParseError::MalformedUlid)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

#[derive(Debug, thiserror::Error)]
pub enum IdParseError {
    #[error("wrong id prefix: expected `{expected}`, got `{got}`")]
    WrongPrefix {
        expected: &'static str,
        got: String,
    },
    #[error("malformed ULID body")]
    MalformedUlid,
}

// Catalog — one line per entity
prefixed_id!(OrgId,            "org");
prefixed_id!(WorkspaceId,      "ws");
prefixed_id!(UserId,           "user");
prefixed_id!(ServiceAccountId, "sa");
prefixed_id!(SessionId,        "sess");
prefixed_id!(PatId,            "pat");
prefixed_id!(WorkflowId,       "wf");
prefixed_id!(WorkflowVersionId,"wfv");
prefixed_id!(ExecutionId,      "exec");
prefixed_id!(NodeAttemptId,    "node");
prefixed_id!(CredentialId,     "cred");
prefixed_id!(ResourceId,       "res");
prefixed_id!(ActionId,         "action");
prefixed_id!(PluginId,         "plugin");
prefixed_id!(JobId,            "job");
prefixed_id!(NodeId,           "nbl");   // Nebula instance, note: collides with "node" in plain English; nbl_ prefix disambiguates
prefixed_id!(TriggerId,        "trig");
prefixed_id!(TriggerEventId,   "evt");
```

**Important note on terminology:** `NodeId` (spelling in Rust) with prefix `nbl_` refers to a **Nebula process instance** (one OS process in multi-process deployment — spec 17). This collides in natural language with «node» as in «workflow graph node», which is called `NodeAttemptId` (`node_` prefix). Rename consideration: `InstanceId` with `nbl_` prefix to avoid ambiguity. Decision deferred to implementation preference.

### Storage representation

**Postgres:**

```sql
-- Each ID column stores 16 bytes, uses either UUID or BYTEA type
CREATE TABLE workflows (
    id          UUID PRIMARY KEY,       -- 16 bytes, indexed
    workspace_id UUID NOT NULL,
    ...
);

CREATE INDEX idx_workflows_workspace ON workflows(workspace_id);
```

`UUID` column type in Postgres stores as 16 bytes, indexes as bytes. ULID fits perfectly.

**SQLite:**

```sql
-- SQLite doesn't have UUID type, use BLOB
CREATE TABLE workflows (
    id          BLOB PRIMARY KEY NOT NULL,  -- 16 bytes
    workspace_id BLOB NOT NULL,
    ...
);
```

`BLOB` of exactly 16 bytes. Same storage size as Postgres.

### Monotonic ULID for hot append paths

Standard ULID: random 80 bits per millisecond. Two IDs created in the same millisecond are not ordered deterministically.

For hot append paths (`execution_journal`, `trigger_events`), use **monotonic** ULID:

```rust
use ulid::Generator;

// One per hot-append component
struct JournalIdGen {
    gen: std::sync::Mutex<Generator>,
}

impl JournalIdGen {
    fn next(&self) -> Ulid {
        self.gen.lock().unwrap().generate().unwrap()
    }
}
```

Monotonic generator increments random portion within same millisecond, guaranteeing total order even for rapid bursts. Cost: tiny mutex contention on hot path. Benefit: deterministic sort, no pagination ghosts.

### Serde behavior

```rust
// Request JSON
{
    "workflow_id": "wf_01J9XYZABCDEF0123456789XYZA",
    "workspace": "ws_01J9XYZ..."
}

// Deserialized
struct Request {
    workflow_id: WorkflowId,   // automatically parsed from prefixed string
    workspace: WorkspaceId,
}

// Response JSON
{
    "id": "wf_01J9XYZ...",    // automatically serialized with prefix
}
```

Rejection of wrong type:

```rust
// This will fail to deserialize with "wrong id prefix: expected `wf_`, got `cred_`"
let req: Request = serde_json::from_str(r#"{
    "workflow_id": "cred_01J9X..."
}"#);
```

Type safety at the wire boundary, not just at compile time.

## SQL encode / decode

```rust
// nebula-storage/src/ids.rs

// sqlx integration
impl<'q> sqlx::Encode<'q, sqlx::Postgres> for WorkflowId {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> sqlx::encode::IsNull {
        let bytes: [u8; 16] = self.as_bytes();
        sqlx::Encode::<sqlx::Postgres>::encode_by_ref(&sqlx::types::Uuid::from_bytes(bytes), buf)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for WorkflowId {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let uuid = <sqlx::types::Uuid as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(WorkflowId::from_bytes(*uuid.as_bytes()))
    }
}

impl sqlx::Type<sqlx::Postgres> for WorkflowId {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <sqlx::types::Uuid as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}
```

**Alternative: blanket impl via trait.** Instead of per-type impl, use blanket impl on the `PrefixedId` trait with custom wrapper:

```rust
// sqlx blanket
impl<'q, T: PrefixedId, DB: sqlx::Database> sqlx::Encode<'q, DB> for T
where
    [u8; 16]: sqlx::Encode<'q, DB>,
{
    fn encode_by_ref(&self, buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>) -> sqlx::encode::IsNull {
        self.as_bytes().encode_by_ref(buf)
    }
}
```

Whichever is cleaner with sqlx's current trait design. Implementation detail.

## Boundary conversions

```
Application code:  WorkflowId(Ulid)                         type-safe
     ↕
API JSON body:     "wf_01J9XYZABCDEF0123456789XYZA"         prefixed string
     ↕
URL path param:    "wf_01J9XYZABCDEF0123456789XYZA" OR slug parsed via FromStr
     ↕
Log / tracing:     workflow_id="wf_01J9XYZ..."              structured field
     ↕
Database:          0x019C... (16 bytes binary)              UUID / BLOB column
     ↕
Metric label:      "wf_01J9XYZ..." (subject to cardinality) prefixed string
```

Conversions happen at boundaries. Inside application code, always typed.

## Testing criteria

**Unit tests (generated by macro):**

```rust
#[test]
fn wf_parse_roundtrip() {
    let id = WorkflowId::new();
    let s = id.to_string();
    assert!(s.starts_with("wf_"));
    assert_eq!(s.parse::<WorkflowId>().unwrap(), id);
}

#[test]
fn wf_rejects_wrong_prefix() {
    let bad = "cred_01J9XYZABCDEF0123456789XYZA";
    assert!(bad.parse::<WorkflowId>().is_err());
}

#[test]
fn wf_rejects_malformed_body() {
    let bad = "wf_XXXXX";
    assert!(bad.parse::<WorkflowId>().is_err());
}

#[test]
fn wf_serde_roundtrip() {
    let id = WorkflowId::new();
    let json = serde_json::to_string(&id).unwrap();
    let parsed: WorkflowId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

#[test]
fn wf_ordering_matches_creation_time() {
    use std::{thread, time::Duration};
    let id1 = WorkflowId::new();
    thread::sleep(Duration::from_millis(2));
    let id2 = WorkflowId::new();
    assert!(id1 < id2);
}

#[test]
fn wf_binary_roundtrip() {
    let id = WorkflowId::new();
    let bytes = id.as_bytes();
    let back = WorkflowId::from_bytes(bytes);
    assert_eq!(back, id);
}
```

Macro emits these 6 tests per type automatically via a helper macro like `prefixed_id_tests!(WorkflowId, "wf")`.

**Integration tests:**
- Encode → DB → decode roundtrip for Postgres and SQLite
- API endpoint with wrong ID type returns 400 with clear error
- Monotonic generator produces ordered IDs under concurrent load

**Property tests:**
- `parse(display(id)) == id` for random IDs
- No two calls to `new()` produce equal IDs (up to 2^80 collision probability per ms)
- Monotonic generator: `id_n < id_{n+1}` always

## Performance targets

- `ulid::Ulid::new()` — **< 100 ns** (measured on modern CPU)
- `Display` encoding — **< 500 ns**
- `FromStr` parsing — **< 1 µs** (validation + base32 decode)
- Monotonic generator lock contention — **< 500 ns** per call at 100k calls/sec

## Module boundaries

| Component | Crate |
|---|---|
| `PrefixedId` trait, `prefixed_id!` macro, `IdParseError` | `nebula-core` |
| All typed ID newtypes (`OrgId`, `WorkspaceId`, etc.) | `nebula-core` |
| Monotonic generator wrappers | `nebula-core` (or `nebula-storage` for journal-specific) |
| SQL encode/decode integration | `nebula-storage` |
| Extended `domain_key` compat layer (existing) | `nebula-core` |

## Migration path

**Existing `domain_key` in `nebula-core`:** whatever format it currently uses (UUID? custom?), extend or replace to emit Prefixed ULID. Exact mechanism depends on current shape. Choices:

1. **Extend** — keep `domain_key`'s outer API, change inner encoding to Prefixed ULID. Call sites unchanged.
2. **Replace** — deprecate `domain_key`, introduce `PrefixedId` trait as described, migrate call sites gradually.
3. **Parallel** — both exist for a transition, eventually remove `domain_key`.

Recommended: **extend**. Less churn. `domain_key` was a good idea, we're upgrading the encoding.

Owner check required: read current `nebula-core::domain_key` shape, decide extension or replacement, open PR.

## Open questions

- **Exact `domain_key` mechanism** — needs source inspection before implementation. Owner action item.
- **Cross-language wire format** — if we later add gRPC or non-Rust SDKs, do we keep prefixed string or switch to raw bytes? Prefixed string is friendly for debugging, raw bytes is faster but unreadable. Likely: prefixed string everywhere, matches Stripe model.
- **Per-process ULID jitter** — should different processes have different random seeds to avoid potential collision during clock sync events? ULID spec says no, 80 bits is enough. Ignored until proven otherwise.
- **ID rotation** — is there ever a case where we re-issue an ID (e.g., privacy regulations)? Design says «no, IDs are permanent». If needed, implement as «soft deletion + new entity», not mutation.
