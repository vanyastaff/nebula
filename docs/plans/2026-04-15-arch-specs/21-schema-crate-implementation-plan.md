# Schema Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `nebula-parameter` with a new `nebula-schema` crate that provides compile-time-safe field types, unified rule handling through `nebula-validator`, and migration-ready API surface for downstream crates.

**Architecture:** Implement `nebula-schema` in incremental slices. Start with crate scaffold + core value objects (`FieldKey`, visibility/required modes, widget enums), then add field model (`Field` + per-type structs), runtime value types (`FieldValue`, `FieldValues`, `FieldPath`), validation/normalization, and finally callsite migration and deletion of `nebula-parameter`.

**Tech Stack:** Rust 2024, `serde`, `serde_json`, `thiserror`, `nebula-validator`, optional `schemars`, `trybuild` for compile-fail tests.

---

### Task 1: Create `nebula-schema` crate scaffold

**Files:**
- Create: `crates/schema/Cargo.toml`
- Create: `crates/schema/src/lib.rs`
- Modify: `Cargo.toml`
- Test: `cargo check -p nebula-schema`

- [x] **Step 1: Add new workspace member**
Edit root `Cargo.toml` and add `crates/schema` to `[workspace].members`.

- [x] **Step 2: Create crate manifest**
Define `nebula-schema` package metadata, dependencies (`serde`, `serde_json`, `thiserror`, `nebula-validator`), optional `schemars` feature.

- [x] **Step 3: Create top-level module skeleton**
Add crate docs and public module declarations for key/core/error/field/value APIs.

- [x] **Step 4: Run compile check**
Run: `cargo check -p nebula-schema`
Expected: crate resolves and compiles as scaffold.

### Task 2: Implement foundational primitives

**Files:**
- Create: `crates/schema/src/error.rs`
- Create: `crates/schema/src/key.rs`
- Create: `crates/schema/src/mode.rs`
- Create: `crates/schema/src/widget.rs`
- Modify: `crates/schema/src/lib.rs`
- Test: `cargo nextest run -p nebula-schema`

- [x] **Step 1: Add `SchemaError`**
Implement error enum for invalid keys and validator/loader integration points.

- [x] **Step 2: Add `FieldKey`**
Implement validated newtype with grammar checks and string accessor.

- [x] **Step 3: Add `VisibilityMode` and `RequiredMode`**
Implement serde-tagged enums backed by `nebula_validator::Rule`.

- [x] **Step 4: Add typed widget enums**
Implement 7 enums (`StringWidget`, `SecretWidget`, `NumberWidget`, `BooleanWidget`, `SelectWidget`, `ObjectWidget`, `ListWidget`, `CodeWidget`) with serde and defaults.

- [x] **Step 5: Add focused unit tests**
Cover key validation and serde defaults for modes/widgets.

### Task 3: Implement field model (Pattern 4)

**Files:**
- Create: `crates/schema/src/field.rs`
- Create: `crates/schema/src/schema.rs`
- Modify: `crates/schema/src/lib.rs`
- Test: `cargo nextest run -p nebula-schema`

- [x] **Step 1: Implement shared field base + macro**
Introduce `define_field!` macro for shared field set and builders.

- [x] **Step 2: Implement 18 typed field structs**
Add all per-type field structs with type-safe builder methods.

- [x] **Step 3: Implement `Field` enum wrapper**
Add serde-tagged enum + typed constructors + shared accessors + `From<TypedField>`.

- [x] **Step 4: Implement `Schema` aggregate**
Add `Schema::new`, `.add`, `.find`, `.validate`, `.normalize` entry points.

### Task 4: Implement runtime values and paths

**Files:**
- Create: `crates/schema/src/value.rs`
- Create: `crates/schema/src/path.rs`
- Modify: `crates/schema/src/lib.rs`
- Test: `cargo nextest run -p nebula-schema`

- [x] **Step 1: Add `FieldValue` with expression/mode detection**
- [x] **Step 2: Add `FieldValues` typed map helpers**
- [x] **Step 3: Add `FieldPath` root/local path wrapper**
- [x] **Step 4: Add round-trip tests for wire format**

### Task 5: Validation, lint, and compatibility tests

**Files:**
- Create: `crates/schema/tests/compile_fail/*.rs`
- Create: `crates/schema/tests/serde_roundtrip.rs`
- Create: `crates/schema/tests/prototype_translation.rs`
- Test: `cargo test -p nebula-schema && cargo nextest run -p nebula-schema`

- [x] **Step 1: Add compile-fail coverage (`trybuild`)**
- [x] **Step 2: Add serde golden tests for all field variants**
- [x] **Step 3: Add integration tests for translated prototypes**
- [x] **Step 4: Add benchmark scaffold under `crates/schema/benches/`**

### Task 6: Migrate consumers and remove `nebula-parameter`

**Files:**
- Modify: `crates/action/**`, `crates/credential/**`, `crates/resource/**`, `crates/engine/**`, `apps/cli/**`
- Delete: `crates/parameter/**`, `crates/parameter/macros/**` (final PR only)
- Test: `cargo check --workspace && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`

- [ ] **Step 1: Introduce dual-world adapter layer in `nebula-action`**
- [ ] **Step 2: Migrate crate-by-crate callsites to `nebula-schema`**
- [ ] **Step 3: Remove old crates and workspace references**
- [ ] **Step 4: Run full workspace verification**
