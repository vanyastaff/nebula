---
title: nebula-schema — PR-2 (Phase 3 security)
status: implemented
created: 2026-04-22
depends_on: [nebula-schema-pr1-phase2-gap.md]
blocks: [nebula-schema-pr3-phase4-json-schema-plus-docs.md]
spec: ../superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md
roadmap: nebula-schema-roadmap.md
---

# PR-2 — Phase 3 security (schema + credential boundary)

**Goal:** Ship `SecretValue` / zeroize / redaction / resolve-time handling per Phase 3 design, with explicit boundary to `nebula-credential`.

**Blast radius:** `crates/schema/`, workspace deps, `crates/credential/` as required by spec — expect a **larger review** than PR-1.

## Tasks

| ID | Task | Primary paths | Tests |
|----|------|---------------|--------|
| P2-B1 | Workspace deps (`zeroize`, optional KDF crate, `tracing` if spec requires) | root `Cargo.toml`, `crates/schema/Cargo.toml` | `cargo deny check` — no orphan / duplicate pins without justification |
| P2-B2 | `SecretValue` types, redacted `Debug`/`Serialize`, explicit expose API | `crates/schema/src/secret.rs` (new), wire into `value.rs` / `validated.rs` | unit tests |
| P2-B3 | Resolve-time secret handling (+ optional KDF per spec) | `validated.rs`, `field.rs`, builders | integration |
| P2-B4 | Credential migrations / adapters where spec demands | `crates/credential/**` | `cargo test -p nebula-credential` |

**Implementation note (2026-04-22):** P2-B1–B3 and the merge-blocking ADR are implemented on `feat/schema-pr2-phase3-security`. P2-B4 is **deferred**: ADR-0034 keeps the explicit schema seam; `nebula-credential` integration for `SecretWire` / consumers of `get_secret` lands in follow-up PRs (see ADR §Decision item 2).

## ADR (merge-blocking)

Write **ADR before deep implementation** documenting:

- `SecretValue` lives in **`nebula-schema`** (field marker + resolve semantics + redacted views).
- How it relates to existing secret types in **`nebula-credential`** (e.g. avoid duplicating crypto/KDF ownership — prefer `From` / explicit conversion at the seam).
- Any L2 canon touch (`docs/PRODUCT_CANON.md` §12.x) — cross-link.

## Merge gate (PR-2)

All of PR-1 gate on `main` after merge, plus:

```bash
cargo test -p nebula-credential
cargo clippy -p nebula-credential --all-targets -- -D warnings
cargo deny check
```

**Security:** **security-lead review is merge-blocking** for this PR (canon §12.5 / credential safety).

**Perf:** same bench-before/after rule as PR-1 for `nebula-schema` benches touched by secret paths.

## After merge

- Land PR-3 from `nebula-schema-pr3-phase4-json-schema-plus-docs.md`.
- Update `CHANGELOG.md` with a clear security subsection for this PR.
