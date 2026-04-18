---
name: Nebula competitive positioning
description: Position vs n8n / Temporal / Windmill / Make / Zapier and our bets against each. Extracted from PRODUCT_CANON.md §2 + §2.5 in Pass 1. Explicitly persuasive, not normative.
status: draft — extracted from canon, surgery in Pass 2
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md]
---

# Nebula competitive positioning

> **Status:** extracted verbatim from `PRODUCT_CANON.md`. This file is
> **persuasive** content — positioning and bets. Normative rules live in
> `PRODUCT_CANON.md`. If this file contradicts the canon, canon wins; open an
> issue to update this file.

---

## Position

**What Nebula is:** A **Rust-native workflow automation engine**: DAG workflows, typed boundaries, durable execution state, explicit runtime orchestration, first-class credentials/resources/actions — not a thin script runner. With room to grow from practical DAG workflows into richer execution models later.

**Peers by problem space (not a single category):** **n8n**, **Zapier**, **Make**, **Temporal**, **Windmill** — each solves a slice of automation/orchestration; Nebula is closest to **self-hosted workflow engines + durable execution**, not to SaaS iPaaS.

**Nebula's bet against all of them:**

- **Runtime honesty** over feature breadth.
- **Typed authoring contracts** over scriptable glue with opt-in validation.
- **Local-first** (single process / minimal deps) over "managed infrastructure minimum" (e.g. compose-only local path).

**Who it is for (primary):** Developers who **write integrations and nodes** — first-party core, community nodes, or internal nodes for a deployment. They need ergonomics, correct boundaries under failure, and confidence that the runtime handles throughput and resilience so they focus on integration logic.

**Who it is for (secondary):** **Operators** who deploy Nebula and compose workflows from existing nodes — they need clarity on durability, recovery, isolation, and observability.

**Pain we solve:** Many workflow tools treat integrations as **second-class** (opaque SDK, leaky abstractions) and assume the **happy path** (short runs, reliable networks). Nebula bets on **explicit state, clear layering, and operational honesty** (resumability, cancellation, leases, journals) without requiring a zoo of external services for the default local path.

**Competitive dimension (do not dilute):** Reliability and clarity of execution **as a system**, plus **DX for integration authors** — not feature parity with n8n/Make on day one, and **not** a surface-area race in v1.

**Success in one sentence:** *You can explain what happened in a run, recover or cancel safely, and trust the boundaries — not because marketing says so, but because the model matches operational reality.*

## Competitive bets

We have studied the leading tools. Each has a real insight. Each has a real ceiling. Nebula makes explicit bets about where those ceilings are.

**n8n**

- **Insight:** Visual graph + self-hosted + large node library is a real product.
- **Ceiling:** JS runtime means no compile-time contracts; node quality is inconsistent; engine-level durability is limited (restart often implies re-run from scratch for many flows); concurrency does not scale to very high throughput without pain.
- **Our bet:** Typed Rust integration contracts + honest durability beat a large but soft ecosystem; a **smaller library of reliable nodes** wins over time.

**Temporal**

- **Insight:** Durable execution as a first-class primitive is the right model; replaying workflows from history is powerful.
- **Ceiling:** Operational complexity is real (worker fleet, persistence cluster, replay constraints bleed into authoring); DX is heavy outside large teams; local path often means **Docker Compose or equivalent**, not "clone and run."
- **Our bet:** **Checkpoint-based recovery** with explicit persisted state is operationally simpler and equally honest for the use cases we target; **local-first must mean a single binary / minimal deps**, not a compose file as the default dev path.

**Windmill**

- **Insight:** Self-hosted + scriptable + visual composition works for developers; multi-language (Python/TS) lowers the authoring bar.
- **Ceiling:** Scripts-as-workflows is a thin model; deep resilience primitives are not the center; type safety is often **advisory** (e.g. TS types are not runtime contracts).
- **Our bet:** **Rust-native typed boundaries** + engine-owned retry/recovery beat scriptable glue with optional validation.

**Make / Zapier**

- **Insight:** Integration breadth and low-friction onboarding moves non-developers.
- **Ceiling:** Not a developer-first self-hosted product; limited operational insight for authors; pricing/hosting model is SaaS-centric.
- **Our bet:** **Not competing here** — different primary user and deployment model.

**What we borrow (intellectual honesty)**

- From **n8n:** the **visual graph** as the primary artifact; **open plugin ecosystem** shape.
- From **Temporal:** **durable execution as a contract**, not a convention in docs alone.
- From **Windmill:** **local-first, single-deployment simplicity** as a goal worth defending in product.
