# Tasks & Backlog

Planned work items for Nebula.

---

## domain-key ID API Migration (external dependency)

**Status:** ✅ Done (2026-03-01)  
**Depends on:** [domain-key](https://github.com/vanyastaff/domain-key) 0.3+

### Goal

Unify UUID ID API in domain-key for better DX:
- `new()` — generate new ID (replaces `v4()` for creation)
- `from(uuid)` — wrap existing UUID (`impl From<uuid::Uuid>`)
- `parse(s)` — parse from string (unchanged)

### Completed

1. ✅ domain-key 0.3 released with `new()`, `From<Uuid>`, `parse()`.
2. ✅ Bumped `domain-key` to 0.3 in workspace `Cargo.toml`.
3. ✅ Migrated all `Id::v4()` → `Id::new()` across workspace.
4. ✅ Migrated `Id::new(uuid)` → `Id::from(uuid)` in core id.rs tests.
5. ✅ Updated `crates/core/src/id.rs` module docs.

### Acceptance

- [x] domain-key supports `new()`, `from()`, `parse()` as described.
- [x] nebula-core and all workspace crates use new API.
- [ ] CI green (webhook tests have unrelated axum path failures).
