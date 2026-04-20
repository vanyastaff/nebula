# Apps

Application targets for Nebula — composition roots that wire library crates
into runnable artefacts.

## Structure

- `apps/cli` — `nebula` CLI for in-process one-shot runs and the optional
  `--tui` execution viewer (release artefact).
- `apps/desktop` — Tauri 2.x reference shell. **Not a release artefact** —
  see `apps/desktop/README.md`.

The audit-driven `library-first` direction places the production
composition root for the `mode-self-hosted` deployment shape (ADR-0013) in
a future `apps/server` chip; tracked as the remaining ADR-0008 follow-up.
The earlier `apps/web` placeholder was removed in the audit P1 sweep —
the canonical SaaS frontend will land alongside `apps/server` when that
chip ships.
