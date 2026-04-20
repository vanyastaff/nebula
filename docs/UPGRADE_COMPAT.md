# Upgrade Compatibility

Scope: expanded compatibility guidance for `docs/PRODUCT_CANON.md` §7.2.

## Pre-1.0 reality (current state)

Nebula is at workspace version **`0.1.0`** — **alpha**. No tagged release has shipped. Until the first tagged release:

- **Every commit on `main` may break** workflow JSON, execution semantics, plugin SDK types, and storage schema.
- **There is no compatibility matrix to populate** — this document describes the **policy that will apply starting with the first tagged release**, plus guidance for operators running off `main` today.
- Operators running off `main` should **pin a commit SHA**, not a moving branch, and re-validate workflows + rebuild plugins after every bump.
- **Schema migrations** for both SQLite and Postgres live in `crates/storage/migrations/`; they are the only load-bearing compatibility surface today and must run cleanly forward.

When the first release ships, populate the matrix in [§Compatibility matrix](#compatibility-matrix) and update `README.md` to point here.

## Compatibility surfaces

These are the three independent surfaces any upgrade must account for. A release note must classify change on **each** axis.

1. **Workflow definitions** — persisted JSON shape + activation semantics (see §10 / §13 in the canon).
2. **Engine / runtime behavior** — execution semantics, durability guarantees, control queue contract (§11, §12.2).
3. **Plugin SDK + binaries** — `nebula-api` / `nebula-sdk` source API + native Rust plugin binary linkage.

A single upgrade may touch one axis and leave the others intact — release notes must be explicit about **which** and link to migration steps.

## Baseline policy (post-1.0)

These rules apply **starting with the first tagged release**. They are stated here so they do not have to be invented under pressure when the first release ships.

- **Patch** (`x.y.Z`): no break on any surface. Bug fixes, docs, non-semantic internal refactors. Workflow JSON and plugin SDK **must** stay forward-compatible; native plugin binaries **should** relink without source changes.
- **Minor** (`x.Y.0`): additive on public surfaces. New fields / new variants / new endpoints allowed; removal, rename, or semantic change is **not**. Native plugin binaries may require recompile against the new SDK — document it in release notes if so.
- **Major** (`X.0.0`): breaking changes allowed on any surface, but each break must ship with a migration note, tests covering the upgrade path, and an entry in the matrix below.

**Mandatory for every release:**

- Release notes list changes per surface (workflow / engine / SDK + binary).
- **SQLite and Postgres migrations** are both run and tested — schema parity is non-negotiable (see `crates/storage/migrations/{sqlite,postgres}/README.md`).
- **Knife scenario** (canon §13) and **integration bar** stay green against the target version before tagging.
- No surface silently drops a capability advertised in a prior `README.md` or `docs/`.

**Forbidden regardless of release type:**

- Advertising “v1 workflows run unchanged on v2” without a populated matrix row to prove it.
- Shipping a `false capability` (canon §4.5, §11.2 retry example) across a version boundary and claiming it works.
- Removing a storage migration or skipping a dialect — both SQLite and Postgres must track each schema change.

## Plugin binary reality

- Rust plugin crates are **compiled artifacts** tied to a specific `nebula-api` / SDK version. A minor or major bump of the engine **may** require rebuilding plugins against the target version — this is normal, not a break.
- Binary-stable ABI is **only** promised on explicit FFI paths (e.g. the `stabby` path, when implemented). Native Rust plugin binaries are **not** implicitly ABI-stable across engine upgrades — do not promise otherwise.
- Plugin authors: pin `nebula-api = "=x.y.z"` or a tight semver range in `Cargo.toml` and re-test on every engine bump. Plugin hosts: `plugin.toml`’s `sdk = "..."` constraint must match the engine version actually loaded.
- Cross-plugin dependencies (canon §7.1) go through `Cargo.toml` `[dependencies]` on the provider plugin crate — version resolution is **Cargo’s** job, not Nebula’s.

## Compatibility matrix

Populate one row per **tagged** release. Empty until the first tag ships.

| Engine version | Release date | Workflow JSON compat | Plugin SDK source compat | Native plugin binary compat | Storage migration (SQLite / Postgres) | Notes / migration link |
| -------------- | ------------ | -------------------- | ------------------------ | --------------------------- | ------------------------------------- | ---------------------- |
| _none yet_     | _pre-1.0_    | n/a                  | n/a                      | n/a                         | n/a                                   | Running off `main` — pin commit SHA and rebuild plugins on every bump. |

**Legend:**

- **Workflow JSON compat:** `yes` = definitions from prior version load and activate without edits; `breaking` = migration required; `n/a` = pre-1.0.
- **Plugin SDK source compat:** `yes` = plugins compile against the new engine without code changes; `rebuild` = relink only; `breaking` = source edits required.
- **Native plugin binary compat:** almost always `rebuild` for native Rust; only `yes` if explicitly FFI-stable.
- **Storage migration:** `fwd-only` (normal), `fwd+back` (reversible), or `destructive` (data loss possible, must be in release notes headline).

## Upgrade checklist (operators)

Before upgrading a deployment:

1. **Back up** the storage layer (SQLite file or Postgres dump) — migrations are `fwd-only` by default.
2. Read the release notes for **all three surfaces** — workflow, engine, SDK.
3. **Validate** stored workflow definitions against the new engine before activation — use `nebula_workflow::validate_workflow` or equivalent (canon §10).
4. Verify the **control queue path** (`execution_control_queue` → engine consumer) is wired for the new deployment mode — a broken cancel path is a §12.2 violation, not a minor bug.
5. **Rebuild and retest plugins** against the target SDK. Check `plugin.toml`’s `sdk = "..."` constraint against the new engine version.
6. Run the canon **knife scenario** (§13) end-to-end against the target version — define → activate → execute → cancel → terminal `Cancelled` — before letting real workflows through.
7. Confirm **observability paths** (journal, structured errors, metrics) still answer “what happened” for a failed run without reading Rust source (canon §4.6, §9 north star).

## 2026-04-20 — `DUPLEX_PROTOCOL_VERSION` 2 → 3

**Breaking (plugin-SDK).** Plugin binaries compiled against
`nebula-plugin-sdk` version-2 no longer handshake with the current host.
Rebuild all plugin binaries against the current SDK; the change is
**additive to the envelope** — replacing flat `plugin_key` /
`plugin_version` with the full `PluginManifest`, adding per-action
`schema: ValidSchema` — so plugin authors re-compile and ship; no
source-level migration beyond importing `PluginManifest` from
`nebula_metadata` and constructing it via the builder.

See [plugin load-path stabilization design
spec](superpowers/specs/2026-04-20-plugin-load-path-stable-design.md).

## See also

- `docs/PRODUCT_CANON.md` §7.2 — normative upgrade contract
- `docs/ENGINE_GUARANTEES.md` — durability matrix (what survives an upgrade)
- `docs/INTEGRATION_MODEL.md` §7 — plugin packaging and SDK boundary
- `crates/storage/migrations/{sqlite,postgres}/README.md` — schema parity truth
