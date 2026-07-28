# CLAUDE.md

[`AGENTS.md`](AGENTS.md) is the canonical agent guide: project map, layer rules,
architecture invariants, commands. Read it before non-trivial work, plus
`crates/<crate>/AGENTS.md` for the crate you're touching.

The few things worth knowing before you open anything:

- **Layers are CI-enforced.** `deny.toml` `[bans].deny` wrappers fail the build on an
  upward or undeclared-lateral crate dependency. Check the layer map in `AGENTS.md`
  before adding a cross-crate dep.
- **`#[expect(lint, reason = "...")]`, never bare `#[allow]`** — `allow_attributes` is
  denied workspace-wide. Exception and `cfg_attr` gating: `AGENTS.md` §Conventions.
- **No `unwrap()`/`expect()`/`panic!()` in library code.** Tests, `const`, and binaries
  are exempt.
- **`task dev:check`** is the pre-PR gate; `cargo check -p nebula-<name>` is the fast loop.
