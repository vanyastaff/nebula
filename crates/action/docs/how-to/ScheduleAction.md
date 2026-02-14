---
title: "How to: ScheduleAction"
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: ScheduleAction

**Purpose.** Time-based planning (cron/rrule) to trigger actions at specific times.

## Trait Signature
```rust
#[async_trait]
pub trait ScheduleAction: Action {
    type Spec: DeserializeOwned + Send + Sync;
    async fn plan(&self, spec: Self::Spec, ctx: &ExecutionContext) -> Result<ActionResult<()>, ActionError>;
    fn due(&self, now: DateTime<Utc>) -> bool;
    fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>>;
}
```

## Implementation Steps

1. Parse cron/rrule into a durable `Spec`.
2. Use `next_after` to compute future triggers accurately (timezone aware).
3. Idempotency: key schedules by `(spec_hash, next_time)`.


## Minimal Example
```rust
pub struct CronScheduler;
#[derive(Deserialize)] pub struct Spec { cron: String }

#[async_trait]
impl ScheduleAction for CronScheduler {
    type Spec = Spec;
    async fn plan(&self, _spec: Spec, _ctx: &ExecutionContext) -> Result<ActionResult<()>, ActionError> { Ok(ActionResult::Done) }
    fn due(&self, _now: chrono::DateTime<chrono::Utc>) -> bool { false }
    fn next_after(&self, _after: chrono::DateTime<chrono::Utc>) -> Option<chrono::DateTime<chrono::Utc>> { None }
}
```
