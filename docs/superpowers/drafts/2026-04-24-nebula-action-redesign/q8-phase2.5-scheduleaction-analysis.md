# Q8 Phase 2.5 — ScheduleAction Core vs Extensible analysis

**Phase:** 2.5 sub-investigation (NOT cascade revisit; user follow-up to Phase 2 §2.6 / F3 deferral framing).
**Author:** architect (analyst, not decider).
**Question (verbatim from user):** *«ScheduleAction не знаю тут надо подумать это как Core или люди могут использовать его и создавать свои? если да то какие?»*

**Translation:** Should `ScheduleAction` be **Core** (Nebula-team-defined-only — closed primitive set) or **user-extensible** (community can implement custom schedule kinds)? If extensible — what kinds would they create?

**Sources read line-by-line:**
- `docs/research/n8n-trigger-pain-points.md` (cron / schedule / timezone sections + correlation table)
- `q8-rust-senior-trigger-research.md` §1.3, §3.1, §6 finding #3 (ScheduleAction structural absence)
- `q8-architect-action-research.md` §3.1 (Schedule shape vs PollAction; Temporal Schedule comparison)
- `q8-phase2-synthesis.md` §2.6 (F3 deferral framing) + §6.3 (conditional escalation)
- `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` §2.6 (sealed-DX peer pattern, lines 563-720)
- `docs/adr/0038-controlaction-seal-canon-revision.md` §1, §2, §Negative item 4 (sealed enumeration constraint)
- `docs/COMPETITIVE.md` lines 17-31, 39-41 (Nebula positioning)

**Posture:** Architect frames trade-offs and recommends; does NOT decide cascade scope inclusion (that is user's call after this analysis lands).

---

## §1 ScheduleAction as Core (closed primitive set)

### §1.1 Concrete shape

If Core: `nebula-action` ships a fixed enum of schedule kinds; community implements `TriggerAction` directly for any unconventional temporal logic.

```rust
// Sealed-DX peer of TriggerAction (same shape as WebhookAction / PollAction per §2.6).
pub trait ScheduleAction: sealed_dx::ScheduleActionSealed + Action + Send + Sync + 'static {
    /// Engine-defined schedule kind. Closed enum — adding a variant requires
    /// nebula-action release + canon §3.5 amendment per ADR-0038 §2.
    fn schedule(&self) -> ScheduleKind;

    /// Event emitted on each fire (typed, not Value).
    type Event: Serialize + Send + Sync;

    fn on_fire<'a>(
        &'a self,
        scheduled_at: DateTime<Utc>,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::Event, ActionError>> + Send + 'a;
}

pub enum ScheduleKind {
    /// 5- or 6-field cron expression in IANA timezone.
    Cron { expression: String, tz: chrono_tz::Tz },
    /// Fixed interval from anchor (anchor defaults to engine startup).
    Interval { period: Duration, anchor: Option<DateTime<Utc>> },
    /// Single-shot at specific instant.
    OneShot { at: DateTime<Utc> },
    /// Cron expression but skip fires whose computed instant falls outside
    /// the named window (e.g., "9-17 Mon-Fri in America/New_York").
    /// Composes Cron + window predicate without exposing predicate as user code.
    CronWindowed { expression: String, tz: chrono_tz::Tz, window: TimeWindow },
}
```

**Engine consumes `ScheduleKind` directly.** Schedule-ledger, missed-fire replay, catch-up policy, leader-elected fire (all per `n8n-trigger-pain-points.md:354` n8n Quick Win 2) are engine-cluster-mode-cascade scope — engine has full visibility into the kind because the enum is closed.

### §1.2 Pros

- **Engine reasons exhaustively** about every schedule. Schedule-ledger schema has fixed columns; missed-fire replay has fixed catch-up policy per kind; leader-elected fire (per `n8n-trigger-pain-points.md:355` Quick Win 3) maps to the kind. No "unknown ScheduleKind" branch.
- **Cron-syntax-as-data** at the trait surface (per Q3 §2.9.1c schema-as-data axis precedent) — Core picks `cron` library version, picks IANA timezone library, picks edge-case semantics (DOM × DOW intersection per `#27238`). Author can't author a broken cron parser and ship as plugin.
- **Smaller surface to test.** 4 variants = 4 catch-up paths = bounded fuzz domain. Compare F1 (PollAction cursor honesty): 5+ axes of author-controlled variability.
- **Aligns with COMPETITIVE.md line 41 ("typed Rust integration contracts + honest durability").** Engine-owned schedule semantics are a runtime-honesty win — author can't accidentally ship a custom schedule that breaks Nebula's missed-fire-replay invariant.

### §1.3 Cons

- **Enum extension requires nebula-action release.** Want "fire on Tuesdays of even weeks"? Wait for next nebula-action minor. ADR-0038 §Negative item 4 explicitly names this trade-off for sealed primaries.
- **No path for genuinely novel schedule semantics.** A community plugin author who has a legitimate temporal pattern (calendar-driven, market-hours, holiday-aware — see §2.2) cannot ship without going through Nebula core process.
- **Author falls back to TriggerAction shape-2** (run-until-cancelled per §2.2.3) for any custom temporal logic — losing schedule-ledger semantics, losing catch-up, losing leader-elected fire. Defeats the structural-fix-class motivation per `n8n-trigger-pain-points.md:354-357`.
- **Falsely declares "this is the temporal universe"** when it's actually the most-common 4 kinds. Cron + Interval + OneShot covers ~80% of n8n use cases; the long tail (per §2.2 below) is real.

### §1.4 DX impact

- **Author DX (common case):** `#[action] pub struct DailyReport { ... } impl ScheduleAction for DailyReport { fn schedule(&self) -> ScheduleKind { ScheduleKind::Cron { expression: "0 9 * * 1-5".into(), tz: chrono_tz::US::Eastern } } ... }`. Five lines. Type-checked. **Excellent.**
- **Author DX (uncommon case):** Falls back to TriggerAction. Forfeits schedule-ledger / catch-up / leader-elected fire. `feedback_active_dev_mode.md` "never settle for deferred" — author SHIPS production integration without Nebula's missed-fire-replay protection. **Worst-case is bad.**

### §1.5 Implementation surface

**Core (action-cascade scope if pulled in):**
- 1 sealed-DX peer trait (`ScheduleAction`)
- 1 closed enum (`ScheduleKind`) + supporting types (`TimeWindow`)
- 1 adapter (`ScheduleTriggerAdapter` per Q7 R6 peer pattern)
- 1 sealed inner trait (`sealed_dx::ScheduleActionSealed`)
- `#[action]` attribute zone for schedule kind (likely)
- ~600-900 lines of crates/action/src/schedule.rs (analogous to webhook.rs 1330 lines, poll.rs 1467 lines)

**Engine-cluster-mode-cascade scope (deferred but constrained):**
- `ScheduleLedger` trait (engine-side schedule fire history)
- Missed-fire replay logic per kind
- Leader-elected schedule registration

---

## §2 ScheduleAction as user-extensible (open trait + community implementations)

### §2.1 Concrete shape

If extensible: `ScheduleAction` trait carries **the next-fire-instant computation** as a method; community implements custom schedules.

```rust
// NOT sealed — community plugins implement directly. Same shape as TriggerAction
// (open trait per §2.2.3) but carries schedule-specific lifecycle.
pub trait ScheduleAction: Action + Send + Sync + 'static {
    type Event: Serialize + Send + Sync;

    /// Compute next fire instant strictly after `from`. Return None to stop the
    /// schedule (one-shot completed, last calendar entry passed, etc.).
    /// Engine calls this on activation, after every fire, and on catch-up replay.
    /// MUST be deterministic given the same `from` argument.
    fn next_fire(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>>;

    /// Catch-up policy when missed fires are detected.
    fn catch_up(&self) -> CatchUpPolicy { CatchUpPolicy::FireAllInWindow { window: Duration::from_secs(3600) } }

    /// Event factory invoked at fire time.
    fn on_fire<'a>(
        &'a self,
        scheduled_at: DateTime<Utc>,
        ctx: &'a ActionContext<'a>,
    ) -> impl Future<Output = Result<Self::Event, ActionError>> + Send + 'a;

    /// Optional: declare if this schedule is durable (engine MUST persist next-fire
    /// to ledger so restart preserves state) or ephemeral (in-memory next-fire OK).
    fn durability(&self) -> ScheduleDurability { ScheduleDurability::Durable }
}

pub enum CatchUpPolicy {
    /// Fire all missed fires within window (chronologically).
    FireAllInWindow { window: Duration },
    /// Fire only the most recent missed fire (skip the rest).
    FireMostRecent,
    /// Skip all missed fires; resume at next future fire.
    SkipMissed,
}
```

Nebula core ships **blessed implementations** (`CronSchedule`, `IntervalSchedule`, `OneShotSchedule`) using this trait — same way `nebula-credential` ships `OAuth2Scheme` while keeping `Credential` open per credential Tech Spec. Community implements custom for the long tail.

### §2.2 Brainstormed custom schedule kinds (community use cases)

Concrete patterns the long tail would need. Each is a real shape the closed Core enum cannot capture cleanly.

| # | Custom kind | Use case | Why Core enum can't capture |
|---|-------------|----------|------------------------------|
| **1** | **MarketHoursSchedule** | Financial workflows: fire every 5 min during NYSE/LSE/TSE open; pause overnight + weekends; respect early-close days (half-day Friday after Thanksgiving) | Requires per-exchange calendar + half-day rules — too domain-specific for Core; market data subscription required to know "early close today" |
| **2** | **BusinessDaySchedule** | Daily business reports: fire every weekday 09:00 in office tz; skip weekends + organization-specific PTO calendar (e.g., company observes Martin Luther King Jr Day but not Columbus Day) | Org-specific weekend / observance calendar; closed enum cannot enumerate every org policy |
| **3** | **HolidaySchedule** (region-specific) | Compliance reports: fire monthly on first non-holiday business day in a country (US/CA/UK/DE/JP/AU all differ); requires up-to-date holiday calendar (Easter shifts, lunar calendars) | Calendar-as-data lives outside Nebula; community wraps `holidays-rs` or `chrono-business` |
| **4** | **SeasonalSchedule** | Energy/utility workflows: fire every 15 min in summer (peak demand window); every hour in winter; daylight-savings-aware | Cron with season transitions doesn't compose; fire frequency itself is variable |
| **5** | **AdaptiveSchedule** (load-driven) | Backfill jobs: fire as fast as queue depth permits, but not faster than max rate; effective interval = `max(min_period, queue_depth / target_throughput)` | Requires queue-depth observation each cycle — can't be expressed as static cron syntax |
| **6** | **ICalSubscriptionSchedule** | Workflows driven by external iCal feed (Google Calendar, Outlook, university timetable) — fire when calendar event starts | iCal RRULE has 200+ variations; subscription URL is config-data; fetched per-cycle |
| **7** | **AstronomicalSchedule** | Solar/lunar workflows: fire at sunrise / sunset / civil-twilight / lunar-phase-change in a given lat/lon (agricultural irrigation; smart-home lighting) | Geolocation + ephemeris computation; depends on `astro-rs` or NOAA tables |
| **8** | **TideSchedule** | Coastal logistics: fire at high-tide / low-tide for a given tide station | NOAA tide-station data subscription |
| **9** | **CalendarVersioned**: **AcademicYearSchedule** | University workflows: fire every Monday during fall + spring semesters; pause during winter break / summer (calendar varies per institution) | Per-institution calendar — config data, not enumerable |
| **10** | **FiscalCalendarSchedule** | Accounting workflows: fire on last business day of fiscal month (4-4-5 calendar; 13-period calendar — different per company) | Multiple fiscal-calendar models exist; not enumerable |
| **11** | **CronWithExclusions** | "Every 15 min, but skip 02:00-03:00 daily for maintenance window" | Cron extension; exclusion windows are deployment-specific |
| **12** | **JitteredSchedule** | Distributed-fleet workflows: fire every 5 min ± 30s jitter to spread load (anti-thundering-herd) | Non-deterministic next_fire — actually CONFLICTS with "deterministic next_fire" trait contract; would need a distinct hook (so even open trait doesn't trivially capture this — see §3.4) |
| **13** | **ProbabilisticSchedule** | Sampled monitoring: fire on 1% of would-be cron fires to keep telemetry volume low | Same conflict as #12 — non-deterministic fire decision |
| **14** | **EventDrivenSchedule** (composite trigger) | "Fire when calendar event ends, but only on business days" — combines #2 + external signal | Composes other custom schedules |
| **15** | **TimeOfDayInUserTimezone** | Per-user notification schedules: each user has different tz; fire 09:00 in EACH user's timezone | Schedule kind itself is parametric over user-data; Core cron-with-tz takes single tz |

**Count: 15 distinct community-plausible schedule kinds.** Of these:
- **#1, #2, #3, #6, #11, #14, #15 are deterministic computations** that fit the trait shape cleanly.
- **#4, #5, #9, #10 are deterministic but require external data** (calendars, queue depth) — fit shape, ctx provides resources.
- **#7, #8 require domain-specific math libraries** — fit shape, depend on external crates.
- **#12, #13 are non-deterministic** — actually expose a NEW design question (deterministic vs probabilistic schedules — see §3.4).

Realistic community pull rate: **5-10 of these 15 would be implemented in the first year by enterprise / vertical-domain plugin authors.** Closed Core enum cannot capture any of them.

### §2.3 Pros

- **Long tail addressed.** §2.2 demonstrates 15 plausible kinds the Core enum cannot capture; community ships them as plugins instead of waiting for Nebula releases.
- **Aligns with TriggerAction's open posture.** Per Q7 R6: WebhookAction / PollAction are PEERS of TriggerAction (not subtraits) and TriggerAction itself is OPEN. ScheduleAction following the same open-peer pattern is **internally consistent**.
- **`nebula-credential` precedent.** Credential trait is open; Nebula ships blessed schemes (OAuth2 / ApiKey / Basic / Bearer); community implements custom schemes for niche providers. Same shape = same DX intuition.
- **No long-tail-blocked-on-Nebula-release pressure.** Plugin authors don't file "please add ScheduleKind::FiscalCalendar" issues against nebula-action.

### §2.4 Cons

- **Engine cannot reason exhaustively.** Schedule-ledger has to handle "unknown ScheduleAction impl" branch — what does catch-up mean for an `AdaptiveSchedule` that depends on queue depth? Fire all missed = re-observe queue depth every fire? Engine has no way to know.
- **Misuse vector.** Community author writes `next_fire` that returns past instants in a loop → engine infinite-fires → workflow death-spiral. Mitigation: engine MUST validate `next_fire(from) > from`; reject impls that violate.
- **Test surface multiplies.** Engine cannot fuzz against fixed enum; must fuzz against trait-impl behavior. Property-test "next_fire must be strictly after from" catches the obvious; subtler bugs (DST gaps, leap-second edges, missing holiday data) are author-responsibility.
- **Sealed-DX-5-trait constraint (per ADR-0038 §Negative item 4).** Adding ScheduleAction as a sealed-DX peer adds a 6th sealed trait — but if it's NON-sealed (open), it's NOT a sealed-DX peer at all; it's a NEW PRIMARY trait, which **requires canon §3.5 revision per ADR-0038 §2** (canon enumerates the 4 primaries + 5 sealed DX). The "extensible ScheduleAction" path triggers canon revision.

### §2.5 DX impact

- **Author DX (common case):** Same as Core — write `impl ScheduleAction for DailyReport { fn next_fire(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> { CronSchedule::new("0 9 * * 1-5").next_fire(from) } ... }`. Slightly more boilerplate than Core's `ScheduleKind::Cron { ... }` but still type-checked, still under 10 lines.
- **Author DX (long-tail case):** `impl ScheduleAction for MarketHoursSchedule { fn next_fire(...) -> Option<DateTime<Utc>> { /* market-calendar lookup */ } ... }`. Possible. Type-checked. **Worst-case is GOOD** — author has structural surface to work with.
- **DX risk:** novice author writes infinite-fire bug. Engine MUST guard. Trait-level invariant testing required (see §2.4 cons).

### §2.6 Implementation surface

**Core (action-cascade scope if pulled in):**
- 1 OPEN trait `ScheduleAction` (NOT sealed)
- 1 enum `CatchUpPolicy` + 1 enum `ScheduleDurability`
- 3 blessed impls: `CronSchedule`, `IntervalSchedule`, `OneShotSchedule` (~200 lines each = 600 lines)
- 1 adapter `ScheduleTriggerAdapter` (~400 lines)
- ~1100-1400 lines crates/action/src/schedule.rs
- **Canon §3.5 revision per ADR-0038 §2** (5th primary trait — NOT a sealed-DX-sugar; this is the load-bearing distinction)

**Engine-cluster-mode-cascade scope:**
- `ScheduleLedger` trait
- Engine MUST validate `next_fire(from) > from` invariant
- Engine catch-up runs `next_fire` repeatedly to enumerate missed fires (cost = O(missed-fires))

---

## §3 Hybrid pattern analysis

Three hybrid forms worth analyzing:

### §3.1 Hybrid A — Sealed-DX `ScheduleAction` + community gets fallback to TriggerAction

**Shape:** ScheduleAction is sealed-DX (Core enum). Community needing custom schedule writes `TriggerAction` directly. **This is the §1 Core option already.** The "fallback" exists but is structurally inferior (no schedule-ledger / catch-up / leader-fire) — making this a single-option, not a hybrid. **Reject as distinct option.**

### §3.2 Hybrid B — Sealed-DX `ScheduleAction` + extensible `Schedule` runtime trait

**Shape:** Two-tier separation per architect-action research §3 hint:
```rust
// Core sealed-DX wrapper — community uses for blessed schedules:
pub trait ScheduleAction: sealed_dx::ScheduleActionSealed + Action {
    fn schedule(&self) -> Box<dyn Schedule>;  // Returns the runtime trait
    type Event: Serialize + Send + Sync;
    fn on_fire<'a>(...) -> impl Future<...> + 'a;
}

// Open runtime trait — community implements custom kinds:
pub trait Schedule: Send + Sync + 'static {
    fn next_fire(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>>;
    fn catch_up(&self) -> CatchUpPolicy { CatchUpPolicy::FireAllInWindow { window: Duration::from_secs(3600) } }
    fn durability(&self) -> ScheduleDurability { ScheduleDurability::Durable }
}

// Nebula ships:
pub struct CronSchedule { /* ... */ }
impl Schedule for CronSchedule { /* ... */ }

pub struct IntervalSchedule { /* ... */ }
impl Schedule for IntervalSchedule { /* ... */ }

// Community ships:
pub struct MarketHoursSchedule { /* ... */ }
impl Schedule for MarketHoursSchedule { /* ... */ }

// Community wires custom into action:
#[action(...)]
impl ScheduleAction for HourlyReportDuringMarketHours {
    fn schedule(&self) -> Box<dyn Schedule> {
        Box::new(MarketHoursSchedule::nyse())
    }
    type Event = ReportPayload;
    async fn on_fire(...) -> Result<Self::Event, ActionError> { ... }
}
```

**Pros:**
- ScheduleAction stays sealed-DX (no canon §3.5 revision per ADR-0038 §2 — matches Webhook/Poll precedent).
- Community gets custom schedule kinds via `Schedule` trait without needing canon revision.
- Engine reasons via `Box<dyn Schedule>` — fixed dispatch surface; trait methods are exhaustive.

**Cons:**
- `Box<dyn Schedule>` allocation per registration (negligible for schedules).
- Community `Schedule` impls live OUTSIDE the sealed-DX seal — author can ship a broken `Schedule` impl. Same misuse vector as §2.4 cons. Engine MUST validate `next_fire(from) > from`.
- Two distinct concepts (`ScheduleAction` for action-side; `Schedule` for kind-side) — slightly more cognitive load than single-trait options.
- **`Schedule` trait is itself a NEW primary** in some sense — ADR-0038 §2's canon revision rule is about the **action trait family** specifically; `Schedule` is a supporting trait, NOT an action-dispatch trait. So canon revision is NOT required. **This is the load-bearing structural advantage of Hybrid B over §2.**

**Pattern precedent in codebase:** This is the **exact** shape `nebula-resilience::Policy` takes — sealed-action-side hook calls open-trait-side strategy. Per memory `reference_resilience_usage.md`. Also matches credential `SchemeFactory<C>` per credential Tech Spec — `nebula-credential::Credential` is open; `Scheme` (the auth-protocol surface) is open AND parametric. Hybrid B is **internally consistent with two existing Nebula patterns**.

### §3.3 Hybrid C — Closed enum + escape hatch variant

**Shape:**
```rust
pub enum ScheduleKind {
    Cron { ... },
    Interval { ... },
    OneShot { ... },
    CronWindowed { ... },
    /// Escape hatch — community-defined schedule. Engine treats as opaque;
    /// catch-up policy degrades to FireMostRecent.
    Custom { compute_next: Box<dyn Fn(DateTime<Utc>) -> Option<DateTime<Utc>> + Send + Sync> },
}
```

**Reject:** boxed-closure schedules can't be persisted, can't be introspected for testing, can't carry type-safe configuration. Engine CANNOT meaningfully reason about `Custom`. Strictly worse than Hybrid B because the escape-hatch-as-closure is an anti-pattern (per memory `feedback_no_shims.md` — "never propose adapters/bridges/shims; replace the wrong thing directly").

### §3.4 Hybrid B subtlety — non-deterministic schedules (§2.2 #12, #13)

Hybrid B's `Schedule` trait declares `next_fire` as deterministic. Schedules #12 (jittered) and #13 (probabilistic) are non-deterministic. Either:
- Cast as **non-issue**: jitter happens at engine-side ("cron fires at HH:MM:00; engine adds ± jitter when enqueueing"); `Schedule::next_fire` returns the planned instant, engine adds jitter when actually firing. Same for probabilistic — Schedule returns the would-be instant; engine samples 1% of fires.
- Cast as **distinct trait**: `RandomSchedule` peer with `fn fire_decision(&self) -> bool`. Adds surface area.

**Architect reading:** non-determinism belongs at engine-fire-time, not in Schedule itself. Hybrid B handles #12/#13 via engine-side jitter / sampling without trait expansion. Same answer as PollAction's existing per-trigger jitter seed (`PollTriggerAdapter` per Tech Spec §2.6 line 707).

### §3.5 Comparison matrix

| Axis | §1 Core (closed enum) | §2 Open trait | Hybrid B (sealed-DX + Schedule trait) |
|------|----------------------|---------------|---------------------------------------|
| Long tail addressed | ❌ No | ✅ Yes | ✅ Yes (via `Schedule` impl) |
| Canon §3.5 revision required | ❌ No | ✅ Yes (5th primary) | ❌ No (sealed-DX peer pattern, ADR-0038 §2) |
| Engine reasoning | ✅ Exhaustive | ⚠️ Partial (trait-impl varies) | ✅ Exhaustive (via fixed `Schedule` trait surface) |
| Fits `nebula-action` `feedback_active_dev_mode` "honest worst-case" rule | ❌ Worst-case is fall back to TriggerAction (loses ledger) | ✅ Worst-case is "author wrote bad Schedule" (engine validates) | ✅ Worst-case is "author wrote bad Schedule" (engine validates, same as §2) |
| Internal pattern consistency | ⚠️ Diverges from credential's open shape | ⚠️ Forces canon revision unlike Webhook/Poll | ✅ Matches credential SchemeFactory + nebula-resilience Policy |
| Implementation surface (action-cascade) | ~600-900 LoC | ~1100-1400 LoC + canon revision | ~1000-1300 LoC, no canon revision |
| `#[action]` macro complexity | Lowest (struct → enum variant) | Medium (struct → trait impl) | Medium (struct → trait impl + Schedule wiring) |
| Misuse risk for community authors | Low (closed enum constrains) | Medium (open trait — bad next_fire crashes engine) | Medium (Schedule trait — same as §2 but bounded by engine validation) |

---

## §4 n8n architecture lessons

### §4.1 What n8n does

Per `n8n-trigger-pain-points.md:106-107, 190-201`:
- Single ScheduleTrigger node uses `cron` library + `moment-timezone`.
- Workflow-level timezone with server-tz fallback.
- Schedule UI offers presets (every N min/hour/day, weekday cron, custom cron expression) — i.e., **a fixed UI vocabulary** mapped to cron under the hood.
- **No extension point for custom schedule kinds.** Authors who need market-hours / business-day / etc. wrap a Schedule node with downstream If/Filter nodes (e.g., "fire every hour, then filter to business hours").

### §4.2 Pain points cited (n8n research §1.3 and §3 trigger-coverage research)

- `#27103` cron-randomization-on-save creates duplicate registrations (engine-side bug — closed cron parser still has bugs)
- `#23906` missed schedule fires lost during downtime (no replay)
- `#25057` "active 2 weeks but not running" (silent stop)
- `#27238` cron with intersected DOM + DOW runs every day (cron parser semantics differ from POSIX)
- `#23943` "Hours Between Triggers" interval mode fails (custom kind that n8n DID add — buggy)
- `#24272`/`#24271` "every 2 hours" doesn't trigger (interval edge case)

**Pattern:** **Even closed Core (n8n's choice) has bugs in the parser, the timezone library, and the catch-up-or-lack-thereof.** The closed shape doesn't prevent bugs; it concentrates them in one place that everyone hits.

### §4.3 Workaround pattern when missing primitive

`n8n-trigger-pain-points.md:140-141`: «Workaround pattern когда trigger отсутствует: Schedule → HTTP poll с `getWorkflowStaticData` cursor.» In other words, **users abuse Schedule + Filter** to express anything not in n8n's preset vocabulary. This is exactly what would happen in Nebula §1 Core: author who needs business-day fires writes `ScheduleKind::Cron { expression: "0 9 * * 1-5", tz: ... }` + downstream Filter node checking `is_business_day(ctx.now())`.

**Result:** Worse than Hybrid B because:
- The filter runs every fire (not just suppressed at fire-decision time)
- Engine has no visibility ("why did 12 of 20 monthly fires execute zero workflow steps?" — answer is in user-author Filter logic, not introspectable)
- Schedule-ledger records every fire as "fired" even when downstream filter dropped it — observability lies

### §4.4 What n8n correlation table prescribes

Per `n8n-trigger-pain-points.md:354`: «Schedule history table `(workflow_id, fire_at, fired)`; на boot replay missed с `since_last_boot` config cap.»

**This works for ANY kind of schedule** — it's a per-workflow ledger column, not a per-kind ledger. Both §1 Core and Hybrid B can implement this. **Engine-side ledger is orthogonal to trait shape choice.**

### §4.5 Lesson summary

1. **Closed schedule vocabulary forces workarounds for the long tail** (n8n authors' fallback to Schedule + Filter is observability-blind).
2. **Engine-side ledger is necessary regardless of trait shape** — closed enum doesn't prevent missed-fire class (engine implementation does).
3. **Cron parser correctness is HARD** — even n8n's vendor-blessed `cron` library has open bugs (`#27238`); shipping a single Nebula-blessed cron parser as `CronSchedule` (in Hybrid B) concentrates the bug surface in one place that gets fixed once. **Better than Core enum concentration but no worse.**
4. **Plugin author frustration grows in proportion to "we won't add your kind, file a feature request"** — n8n's `#23943` "Hours Between Triggers" was a community-pressure-driven addition that arrived with bugs. Open trait + community-shipped kinds isolate the failure modes.

---

## §5 Recommendation + rationale

### §5.1 Pick: **Hybrid B** (sealed-DX `ScheduleAction` peer + open `Schedule` runtime trait)

Concretely:
- `ScheduleAction` joins WebhookAction / PollAction as a **sealed-DX peer of TriggerAction** (no canon §3.5 revision required per ADR-0038 §2 — matches the existing Webhook/Poll seal-of-DX-but-open-supporting-types pattern).
- `Schedule` runtime trait is **OPEN** — community plugins implement custom schedule kinds (`MarketHoursSchedule`, `BusinessDaySchedule`, `HolidaySchedule`, etc. per §2.2's 15 catalogued use cases).
- Nebula core ships **3 blessed `Schedule` implementations** (`CronSchedule`, `IntervalSchedule`, `OneShotSchedule`) — 80% case is single-line. `CronWindowed` deferred to community impl (composes Cron + window predicate).
- Engine validates `next_fire(from) > from` invariant at registration AND on every fire (defense-in-depth against author bugs).
- Schedule-ledger / missed-fire replay / leader-elected fire are engine-cluster-mode-cascade scope — same deferral as F1 / F4 cluster-mode hooks per Phase 2 §2.3.

### §5.2 Rationale (decisive citations)

1. **Aligns with COMPETITIVE.md line 17-31 / line 41 ("typed Rust integration contracts + honest durability + open plugin ecosystem shape").** Open `Schedule` trait + sealed-DX action wrapper = typed contract for the action-author + open ecosystem for kind-authors. Both wins simultaneously.
2. **Internal pattern consistency.** Matches `nebula-credential::Credential` (open trait + blessed schemes) AND `nebula-resilience::Policy` (sealed action-side hook + open strategy trait). User picking Hybrid B reinforces a pattern that already appears twice in Nebula. Per architect Phase 1 §6.4 — "no new axis from research that would unblock §2.9 consolidation" — Hybrid B doesn't introduce a new pattern, it reuses the credential pattern.
3. **Plugin-author DX wins for the 80% case AND the long-tail case.** §1 Core option fails the long tail; §2 fully-open option requires canon revision. Hybrid B is the only option with no compromise on either axis.
4. **§2.2 brainstorm enumerated 15 plausible community schedule kinds** — this is NOT a hypothetical long tail; market-hours / business-day / fiscal-calendar / iCal subscription are real enterprise / vertical-domain needs that already exist in n8n forum threads (per `n8n-trigger-pain-points.md` workaround pattern).
5. **Future-proofing per `feedback_active_dev_mode.md` ("never settle for cosmetic / quick win / deferred").** Closed Core is a "quick win" — picks 4 kinds, ships, declares done. Hybrid B is the more-ideal path: picks the open shape now, blessed kinds for ergonomics, no future-cascade pressure to "add ScheduleKind variant N+1."
6. **No canon revision required per ADR-0038 §2** — sealed-DX peer + open supporting trait is the established Webhook/Poll precedent. Strategy §3.4 line 164 + Tech Spec §1.2 N4 deferral framework absorbs the cluster-mode pieces (ledger, leader-fire) without needing to revise canon.

### §5.3 Honest trade-off

Hybrid B is **slightly more complex** for action authors than §1 Core in the **simplest possible case**:
- §1 Core: `ScheduleKind::Cron { expression: "0 9 * * 1-5".into(), tz: chrono_tz::US::Eastern }` (one expression).
- Hybrid B: `Box::new(CronSchedule::new("0 9 * * 1-5", chrono_tz::US::Eastern))` (one expression + boxed-trait wrapping).

This is 1 line vs 1 line; both type-checked. The complexity delta is **negligible at the call-site** but visible at the trait-shape level (author types `Box<dyn Schedule>` instead of `ScheduleKind` enum). **Acceptable** — the win in long-tail address + ecosystem health far outweighs the boxed-trait-overhead cosmetic.

### §5.4 What this does NOT decide

Per Q8 Phase 2.5 constraints:
- Does NOT decide if user pulls F3 / ScheduleAction into current cascade scope (vs deferring per Phase 2 §3.2 D2). User picks after this analysis.
- Does NOT decide if cluster-mode-cascade scope should expand to absorb schedule-ledger work in this cascade or the next.
- Does NOT decide if `CronWindowed` belongs in Nebula core or community (proposed: community).
- Does NOT enact Tech Spec amendment.

**Architect framing:** Hybrid B is the structurally honest answer to user's question "Core or extensible?"  Answer: **NEITHER pure-Core nor pure-extensible — sealed-DX action wrapper + open Schedule kind trait, matching existing Nebula precedents.** Tech-lead picks scope inclusion separately.

---

## §6 Open items raised this analysis

- **§5.1** — User decides whether to pull ScheduleAction into current cascade scope (would require: new sealed-DX peer; new open `Schedule` trait; 3 blessed Schedule impls; Schedule adapter; ~1000-1300 LoC + spike iter-3 because new sealed-DX peer interacts with Q7 R6 peer pattern). Phase 2 §6.3 default is DEFER.
- **§3.4** — Non-deterministic schedules (#12 jittered, #13 probabilistic): proposed handled at engine-fire-time per §3.4 reading; needs tech-lead ratification at cascade-design time if pulled in.
- **§5.3** — `chrono_tz` crate dependency would be added to nebula-action (currently not a dependency per audit). Devops should weigh in on dependency-graph impact if scope pulled in.
- **§4.5** — Confirms engine-side schedule ledger is necessary REGARDLESS of trait shape choice; this aligns with Phase 2 §3.2 D2 (deferred-to-cluster-mode-cascade) but means trait-shape decision and ledger-shape decision are separable.

---

## §7 Sources

| Source | Used for |
|--------|----------|
| `docs/research/n8n-trigger-pain-points.md:40-46, 106-107, 117-127, 140-141, 190-201, 268-275, 354-355, 420-424` | n8n architecture; closed-Core failure modes; pain class severity |
| `q8-rust-senior-trigger-research.md` §1.3 / §3.1 / §6 finding #3 (lines 64-72, 158-196, 346-359) | Sealed-DX peer pattern; ScheduleAction structural absence; Tech Spec §2.6 implications |
| `q8-architect-action-research.md` §3.1 schedule row + §3.2 third-candidate-primary | Temporal Schedule comparison; "5th primary" canon-revision constraint |
| `q8-phase2-synthesis.md` §1 F3 / §2.6 / §6.3 | F3 deferral framing; conditional escalation if pulled in |
| `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` §2.6 (lines 563-720) | Sealed-DX peer pattern; Webhook/Poll precedent |
| `docs/adr/0038-controlaction-seal-canon-revision.md` §1 / §2 / §Negative item 4 | Sealed-DX 5-trait constraint; canon §3.5 revision rule |
| `docs/COMPETITIVE.md` lines 17-31, 39-41 | Nebula positioning ("typed contracts + honest durability + open plugin ecosystem") |
| `nebula-credential::Credential` + `SchemeFactory<C>` (per memory `reference_credential_tech_spec_pins.md`) | Hybrid B precedent — open trait + blessed implementations |
| `nebula-resilience::Policy` (per memory `reference_resilience_usage.md`) | Hybrid B precedent — sealed action-side hook + open strategy trait |
| `feedback_active_dev_mode.md` | "Never settle for cosmetic / quick win / deferred" — applied to §5.2 rationale 5 |
| `feedback_no_shims.md` | Hybrid C escape-hatch rejection per §3.3 |
