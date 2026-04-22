---
name: Rust Expert Style Guide (LLM)
description: Deep Rust 1.95+ / Edition 2024 behavioral contract; files 01–09 under docs/guidelines/. Nebula canon and STYLE override on conflict.
status: reference
last-reviewed: 2026-04-21
related: [STYLE.md, AGENT_PROTOCOL.md, QUALITY_GATES.md]
---

# Rust Expert Style Guide

**Target reader:** an LLM generating Rust code — use as a **behavioral contract**, not as prose to quote.

**Nebula authority:** **`docs/PRODUCT_CANON.md`**, **`docs/STYLE.md`**, **`docs/GLOSSARY.md`**, and **`deny.toml`** override this guide when they conflict. This guide does **not** define layers, credentials, or product invariants.

**Where the full guide lives:** **`docs/guidelines/`** — numbered files **`01`–`09`** + **`README.md`** (rule IDs, operational semantics, navigation).

→ Start at **[`guidelines/README.md`](guidelines/README.md)**, then **[`01-meta-principles.md`](guidelines/01-meta-principles.md)**. The largest reference block is **`02-language-rules.md`** (`L-` rules).

**Optional digests:** **[`guidelines/research/README.md`](guidelines/research/README.md)** — supplementary clustered notes (async/Tokio, Nomicon, ecosystem, …), tag-aligned with the main files.
