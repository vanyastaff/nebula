# DX Types (Convenience Wrappers)

All DX types blanket-impl to one of 4 core types. Engine never sees DX types.

---

## SimpleAction (→ StatelessAction)

Returns `Result<Output>`, auto-wraps in `ActionResult::Success`.

```rust
pub trait SimpleAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<Self::Output, ActionError>> + Send;
}

// Blanket: SimpleAction → StatelessAction
impl<T: SimpleAction> StatelessAction for T {
    type Input = T::Input;
    type Output = T::Output;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<ActionResult<Self::Output>, ActionError>
    {
        let output = SimpleAction::execute(self, input, ctx).await?;
        Ok(ActionResult::success(output))
    }
}
```

**Note:** SimpleAction cannot return Wait, Branch, Skip, or other flow control.
For flow control, use StatelessAction directly.
For streaming output, use StatelessAction with `ActionOutput::Streaming`.

---

## TransformAction (→ SimpleAction → StatelessAction)

Pure synchronous data transformation. No async, no context, no side effects.
Testable with plain `#[test]`, no tokio required.

```rust
pub trait TransformAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    fn transform(&self, input: Self::Input) -> Result<Self::Output, ActionError>;
}

impl<T: TransformAction> SimpleAction for T {
    type Input = T::Input;
    type Output = T::Output;

    async fn execute(&self, input: Self::Input, _ctx: &ActionContext)
        -> Result<Self::Output, ActionError>
    {
        self.transform(input)
    }
}
```

**Use cases:** field mapping, JSON reshape, filtering, formatting, math.

---

## PaginatedAction (→ StatefulAction)

Declarative pagination. Author writes only the business logic of one page fetch.
Framework manages cursor state, Continue/Break, progress reporting.

```rust
pub trait PaginatedAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Item: Serialize + Send + Sync + 'static;
    type Cursor: Serialize + DeserializeOwned + Default + Send + Sync + 'static;

    /// Fetch one page. Framework handles cursor persistence and iteration.
    fn fetch_page(
        &self,
        input: &Self::Input,
        cursor: &mut Self::Cursor,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<PageResult<Self::Item>, ActionError>> + Send;
}

pub struct PageResult<T> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub delay: Option<Duration>,
}

// Blanket: PaginatedAction → StatefulAction
// State = PaginationState<Cursor> { cursor, total_items }
// execute:
//   fetch_page(input, &mut state.cursor, ctx)
//   if has_more → Continue { output: items, delay }
//   else → Break { output: items, reason: Completed }
```

**Use cases:** API list endpoints, CRM data export, search result iteration.

---

## BatchAction (→ StatefulAction)

Process array of items in configurable chunks. Framework manages iteration,
partial output per chunk, and final aggregation.

```rust
pub trait BatchAction: Action {
    type Item: DeserializeOwned + Send + Sync + 'static;
    type OutputItem: Serialize + Send + Sync + 'static;

    fn batch_size(&self) -> usize { 100 }

    /// Optional rate limiting.
    fn rate_limit(&self) -> Option<RateLimit> { None }

    fn process_batch(
        &self,
        items: Vec<Self::Item>,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<Vec<Self::OutputItem>, ActionError>> + Send;
}

pub struct RateLimit {
    pub max_per_second: u32,
}

// Blanket: BatchAction → StatefulAction
// State = BatchState { index, all_items }
// execute:
//   chunk = all_items[index..index+batch_size]
//   rate_limit delay if configured
//   process_batch(chunk, ctx)
//   if more_chunks → Continue { output: chunk_results }
//   else → Break { output: all_results, reason: Completed }
```

**Use cases:** spreadsheet row processing, CRM bulk sync, email list operations.

---

## InteractiveAction (→ StatefulAction, with epoch)

Human-in-the-loop: forms, approvals, confirmations.

```rust
pub trait InteractiveAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    fn interaction_request(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<InteractionRequest, ActionError>> + Send;

    fn process_response(
        &self,
        input: Self::Input,
        response: InteractionResponse,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;

    fn on_timeout(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
        async { Err(ActionError::fatal("interaction timed out")) }
    }
}

/// Handle for matching responses to interactions. Prevents late/duplicate responses.
/// Pattern mirrors PendingToken from credential-hld.
pub struct InteractionHandle {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub epoch: u64,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

pub struct InteractionRequest {
    pub kind: InteractionKind,
    pub title: String,
    pub description: Option<String>,
    pub form: Option<ParameterCollection>,
    pub timeout: Option<Duration>,
    pub assignee: Option<String>,
}

#[non_exhaustive]
pub enum InteractionKind {
    Approval { approve_label: String, reject_label: String },
    Form,
    Confirmation { message: String },
    Selection { options: Vec<SelectionOption> },
}

pub struct InteractionResponse {
    pub handle: InteractionHandle,
    pub decision: InteractionDecision,
    pub form_values: Option<ParameterValues>,
    pub responded_by: String,
    pub responded_at: chrono::DateTime<chrono::Utc>,
}

#[non_exhaustive]
pub enum InteractionDecision {
    Approved,
    Rejected,
    FormSubmitted,
    Selected(String),
    TimedOut,
}
```

**Late response handling:** Response matched by (execution_id, node_id, epoch).
Expired/consumed → reject with audit trail.

---

## TransactionalAction (→ StatefulAction)

Saga pattern: execute + compensate.

```rust
pub trait TransactionalAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;
    type CompensationData: Serialize + DeserializeOwned + Send + Sync + 'static;

    fn execute_tx(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<(Self::Output, Self::CompensationData), ActionError>> + Send;

    fn compensate(
        &self,
        data: Self::CompensationData,
        ctx: &ActionContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}
```
