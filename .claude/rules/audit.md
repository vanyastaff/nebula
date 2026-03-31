# Deep Invariant Audit

When auditing a crate, find BROKEN INVARIANTS and EMERGENT BUGS — not style issues (clippy handles those).

## Three-Pass Process

### Pass 1: Build the Contract Map

For each public type and each `pub`/`pub(crate)` method, derive the contract from:
- The method name, doc comment, return type, parameter names
- Do NOT read the implementation yet

Output format:
```
TYPE::METHOD → "guarantees: ..."
```

### Pass 2: Verify Contracts Against Implementation

For each contract from Pass 1, read the implementation and check:

**(a) Extreme Values**
- Numeric fields at `u32::MAX` / `usize::MAX` / `0`?
- `f64` producing infinity, NaN, or overflowing `Duration`?
- Config field "should never be 0" — but what if it is?
- Is the cap applied BEFORE or AFTER the dangerous operation?

**(b) Partial Execution**
- If this async fn is dropped at every `.await` point, what state is left?
- Which counters/flags/permits are incremented but not decremented?
- Are RAII guards defused correctly in ALL paths (Ok, Err, drop)?

**(c) Cross-Method Consistency**
- Does every method that READS shared state apply the same cleanup/normalization as the WRITE methods?
- If method A updates field X and method B reads field X, do they agree on what "current" means?

Output per contract:
```
CONTRACT: "..."
HOLDS: yes / NO
FINDING: [precise description if NO]
```

### Pass 3: Cross-File Pattern Audit

After reading all files, scan for INCONSISTENCIES:

**(a) Attribute consistency**
Pick 3 attributes/derives on most public types. List every type MISSING them.

**(b) Error handling consistency**
List all constructors: which validate? which silently clamp? which panic?

**(c) Observable vs actual state**
For every "stats"/"metrics" method: does it reflect CURRENT state or state at last operation?
Are time-dependent values recomputed at observation time?

**(d) Duplicate logic**
Logic appearing 2+ times with slight differences — the differences are almost always bugs.

## Output Format

For each finding:

```
**[SEVERITY]** `path::to::item`
Category: Broken invariant / Extreme value / Partial execution / Inconsistency
Invariant: what SHOULD be true
Violation: what IS true instead
Trigger: minimum conditions to hit (concrete values, not "theoretically")
Fix: one concrete suggestion
```

Severities:
- **CRITICAL** — reachable panic or silent wrong behavior in production
- **HIGH** — wrong behavior under realistic load or config
- **MEDIUM** — observable inconsistency or API hazard
- **LOW** — latent risk requiring unusual conditions

## What NOT to Report

- Style issues (clippy handles these)
- Missing tests
- Things that "could" be better but aren't wrong
- Performance suggestions without broken invariants

Report ONLY broken contracts and inconsistencies.

## Specific Patterns to Watch (Nebula)

From past audits — these are the patterns that actually had bugs:

- **RAII guard + `mem::forget`** — always use `defused: bool` flag instead
- **`Duration::from_secs_f64`** on uncapped `f64` — can panic on infinity/overflow
- **`count_X_as_Y = false`** skipping total counters — probe slots leak in state machines
- **Seeded RNG recreated per call** — produces identical output, defeats purpose
- **Pipeline retry not propagating config fields** — new fields on config silently dropped
- **`current_rate()` / `stats()` not recomputing time-dependent values** — reports stale data
- **`with_burst()` setting only one of two related fields** — capacity and burst drift apart
- **Fallback error erasure producing wrong variant** — `Cancelled` vs `FallbackFailed`
- **`HashMap` for tiny key spaces** — `Vec` with linear scan is faster for ≤10 entries
