# Proposals

## P001: ListableStorage extension trait

**Type:** Non-breaking

**Motivation:** Consumers need to list keys (e.g. list workflows, list executions). Current Storage has no list.

**Proposal:** Add extension trait `ListableStorage` with `list_prefix(&self, prefix: &str) -> Result<Vec<Self::Key>, StorageError>`. Storage implementors optionally implement ListableStorage. Memory, Postgres, Redis, S3 can support it.

**Expected benefits:** Additive; backends opt-in; consumers can check `impl ListableStorage` or use `list_prefix` when available.

**Costs:** Another trait; backend impl effort.

**Risks:** Prefix semantics differ (Redis SCAN vs S3 prefix vs Postgres LIKE).

**Compatibility impact:** Additive.

**Status:** Draft

---

## P002: TTL support

**Type:** Non-breaking

**Motivation:** Redis and in-memory caches benefit from TTL. Credentials, sessions may expire.

**Proposal:** Add `set_with_ttl(&self, key, value, ttl: Duration)` to Storage or extension trait. Backends that support TTL implement it; others return error or ignore.

**Expected benefits:** Automatic expiry; reduces manual cleanup.

**Costs:** Trait extension; not all backends support (Postgres would need scheduled job).

**Risks:** Inconsistent behavior across backends.

**Compatibility impact:** Additive.

**Status:** Defer

---

## P003: StorageProvider alignment with credential

**Type:** Non-breaking (investigation)

**Motivation:** nebula-credential has StorageProvider (store, retrieve, delete, list). nebula-storage has Storage (get, set, delete, exists). Similar but different.

**Proposal:** Investigate adapter: `StorageProvider` impl that wraps `Storage<CredentialId, EncryptedData>`. Or document that they serve different purposes (credential = encrypted, domain-specific; storage = generic kv).

**Expected benefits:** Reuse storage backends for credential if applicable.

**Costs:** Adapter layer; credential may need list with filter.

**Risks:** StorageProvider has list with filter; Storage has no list yet.

**Compatibility impact:** Additive if adapter.

**Status:** Defer
