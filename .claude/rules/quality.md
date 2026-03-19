# Open-Source Quality Standards — Nebula

## Documentation

- Every public item must have a doc comment (`///`) — CI enforces `missing_docs`
- Doc comments describe **what** and **why**, not **how** (the code shows how)
- Include a `# Examples` section for non-trivial public APIs
- `# Errors` section for fallible functions listing when each error variant is returned
- `# Panics` section if the function can panic (should be rare outside tests)

## API Design

- Public API surface is a contract — treat additions as permanent
- Mark experimental APIs with `#[doc(hidden)]` or gate behind a feature flag
- Deprecate before removing: `#[deprecated(since = "0.x.0", note = "use Y instead")]`
- Re-exports in `lib.rs` define the public API — internal modules stay `pub(crate)`
- Exhaustive enums get `#[non_exhaustive]` if they may grow

## Clippy & Formatting

- `cargo clippy --workspace -- -D warnings` must pass (zero warnings policy)
- `cargo fmt --all` with `rustfmt.toml` config (max_width=100, edition 2024)
- Clippy config in `clippy.toml`: cognitive-complexity ≤25, nesting ≤5, fn-params ≤7

## Dependency Hygiene

- `cargo deny check` must pass — licenses, advisories, bans, sources
- Allowed licenses: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, MPL-2.0, Unlicense, CC0
- No `*` version requirements — pin to `"major.minor"` minimum
- Audit new deps: check download count, maintenance status, transitive tree size

## Security

- Credentials encrypted at rest (AES-256-GCM), `SecretString` zeroizes on drop
- No `unsafe` without a `// SAFETY:` comment explaining the invariant
- No `println!` / `eprintln!` in library code — use the `nebula-log` infrastructure
- Sanitize all external input at system boundaries (API handlers, plugin interfaces)

## MSRV

- rust-version 1.93 — CI runs `cargo check` with this exact version
- Don't use nightly features or unstable APIs
- If a dep bumps its MSRV above ours, pin the older version or find an alternative
