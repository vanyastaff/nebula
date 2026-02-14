---
title: How to: TransactionalAction (Saga)
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: TransactionalAction (Saga)

**Purpose.** Split work into `try/confirm/compensate` phases for distributed transactions.

## Trait Signature
```rust
#[async_trait]
pub trait TransactionalAction: Action {
    type Input: DeserializeOwned + Send + Sync;
    type Output: Serialize + Send + Sync;
    async fn try_phase(&self, input: Self::Input, ctx: &ExecutionContext) -> Result<ActionResult<Self::Output>, ActionError>;
    async fn confirm(&self, ctx: &ExecutionContext) -> Result<(), ActionError>;
    async fn compensate(&self, ctx: &ExecutionContext) -> Result<(), ActionError>;
}
```

## Implementation Steps

1. `try_phase` reserves resources (idempotent).
2. If downstream succeeds, call `confirm`; on failure/timeouts, call `compensate`.
3. Persist a saga log with correlation IDs for recovery.


## Minimal Example
```rust
pub struct PaymentSaga;
#[derive(Deserialize)] pub struct PayIn { pub order_id: String, pub amount: u64 }
#[derive(Serialize)] pub struct PayOut { pub auth_id: String }

#[async_trait]
impl TransactionalAction for PaymentSaga {
    type Input = PayIn; type Output = PayOut;
    async fn try_phase(&self, input: PayIn, _ctx: &ExecutionContext) -> Result<ActionResult<PayOut>, ActionError> {
        // reserve amount, get auth_id
        Ok(ActionResult::Success(PayOut { auth_id: "auth-xyz".into() }))
    }
    async fn confirm(&self, _ctx: &ExecutionContext) -> Result<(), ActionError> { Ok(()) }
    async fn compensate(&self, _ctx: &ExecutionContext) -> Result<(), ActionError> { Ok(()) }
}
```
