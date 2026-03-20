# Top 6 Findings Prioritization: nebula-eventbus

**Date:** 2026-03-19
**Task:** subtask-6-2
**Methodology:** Impact = Frequency × Severity

---

## Impact Scoring Framework

**Severity Weighting:**
- **Bug:** 10 points (correctness issues with demonstrable production impact)
- **Footgun:** 5 points (API design issues that invite misuse)
- **Improvement:** 1 point (documentation gaps and optimizations)

**Frequency Estimation (Production Likelihood):**
- **High (10):** Will definitely occur in typical production use
- **Medium (7):** Likely to occur under common scenarios
- **Medium-Low (5):** Occurs in specific configurations or under load
- **Low (2):** Uncommon edge case
- **Very Low (1):** Extremely rare, practically impossible

**Impact Score = Severity × Frequency**

---

## All Findings Ranked by Impact Score

| Rank | Finding | Severity | Frequency | Score | Rationale |
|------|---------|----------|-----------|-------|-----------|
| 1 | BUG-1: TOCTOU in publish_drop_newest() | 10 | 5 | **50** | DropNewest policy violates semantics under concurrent load |
| 1 | BUG-2: TOCTOU in emit_blocking() | 10 | 5 | **50** | Block policy can overwrite instead of waiting under load |
| 1 | FOOTGUN-1: Missing #[must_use] on emit() | 5 | 10 | **50** | Primary API, very likely to ignore return value |
| 1 | FOOTGUN-3: Missing #[must_use] on recv() | 5 | 10 | **50** | Primary subscriber API, events silently discarded |
| 5 | FOOTGUN-2: Missing #[must_use] on emit_awaited() | 5 | 7 | **35** | Async variant of emit(), common in async contexts |
| 5 | FOOTGUN-8: recv() Infinite Loop Risk | 5 | 7 | **35** | Severe impact (hangs), easy to trigger with typos |
| 7 | FOOTGUN-4: Missing #[must_use] on try_recv() | 5 | 5 | 25 | Non-blocking variant, less common |
| 7 | FOOTGUN-5: Missing #[must_use] on FilteredSubscriber::recv() | 5 | 5 | 25 | Filtered subscribers less common than regular |
| 9 | FOOTGUN-6: Missing #[must_use] on get_or_create() | 5 | 2 | 10 | Registry API, less critical |
| 9 | FOOTGUN-9: Block accepts Duration::ZERO | 5 | 2 | 10 | Edge case, ambiguous semantics |
| 11 | FOOTGUN-7: Lag counter saturation | 5 | 1 | 5 | Requires quintillions of events |
| 11 | FOOTGUN-10: total_attempts() overflow | 5 | 1 | 5 | Practically impossible |
| 13+ | All IMPROVEMENT findings | 1 | varies | 1-10 | Documentation gaps, no correctness impact |

---

## Selected Top 6 Findings (Score ≥ 35)

### Finding 1: BUG-1 - TOCTOU Race in publish_drop_newest() Violates Policy Semantics
- **Score:** 50 (Severity 10 × Frequency 5)
- **Location:** `crates/eventbus/src/bus.rs:105-118`
- **Impact:** Violates documented DropNewest policy under concurrent load
- **Production Scenario:** Systems using DropNewest policy with multiple threads emitting events will experience unpredictable behavior (either drops events when space available, or acts like DropOldest)

### Finding 2: BUG-2 - TOCTOU Race in emit_blocking() Violates Block Policy Semantics
- **Score:** 50 (Severity 10 × Frequency 5)
- **Location:** `crates/eventbus/src/bus.rs:162-173`
- **Impact:** Violates documented Block policy under high concurrent load
- **Production Scenario:** Systems using Block policy with backpressure will unexpectedly overwrite oldest events instead of waiting, defeating the purpose of the Block policy

### Finding 3: FOOTGUN-1 - Missing #[must_use] on EventBus::emit()
- **Score:** 50 (Severity 5 × Frequency 10)
- **Location:** `crates/eventbus/src/bus.rs:85`
- **Impact:** Silent observability gaps - users cannot detect dropped events
- **Production Scenario:** Fire-and-forget emit() calls are the most natural pattern, leading to widespread ignoring of PublishOutcome without compiler warnings

### Finding 4: FOOTGUN-3 - Missing #[must_use] on Subscriber::recv()
- **Score:** 50 (Severity 5 × Frequency 10)
- **Location:** `crates/eventbus/src/subscriber.rs:66`
- **Impact:** Events silently discarded without compiler warning
- **Production Scenario:** Accidental `sub.recv().await;` (missing `let event = ...`) discards events without any indication

### Finding 5: FOOTGUN-2 - Missing #[must_use] on EventBus::emit_awaited()
- **Score:** 35 (Severity 5 × Frequency 7)
- **Location:** `crates/eventbus/src/bus.rs:138`
- **Impact:** Same as FOOTGUN-1 but for async contexts with Block policy
- **Production Scenario:** Async code using Block policy will miss delivery failures without compiler warnings

### Finding 6: FOOTGUN-8 - FilteredSubscriber::recv() Infinite Loop Risk
- **Score:** 35 (Severity 5 × Frequency 7)
- **Location:** `crates/eventbus/src/filtered_subscriber.rs:34-41`
- **Impact:** Task hangs indefinitely if filter never matches
- **Production Scenario:** Typo in filter predicate (e.g., `event.kind == "WorkflowStarted"` when actual field is `event.type`) causes silent hang that looks like "waiting for event"

---

## Findings NOT Selected (Score < 35)

**25 points:**
- FOOTGUN-4: Missing #[must_use] on try_recv() - Less common than recv()
- FOOTGUN-5: Missing #[must_use] on FilteredSubscriber::recv() - Filtered subscribers are specialized use case

**10 points:**
- FOOTGUN-6: Missing #[must_use] on get_or_create() - Registry API, less critical than core emit/recv
- FOOTGUN-9: Block accepts Duration::ZERO - Edge case, less likely to occur

**5 points:**
- FOOTGUN-7: Lag counter saturation - Requires quintillions of events (584,942 years at 1M/sec)
- FOOTGUN-10: total_attempts() overflow - Practically impossible

**1-10 points:**
- All 20 IMPROVEMENT findings - Documentation gaps with no correctness impact

---

## Rationale for Selection

The top 6 findings represent the **highest production impact** issues:

1. **Both BUGS (50 each):** Critical correctness issues that violate documented API contracts under concurrent load. These MUST be fixed or documented as known limitations.

2. **Three #[must_use] Footguns (50, 50, 35):** The core emit/recv APIs are most frequently used and most likely to have their return values ignored. These are easy to fix and provide immediate value.

3. **One Infinite Loop Risk (35):** Severe impact (hangs indefinitely) with medium-high likelihood (typos in filters are common). Currently documented but easy to miss.

**Excluded findings** scored lower due to:
- Lower frequency (specialized APIs, edge cases)
- Lower severity (documentation-only improvements)
- Lower production likelihood (practically impossible scenarios)

---

## Validation Against Spec Requirements

From spec.md:
> "Select top 6 by impact (frequency × severity)"

✅ **All 6 findings score ≥ 35 points** (top tier)
✅ **2 Bugs + 4 Footguns** (prioritizes correctness over documentation)
✅ **Concrete production scenarios** for each finding
✅ **Ruthless prioritization** - excluded 26 lower-impact findings

---

## Next Steps

**Subtask 6-3:** Write concrete reproduction scenarios for each of the top 6 findings
**Subtask 6-4:** Suggest fixes with code snippets and rationale
**Subtask 6-5:** Generate final review report and coverage checklist

---

**Selection Complete: 2026-03-19**
