# Architecture

## Problem Statement

- **Business problem:** Authors must implement Action, Resource, Plugin, Credential, and parameter definitions with minimal boilerplate and no drift from platform contracts.
- **Technical problem:** Proc-macros must expand to code that compiles and satisfies trait bounds; attribute set must be stable and diagnosable.

## Current Architecture

- **Module map:** action, resource, plugin, credential, parameter, validator, config (each derive); support, types (shared). No unsafe; proc_macro only.
- **Data/control flow:** TokenStream in → parse attributes and struct → emit impl and helpers → TokenStream out. All at compile time.
- **Known bottlenecks:** Contract tests (macro output + action/plugin/credential compile and run) to be formalized; diagnostic quality can improve.

## Target Architecture

- **Target module map:** Same; optional doc or tooling for expansion debugging (cargo expand).
- **Public contract boundaries:** Each derive and its attributes are the public API; generated code is the implicit contract with action/resource/plugin/credential.
- **Internal invariants:** No unsafe; no runtime behavior in macro crate; expansion is deterministic.

## Design Reasoning

- **Trade-off:** Single crate for all derives — one version, one compatibility matrix; but crate grows with each new derive.
- **Rejected:** Unsafe for "performance" in codegen — not needed; forbid(unsafe_code) keeps trust model simple.

## Comparative Analysis

Sources: serde (derive pattern), diesel (macro API stability).

- **Adopt:** Derive + attributes, stable attribute set, clear errors (serde/diesel style).
- **Reject:** Unversioned experimental attributes; macro output that does not implement the trait.
- **Defer:** Macro hygiene and advanced span handling beyond current syn/quote.

## Breaking Changes (if any)

- Attribute or output contract change: major; see MIGRATION.md.

## Open Questions

- Formal compatibility matrix (macro version X ↔ action/resource/plugin/credential Y) and release cadence.
