# Research digests (supplementary)

Dense notes mined from official docs, Nomicon, RFC clusters, Tokio/async books, ecosystem pages, and blogs. **Same tag vocabulary** as the main guide (`[TAG: 01-meta-principles]`, `[TAG: 02-language-rules]`, …) so you can cross-reference `../01-meta-principles.md` … `../09-appendices.md`.

## Authority

- **Normative language semantics:** [The Rust Reference](https://doc.rust-lang.org/reference/) and rustc version pinned in `rust-toolchain.toml`.
- **Nebula product and layers:** `docs/PRODUCT_CANON.md`, `docs/STYLE.md`, `deny.toml` — these digests do **not** override them.
- **Conflict:** if a fetched note disagrees with the Reference or a later edition, **believe the Reference**; clusters may lag (stated in cluster-02).

## Files (reading hints)


| File                                                                                               | Focus                                                                       |
| -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| [cluster-01-book-examples-std.md](cluster-01-book-examples-std.md)                               | Book, RBE, std, style, edition, error codes — dense bullets + `[source: …]` |
| [cluster-02-nomicon-unsafe-rfcs.md](cluster-02-nomicon-unsafe-rfcs.md)                           | Nomicon, unsafe guidelines, RFC/dev-guide excerpts, modern RFCs             |
| [cluster-03-cargo-rustc.md](cluster-03-cargo-rustc.md)                                           | Cargo, rustc, profiles, features                                            |
| [cluster-04-perf-effective-macros.md](cluster-04-perf-effective-macros.md)                       | Performance, Effective Rust, macro/proc-macro notes                         |
| [cluster-05-atomics-locks.md](cluster-05-atomics-locks.md)                                       | Atomics, memory order, locks                                                |
| [cluster-06-async-tokio.md](cluster-06-async-tokio.md)                                           | Async/Tokio (parallel to async-tokio expert file)                           |
| [cluster-07-embedded-wasm-cli-secure.md](cluster-07-embedded-wasm-cli-secure.md)                 | embedded, wasm, CLI, security                                               |
| [cluster-08-cookbook-rustlings-comprehensive.md](cluster-08-cookbook-rustlings-comprehensive.md) | Cookbook, Rustlings-style drills                                            |
| [cluster-09-ecosystem.md](cluster-09-ecosystem.md)                                               | Crate ecosystem notes                                                       |
| [cluster-10-blogs.md](cluster-10-blogs.md)                                                       | Blog / article digests                                                      |
| [async-tokio-expert-notes-fetched.md](async-tokio-expert-notes-fetched.md)                       | Async book + Tokio tutorial/topics — taxonomy table + sections              |


## How agents should use this

- Prefer the **numbered guide** (`../README.md`) for **actionable rule IDs** and PR checklist; use **research/** when you need **extra citations**, error-code patterns (E0xxx), or ecosystem specifics.
- Do not treat blog excerpts as normative — verify against official docs when behavior matters.
- For local environment/bootstrap commands, treat `docs/dev-setup.md` as the source of truth; research cluster snippets are supplementary.

