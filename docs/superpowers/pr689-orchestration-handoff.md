# PR #689 — orchestration handoff (session-disposable)

Durable orchestration state so a fresh session can resume WITHOUT this
session's context. Source of truth = git + this file. Not coderabbit-reviewed
(docs/superpowers/ is path-filtered).

## What this is

`nebula-storage` spec-16 port/adapter/tenancy redesign. PR
**https://github.com/vanyastaff/nebula/pull/689**, branch
`youthful-curran-5fc879`, worktree
`C:\Users\vanya\RustroverProjects\nebula\.claude\worktrees\youthful-curran-5fc879`.
P1–P7 of the plan are COMPLETE & committed (the spec-16 collapse). PR is open;
now resolving (A) merge conflicts + (B/C/D) review feedback.

## Active worker

implement-coordinator agent id `a23c9aac06c7b5c0e` (name `pr689-coord`),
running in background in THIS worktree. Resume it via
`SendMessage(to:"a23c9aac06c7b5c0e", ...)`. If its transcript is lost (routine
~every long run): spawn a fresh `implement-coordinator` with a brief built from
this file + `git status` + the coordinator's last phase report. NEVER relocate
to a different worktree while `MERGE_HEAD` is active (a fresh worktree won't
inherit the in-progress merge).

## PROGRESS LOG (fact-only; durable resume state)

- **(A) MERGE — done&verified.** `be42f4e0`, 2-parent, origin/main ancestor (PR mergeable).
- **(B) P1 — done&verified.** `da216ca4` atomic `WorkflowStore::save_with_published_version`.
- **(C) P2 — done&verified.** Folded into `087467c0`: `workflow_list` now uses
  dual-format `extract_timestamp` (codex 3255507515).
- **`087467c0` — done&verified.** Three coherent fixes:
  1. **InMemory workflow-store footgun (regression from da216ca4).** P1's
     `save_with_published_version` wrote the version into the row store's
     map, but `InMemoryWorkflowStore::new()` gave a *private* unshared
     map; every site except the conformance matrix paired `new()` with a
     separate version store → created workflows 404'd (10 nebula-api
     tests, masked by fail-fast at HEAD). Removed the `new()`/`Default`
     footgun; `new_with_versions(&versions)` is the sole constructor
     (mirrors `InMemoryControlQueue`/`InMemoryJournalReader`). Rewired
     all 11 call sites.
  2. **(C)** workflow_list dual-format sort key (3255507515).
  3. **Pre-existing merge drift (independent, fixed not masked):**
     `From<nebula_storage::StorageError> for ApiError` mapped non-caller
     faults to `Internal`, but #688's test +
     `map_resource_create_storage_error` doc require the opaque `Storage`
     variant. Faults now map through the port `StorageError`. Classified
     pre-existing: reproduced at clean `da216ca4`; #688 test vs HEAD impl
     drifted across the merge; unrelated to footgun/C.
  - Verified: nebula-api 366/366 (1 pg-gated skip), storage+tenancy+
    storage-port 272/272, touched crates fmt + clippy -D warnings clean,
    lefthook green.
- **REMAINING: (D) only** — work the VERIFIED REVIEW TRIAGE below
  (FIX/ADOPT/PUSHBACK; some items discharged by da216ca4 — verify with
  `git show da216ca4 --stat` before redoing). Then per-crate gates →
  `git push` (no force) → per-comment_id factual replies + resolve.

## Finish sequence (strict order)

- **(A) MERGE — blocking, in progress (~90%).** `git merge origin/main`
  (origin +10 / branch +54). Resolve every conflict ONTO main's CANONICAL
  restructured layout (`crates/api/src/domain/<area>/{dto,handler}.rs`; main
  #671/#677/#678/#688 deleted the old `models/`+`handlers/`+`routes/` — do NOT
  resurrect). HEAD's spec-16 port imports win where legacy repos were deleted;
  verify no origin/main #678/#688 logic dropped by auto-merge (e.g.
  `resource_registrars` present — already verified). ADR renumber DONE:
  storage ADR is `docs/adr/0068-nebula-storage-spec16-port-adapter-tenancy.md`
  (0066=credential-runtime #678, 0067=resource #688 from main); cross-refs
  updated. Last conflicted file: `crates/api/tests/knife.rs` (re-express the
  §13 harness + origin's net-new step3/step5 onto port:
  `WorkflowEngine::new(rt,metrics).with_execution_stores(store_seam::ExecutionStores{execution,journal,node_results,checkpoints,idempotency}).with_workflow_stores(...)`,
  AppState scoped handles `state.{execution_store,workflow_store,workflow_version_store,control_queue,node_result_store,journal_reader}` — NOT the deleted `state.*_repo`). Also re-express
  `crates/api/tests/common/mod.rs` `engine_seam` module, `resource_handlers.rs`
  (2 legacy `AppState::new` 4-arg sites), `me_e2e.rs:8` stale doc-comment.
  Then `git checkout --theirs Cargo.lock && cargo build -q && git add Cargo.lock`
  (lockfile discipline; never `cargo update -p`). Drive workspace+lefthook-green
  → COMMIT the merge (one coherent merge commit; never stash; MERGE_HEAD +
  working tree persist across turns/transcript loss).
- **(B) P1 fix** — atomic workflow save (see triage 3255507511/3255514541).
- **(C) P2 fix** — workflow_list dual-format created_at sort (3255507515).
- **(D)** per-crate gates (storage-port/storage/tenancy/engine/api/storage-loom-probe:
  `cargo fmt -p` + `cargo clippy -p <c> --all-targets --all-features -- -D warnings`
  + `cargo nextest run -p <c>`; knife+loom+conformance+identity_conformance;
  Postgres DATABASE_URL-gated skip-clean, never claim pg-verified) → `git push`
  (normal, no force) → for each comment_id below: one-line factual thread reply
  via `gh api repos/vanyastaff/nebula/pulls/689/comments/{id}/replies` + resolve
  the thread (GraphQL resolveReviewThread). No performative language. Skip
  replies for non-actionable bot comments (copilot too-big, codex/coderabbit
  summaries).

## VERIFIED REVIEW TRIAGE (irreplaceable — evaluated per receiving-code-review)

Apply AFTER the merge commit (line-refs shift post-merge; re-locate by symbol).
"FIX" = valid, implement. "ADOPT" = accept the hardening. "PUSHBACK" = reasoned
decline, reply with reason, do NOT implement unless the user insists.

**Security / correctness — FIX:**
- `3255514540` api state.rs placeholder_scope hardcoded `Scope::new("nebula","nebula")` → all tenants share scope, **defeats the P3 tenancy boundary**. Thread the real request tenant `Scope` (from `TenantContext`/principal) through the handlers into the port handle calls; remove `placeholder_scope()`. Heavy but mandatory — the headline security property.
- `3255514561` storage inmem/execution.rs scope-mismatch returns `VersionConflict{actual: row.version}` (unknown id → 0) = cross-tenant version oracle. Return `actual: 0` on scope mismatch.
- `3255507511` + `3255514541` api workflow_save: row write then version write = two non-atomic awaits → orphan row / bumped-counter-without-version; readers skip rows w/o published version (workflow "vanishes"). STRUCTURAL fix: add an atomic combined port unit-of-work `WorkflowStore::save_with_published_version(&scope, WorkflowRecord, WorkflowVersionRecord, expected_version)` (or `WorkflowSaveBatch`), atomic per backend (one tx SQLite/Postgres; one mutex-mutation InMemory), thread through `nebula-tenancy` ScopedWorkflowStore, swap workflow_save to the single call. NOT a best-effort/compensation band-aid.
- `3255507515` api workflow_list sort uses `Value::as_i64` on `created_at` but bug-fix #3 writes RFC3339 strings → all sort 0 → id-only order. Parse dual-format (reuse `extract_timestamp`).
- `3255514542` api workflow_count = full `list().len()` on /ready (O(n)). Add `WorkflowStore::count(&Scope)->u64` (impl 3 backends + decorator), use it.
- `3255514543` engine control_consumer processor-id silent truncate/pad to 16B → cross-worker fence collapse. Use typed `[u8;16]` processor id end-to-end.
- `3255514559` storage migrations/postgres/0027 column drift: `lease_expires_at`/`processed_at`/`expires_at` (TIMESTAMPTZ) vs SQLite `*_ms` (BIGINT). Normalize to one cross-dialect contract — `*_ms BIGINT` both dialects (this is MY 0027 schema bug; also fix the postgres adapter rows that read these).
- `3255514577` storage inmem/identity.rs update paths skip create's active-uniqueness (email/slug). Add same uniqueness scan to all update sites (90-104,174-188,267-282,456-476).
- `3255514579` storage inmem/identity.rs blob `evict_expired` lexical RFC3339 compare. Parse+compare timestamps (mirror idempotency cache).
- `3255514586` storage postgres/idempotency_store.rs `trigger_id` `unwrap_or_default()` masks decode err. `try_get(...).map_err(conn_err)?`.
- `3255514589` storage postgres/identity.rs systemic `try_get().unwrap_or_default()` in all `*_from_row` → silent corruption. Make helpers return `Result`, propagate required-field errors (`.ok()` only truly-optional cols).
- `3255514593` storage postgres/workflow.rs `unwrap_or_default()` decode (80,102,192). Fallible get + `.transpose()`.
- `3255514595` storage postgres/workflow.rs `update` lacks `deleted=FALSE` → revives soft-deleted; disambig SELECT too. Add `AND deleted = FALSE`.
- `3255514598` storage sqlite/control_queue.rs claim UPDATE by id only + unconditional push → returns work not actually claimed. `WHERE id=? AND status='Pending'` + only push when `rows_affected==1`.
- outside-diff workflow.rs:729-740 `validate_workflow_handler` uses `from_value` while activate uses `from_str` → inconsistent accept. Use `from_str` both (same class as bug #3).
- outside-diff workflow.rs:289-307 create path persists client `id/version/owner_id/schema_version` verbatim; update guard strips them. Strip on create too.
- `3255514546/47/48/49` + dto/workflow.rs:24-27 + inmem/execution.rs:197-200: project guideline requires `// guard-justified: <reason>` directly above every `#[allow]`/`unreachable!`. Convert all existing prose rationales to the exact marker. Mechanical batch.

**Hardening — ADOPT:**
- `3255514551` storage-port store/idempotency.rs: add explicit `&Scope` param to `get`/`put` (don't rely on pre-namespaced key). Consistent with the rest of the port taking `&Scope` + §6 intent; update impls (3 backends) + decorator + api caller.
- `3255514553` storage-port store/identity.rs membership: replace `scope_kind`/`principal_kind` `&str` with `enum ScopeKind{Org,Workspace}` / `PrincipalKind{User,ServiceAccount}` (compile-time authz-domain safety). Update impls + call sites.
- postgres/control_queue.rs:201-203 `cleanup` is a silent `Ok(0)` no-op. Implement the retention DELETE (operational honesty per ADR-0008) — small.
- store_seam.rs:57-61 `node_result_record` silent `"Unknown"` for missing `type` → add `tracing::warn!` + `debug_assert!` (observability DoD).
- control_consumer.rs:248-252 malformed traceparent dropped silently → `warn!` + counter (observability DoD; cheap).
- storage/src/lib.rs:3-11 + credential/layer/mod.rs:10-11 + storage-port/README.md:27 (MD040 ```text fence): stale doc fixes (post-collapse `InMemory*Store` names; `CredentialScopeLayer`). Trivial.

**Reasoned PUSHBACK (reply, do NOT implement unless user insists):**
- `3255514555` storage-port store/node_result.rs "raw vs typed newtypes": the engine call sites do not interchange `save_node_output`/`save_node_result` inputs; the spec deliberately uses one `NodeResultRecord`. Adding `RawNodeOutput`/`TypedNodeResult` wrappers is churn the spec didn't call for with no demonstrated swap at any call site. Reply: declined as YAGNI for current call sites; revisit if a real swap risk emerges. (Defensible to adopt if the user/owner wants max type-safety — escalate if they insist.)

## Load-bearing rules (unchanged, enforce)

Expand-contract/no-shims; no `unwrap/expect/panic!` in lib (tests/const/bins
exempt); typed StorageError + tracing + invariant on new state/error/hot;
no plan IDs in committed code; conventional commits, scope=crate w/o `nebula-`
or top-level area, body ends EXACTLY `Co-Authored-By: Claude Opus 4.7 (1M
context) <noreply@anthropic.com>`; decompose git; stage Cargo.lock on dep
change; lefthook gates EVERY commit (typos/taplo/fmt-check/cargo-deny/clippy
--workspace -D warnings/convco) — never `--no-verify`; per-crate verify only
(Windows os err 206 on `task dev:check`/`cargo fmt --all`); Postgres
DATABASE_URL-gated skip-clean, never claim pg-verified without it; commit every
green increment, never cross a turn boundary with uncommitted COMPILING work
(the active merge is the one allowed multi-turn uncommitted unit — never
stash); honesty classification done&verified / done-but-pg-unverified /
partial / not-started; phase-boundary command-evidenced reports.
Stop-gate hook fires while the tree is dirty mid-merge — that is expected, NOT
a done-claim; the gate is satisfied by lefthook at commit + per-crate (D) +
independent spot-verify before any "done".

## Lead Adjudications & Recovery Log (durable; session is disposable)

**Blocker-2 (DECIDED — credential `ScopeResolver` home).** Do NOT add a
`deny.toml [wrappers]` Exec→Business edge (`credential-runtime → tenancy`)
— boundary erosion. Spec §3 data-vs-policy split applied: the
`ScopeResolver` **trait** moved DOWN to `nebula-credential` (contract
crate; `crates/credential/src/store.rs`, re-exported `crates/credential/src/lib.rs`
`pub use store::{… ScopeResolver …}`). The concrete `CredentialScopeLayer`
policy + decorator stay in `nebula-tenancy` (`crates/tenancy/src/credential_scope.rs`
now `use nebula_credential::ScopeResolver`; local trait def deleted).
`nebula-tenancy` re-exports `pub use nebula_credential::ScopeResolver as
CredentialScopeResolver` (lib.rs) — name-compat alias of the canonical
trait, NOT a back-compat surface re-export. `credential-runtime/src/scope.rs`
now `use nebula_credential::ScopeResolver` (both sites, downward-legal —
already deps `nebula-credential`; no `deny.toml` change; `nebula-credential`
is shared-infra with no `[[wrappers]]` gate). Legacy
`nebula_storage::credential::{ScopeLayer,ScopeResolver}` surface stays
DELETED (spec-16 CONTRACT phase); zero back-compat re-export. Stale docs
fixed: `tenancy/src/lib.rs` (false back-compat claim → canonical-path
statement), `api/src/transport/credential.rs:56`
(`nebula_storage::credential::ScopeLayer` → `nebula_tenancy::CredentialScopeLayer`).
`storage/src/credential/layer/mod.rs` doc already correct (re-homed
statement accurate post-merge).

**Blocker-1 (lost MERGE_HEAD — recovery method, do LAST after green).**
Resolved tree is fully staged + intact (NEVER stash). Recreate the
2-parent merge commit only after workspace+lefthook-green:
`git checkout --theirs Cargo.lock && cargo build -q && git add -A` →
`echo "$(git rev-parse origin/main)" > "$(git rev-parse --git-path MERGE_HEAD)"`
→ write the merge message to `"$(git rev-parse --git-path MERGE_MSG)"` →
`git commit` (NORMAL commit; MERGE_HEAD present ⇒ true 2-parent merge +
lefthook runs; never bypass) → VERIFY
`git cat-file -p HEAD | grep -c '^parent'` == 2 AND
`git merge-base --is-ancestor origin/main HEAD && echo MERGED`. Only then
is (A) done. Merge-msg body ends EXACTLY
`Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
