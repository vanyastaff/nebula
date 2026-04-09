# Engine v2: ActionResult Handling + nebula-plugin-core

> Design spec for engine flow control and built-in core actions.
> Based on competitive analysis of Obelisk, Sayiir, Acts, Orka, Cano.

## Philosophy

- **Engine is a generic DAG executor.** It reacts to `ActionResult` variants. It never checks action keys.
- **nebula-plugin-core provides flow control actions.** If, Switch, Loop, Delay, Merge — all are normal actions returning ActionResult variants.
- **All state lives in ExecutionState.** One table, one state machine, one persist path. No separate wait queues.

---

## Part 1: Engine ActionResult Handling

### 1.1 ActionResult → Engine Behaviour Matrix

| ActionResult | Engine Behaviour | NodeState | Edge Activation |
|---|---|---|---|
| `Success { output }` | Store output, activate outgoing edges | `Completed` | Edges with `Always` or `OnResult { Success }` |
| `Skip { reason }` | Mark skipped, propagate skip to dependents | `Skipped` | None — dependents skipped |
| `Branch { selected, alternatives }` | Store selected output, activate only edge with matching `branch_key` | `Completed` | Only edge where `branch_key == selected` |
| `Continue { output, progress, delay }` | Store intermediate output, re-enqueue after delay | `Running` (stays) | None — node not finished |
| `Break { output, reason }` | Store final output, activate outgoing edges | `Completed` | Same as Success |
| `Wait { condition }` | Persist wait state, suspend node | `Waiting` (new) | None — node suspended |
| `Retry { after, reason }` | Re-enqueue after delay | `Retrying` | None — node retrying |

### 1.2 New NodeState: Waiting

```rust
pub enum NodeState {
    Pending, Ready, Running, Completed, Failed,
    Skipped, Retrying, Cancelled,
    Waiting,  // NEW — suspended, awaiting timer/signal
}
```

`Waiting` is not terminal, not active. Engine does not dispatch it but does not consider it finished.

### 1.3 NodeExecutionState Extensions

```rust
pub struct NodeExecutionState {
    pub state: NodeState,
    pub attempts: u32,
    pub error_message: Option<String>,
    // NEW:
    pub wait_condition: Option<WaitCondition>,
    pub intermediate_output: Option<serde_json::Value>,
    pub iteration_count: u32,
}
```

### 1.4 Wait Persistence

All waits persist through `ExecutionState` in the existing execution storage table:

```
node_states: {
    "node_2": {
        "state": "waiting",
        "wait_condition": { "type": "duration", "duration_ms": 3600000 },
        "iteration_count": 0
    }
}
```

On engine start / resume:
1. Load `ExecutionState` from storage
2. Find nodes with `state == Waiting`
3. For each: check if `wait_condition` is satisfied
   - `Duration` / `Until` → compare wall clock
   - `Webhook` / `Approval` → check if signal received (via API endpoint)
   - `Execution` → check if target execution completed
4. If satisfied → set `Ready`, re-dispatch
5. If not → schedule timer or wait for signal

### 1.5 NodeDefinition.trigger Field

```rust
pub enum TriggerMode {
    All,   // dispatch when ALL predecessors are terminal (default)
    Each,  // dispatch on EACH predecessor completion
}
```

```yaml
- name: "Merge Results"
  action_key: "core.merge"
  trigger: "each"
```

Engine does not know why a node uses `trigger: "each"`. It simply dispatches the node each time a predecessor completes. The action decides whether to return `Wait` (need more inputs) or `Success` (ready).

### 1.6 Branch Edge Activation

When engine receives `Branch { selected: "case_a" }`:
1. Find outgoing edges from this node
2. Filter to edges where `connection.branch_key == Some("case_a")`
3. Activate only those edges
4. Edges without `branch_key` or with different `branch_key` are NOT activated
5. Nodes downstream of non-activated edges are marked `Skipped`

### 1.7 Continue/Break Loop

When engine receives `Continue { output, delay }`:
1. Store `output` in `node.intermediate_output`
2. Increment `node.iteration_count`
3. If `delay.is_some()` → return `Wait { Duration(delay) }` internally
4. Else → immediately re-enqueue node in ready_queue
5. Node re-executes with `iteration_count` and `intermediate_output` in context

When engine receives `Break { output }`:
1. Store `output` as final node output
2. Mark node `Completed`
3. Activate outgoing edges (same as Success)

### 1.8 Wait Resolve API

External signals (webhook, approval) resolve waits via API:

```
POST /api/v1/executions/{execution_id}/signal
{
    "node_id": "...",
    "input": { "approved": true }
}
```

Engine:
1. Load execution state
2. Find node in `Waiting` state
3. Set `resume_input` from signal
4. Transition to `Ready`
5. Save state
6. Resume execution

---

## Part 2: nebula-plugin-core

### 2.1 core.if — Conditional Branch

**Modes:**

`rules` mode (UI-friendly):
```yaml
parameters:
  mode: "rules"
  combine: "all"           # all (AND) | any (OR)
  rules:
    - field: "{{ $node.Fetch.output.status }}"
      operator: "equals"
      value: 200
    - field: "{{ $node.Fetch.output.body.active }}"
      operator: "equals"
      value: true
```

`expression` mode (advanced):
```yaml
parameters:
  mode: "expression"
  condition: "{{ $node.Fetch.output.status >= 200 && $node.Fetch.output.status < 300 }}"
```

**Returns:** `Branch { selected: "true" }` or `Branch { selected: "false" }`

**Operators:**
```rust
pub enum Operator {
    Equals, NotEquals,
    GreaterThan, LessThan, GreaterThanOrEqual, LessThanOrEqual,
    Contains, NotContains,
    StartsWith, EndsWith,
    IsEmpty, IsNotEmpty,
    MatchesRegex,
    Exists, NotExists,
}

pub enum CombineMode {
    All,  // AND
    Any,  // OR
}

pub struct ConditionRule {
    pub field: String,
    pub operator: Operator,
    pub value: Option<serde_json::Value>,
}
```

### 2.2 core.switch — Multi-Branch

```yaml
parameters:
  field: "{{ $node.Input.output.type }}"
  cases:
    - name: "email"
      operator: "equals"
      value: "email"
    - name: "sms"
      operator: "equals"
      value: "sms"
  fallback: "email"
```

**Returns:** `Branch { selected: "email" }` — matches first case, or fallback.

### 2.3 core.delay — Durable Wait

```yaml
parameters:
  duration: "1h"          # or
  until: "2026-04-10T09:00:00Z"
```

**Returns:** `Wait { Duration(1h) }` or `Wait { Until(datetime) }`

### 2.4 core.wait_webhook — External Callback

```yaml
parameters:
  callback_id: "order_{{ $node.Create.output.id }}"
  timeout: "24h"
```

**Returns:** `Wait { Webhook { callback_id } }`

### 2.5 core.wait_approval — Human-in-the-Loop

```yaml
parameters:
  approver: "manager@company.com"
  message: "Approve order #{{ $node.Order.output.id }} for ${{ $node.Order.output.total }}"
```

**Returns:** `Wait { Approval { approver, message } }`

### 2.6 core.loop — Iteration

**ForEach mode:**
```yaml
parameters:
  mode: "foreach"
  items: "{{ $node.Fetch.output.orders }}"
  max_iterations: 1000
```

Action behaviour:
- First call: `Continue { output: { current_item: items[0], index: 0, total: N } }`
- Subsequent: reads `iteration_count` from context, returns next item
- Last item: `Break { output: { processed: N } }`

**While mode:**
```yaml
parameters:
  mode: "while"
  condition: "{{ $node.Check.output.status != 'ready' }}"
  max_iterations: 100
  delay: "5s"
```

Action behaviour:
- Eval condition → true: `Continue { delay: 5s }`
- Eval condition → false: `Break`
- Max iterations reached: `Break { reason: MaxIterations }`

### 2.7 core.merge — Fan-in

Uses `trigger: "each"` on NodeDefinition.

```yaml
- name: "Collect All"
  action_key: "core.merge"
  trigger: "each"
  parameters:
    join: "all"           # all | any | quorum
    quorum_min: 2         # for quorum mode
```

Action behaviour with `join: "all"`:
- Receives partial inputs on each predecessor completion
- Returns `Wait` until all predecessors complete
- When all done: `Success { output: merged_inputs }`

Action behaviour with `join: "any"`:
- First predecessor completes → `Success { output: first_input }`

Action behaviour with `join: "quorum"`:
- Counts completed predecessors
- When count >= quorum_min → `Success { output: completed_inputs }`

### 2.8 Existing actions (already built-in CLI)

These move from `apps/cli/src/actions.rs` to `nebula-plugin-core`:
- `core.set` (was `set`) — output fixed values
- `core.noop` (was `noop`) — passthrough
- `core.fail` (was `fail`) — force failure
- `core.echo` (was `echo`) — echo input
- `core.log` (was `log`) — log and passthrough
- `core.delay` (was `delay`) — simple delay (enhanced to use Wait)

---

## Part 3: Crate Organization

```
crates/plugin-core/          NEW — built-in flow control actions
  src/
    lib.rs                   Plugin impl, registers all core actions
    condition.rs             ConditionRule, Operator, CombineMode types
    actions/
      if_action.rs           core.if
      switch_action.rs       core.switch
      delay_action.rs        core.delay
      wait_webhook.rs        core.wait_webhook
      wait_approval.rs       core.wait_approval
      loop_action.rs         core.loop (foreach + while)
      merge_action.rs        core.merge
      set_action.rs          core.set
      noop_action.rs         core.noop
      fail_action.rs         core.fail
      echo_action.rs         core.echo
      log_action.rs          core.log

crates/workflow/src/node.rs  MODIFIED — add trigger: TriggerMode
crates/workflow/src/state.rs MODIFIED — add NodeState::Waiting
crates/execution/src/state.rs MODIFIED — add wait_condition, intermediate_output, iteration_count
crates/engine/src/engine.rs  MODIFIED — handle all ActionResult variants, trigger: each
apps/cli/src/actions.rs      MODIFIED — remove built-in actions, use plugin-core
```

---

## Part 4: Dependencies and Sequencing

### What can be built now (no blockers):
- NodeState::Waiting
- NodeExecutionState extensions
- Engine Branch handling
- Engine Continue/Break handling
- trigger: "each" field
- core.if, core.switch, core.set, core.noop, core.fail, core.echo, core.log
- core.loop (foreach — Continue/Break based)

### What needs Storage v1 (PgExecutionRepo):
- Wait persistence and resume across restarts
- core.delay with durable wait
- core.wait_webhook, core.wait_approval
- core.loop while mode with delay (durable timer)

### What needs API v1:
- POST /executions/{id}/signal endpoint for webhook/approval resolve

---

## Part 5: Breaking Changes

1. `NodeState` gains `Waiting` variant — already `#[non_exhaustive]`, backward compatible for deserialization
2. `NodeDefinition` gains `trigger: TriggerMode` field — `#[serde(default)]`, backward compatible
3. `NodeExecutionState` gains 3 new fields — `#[serde(default)]`, backward compatible
4. Built-in actions in CLI move to `nebula-plugin-core` — CLI imports from plugin-core instead of local actions module
5. Engine `run_frontier` changes to check `trigger` mode and handle all ActionResult variants — internal, no external API change
