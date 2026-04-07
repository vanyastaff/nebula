# Phase 0: Critical Fixes for Core & Cross-cutting Layers

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 5 critical bugs across expression, workflow, execution, and core crates that block node development.

**Architecture:** Each fix is independent — no cross-task dependencies. All are leaf changes within their respective crates. Breaking changes are permitted.

**Tech Stack:** Rust 1.94, edition 2024. Testing: `cargo nextest run`. Linting: `cargo clippy -- -D warnings`.

---

## File Map

| Task | Creates | Modifies |
|------|---------|----------|
| 1. Expression lexer Box::leak | — | `crates/expression/src/token.rs`, `crates/expression/src/lexer.rs` |
| 2. Expression cache error erasure | — | `crates/expression/src/engine.rs` |
| 3. Workflow NodeDefinition panic | — | `crates/workflow/src/node.rs`, `crates/workflow/src/error.rs` |
| 4. Execution missing transitions | — | `crates/execution/src/transition.rs` |
| 5. Core CredentialEvent typed ID | — | `crates/core/src/credential_event.rs` |

---

### Task 1: Fix `Box::leak` Memory Leak in Expression Lexer

**Problem:** `crates/expression/src/lexer.rs:320` leaks memory via `Box::leak` every time a string with escape sequences is lexed. The leak exists because `TokenKind::String(&'a str)` borrows from the input, but escape-processed strings are new allocations that can't borrow from input.

**Fix:** Change `TokenKind::String` from `&'a str` to `Cow<'a, str>`. Zero-copy path stays zero-copy (borrows input), escape path uses `Cow::Owned`.

**Files:**
- Modify: `crates/expression/src/token.rs:24-32` (TokenKind enum)
- Modify: `crates/expression/src/lexer.rs:263-267, 318-321` (both string paths)
- Modify: any files that pattern-match on `TokenKind::String(&str)` — parser, evaluator, Display impl

**Breaking change:** `TokenKind` loses `Copy` derive (Cow is not Copy). All pattern matches on `TokenKind::String(s)` now bind `&Cow<str>` instead of `&&str`.

- [ ] **Step 1: Write a test proving the leak exists (behaviorally)**

Add to `crates/expression/src/lexer.rs` in the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn escaped_string_does_not_leak() {
    // Parse a string with escapes — previously leaked via Box::leak.
    // After fix, this should work without unbounded memory growth.
    let input = r#""hello\nworld""#;
    let mut lexer = Lexer::new(input);
    let tokens = lexer.tokenize().unwrap();
    assert_eq!(tokens.len(), 2); // String + Eof
    match &tokens[0].kind {
        TokenKind::String(s) => assert_eq!(s.as_ref(), "hello\nworld"),
        other => panic!("expected String, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run test to verify it passes (it will — the leak is silent)**

Run: `cargo nextest run -p nebula-expression -- escaped_string_does_not_leak`

Expected: PASS (the bug is a memory leak, not incorrect output — the test establishes the behavioral contract)

- [ ] **Step 3: Change `TokenKind::String` to use `Cow<'a, str>`**

In `crates/expression/src/token.rs`:

Add import at the top:
```rust
use std::borrow::Cow;
```

Change the enum definition — remove `Copy` from derives and change String variant:
```rust
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    // Literals
    /// Integer literal (e.g., 42, -10)
    Integer(i64),
    /// Float literal (e.g., 3.14, -2.5)
    Float(f64),
    /// String literal (e.g., "hello", 'world')
    String(Cow<'a, str>),
    /// Boolean literal (true, false)
    Boolean(bool),
    /// Null literal
    Null,
    // ... rest unchanged
```

- [ ] **Step 4: Fix the zero-copy path in lexer**

In `crates/expression/src/lexer.rs:262-268`, change:
```rust
// old:
TokenKind::String(&self.input[start_pos + 1..end_pos]),
// new:
TokenKind::String(Cow::Borrowed(&self.input[start_pos + 1..end_pos])),
```

Add import at top of lexer.rs:
```rust
use std::borrow::Cow;
```

- [ ] **Step 5: Fix the escape path — remove `Box::leak`**

In `crates/expression/src/lexer.rs:318-321`, replace:
```rust
// old:
let leaked = Box::leak(result.into_boxed_str());
Ok(Token::new(TokenKind::String(leaked), span))
// new:
Ok(Token::new(TokenKind::String(Cow::Owned(result)), span))
```

- [ ] **Step 6: Fix all compilation errors from `Copy` removal and type change**

The `Copy` removal and `&str` → `Cow<str>` change will break pattern matches across the expression crate. Find all breakages:

Run: `cargo check -p nebula-expression 2>&1`

For each error, fix the pattern match. Common patterns:

```rust
// old:
TokenKind::String(s) => /* s is &str */
// new:
TokenKind::String(s) => /* s is &Cow<str>, use s.as_ref() or &**s where &str is needed */
```

In `token.rs` Display impl (line 226):
```rust
// old:
TokenKind::String(s) => write!(f, "\"{}\"", s),
// new — Cow<str> implements Display, so this just works:
TokenKind::String(s) => write!(f, "\"{}\"", s),
```

For the parser: any place doing `TokenKind::String(s)` where `s` was used as `&str`, use `s.as_ref()`.

- [ ] **Step 7: Run full expression test suite**

Run: `cargo nextest run -p nebula-expression`

Expected: All tests pass.

- [ ] **Step 8: Run clippy**

Run: `cargo clippy -p nebula-expression -- -D warnings`

Expected: Clean.

- [ ] **Step 9: Commit**

```bash
git add crates/expression/src/token.rs crates/expression/src/lexer.rs
# Also add any other files changed in step 6 (parser, evaluator, etc.)
git commit -m "$(cat <<'EOF'
fix(expression): replace Box::leak with Cow<str> in lexer

TokenKind::String now holds Cow<'a, str> instead of &'a str.
Zero-copy path unchanged (Cow::Borrowed). Escape path uses
Cow::Owned instead of leaking memory via Box::leak.

BREAKING: TokenKind no longer implements Copy.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Fix Expression Cache Error Erasure

**Problem:** `crates/expression/src/engine.rs:165-168` erases `ExpressionError` to a generic `MemoryError("parse expression failed")` because `ConcurrentComputeCache::get_or_compute` requires `FnOnce() -> Result<V, MemoryError>`. Users get useless error messages.

**Fix:** Use `cache.get()` + `cache.insert()` instead of `get_or_compute`. Parse outside the cache closure. Only cache successful parses.

**Files:**
- Modify: `crates/expression/src/engine.rs:155-200`

- [ ] **Step 1: Write a test proving the error is erased**

Add to `crates/expression/src/engine.rs` (or `crates/expression/tests/` if integration tests exist):

```rust
#[cfg(test)]
mod cache_error_tests {
    use super::*;

    #[test]
    fn cache_preserves_parse_error_details() {
        let config = EngineConfig {
            cache_capacity: 100,
            ..Default::default()
        };
        let engine = ExpressionEngine::with_config(config);
        let ctx = EvaluationContext::default();

        let err = engine.evaluate("@@@invalid syntax!!!", &ctx).unwrap_err();
        let msg = err.to_string();
        // Must NOT be a generic "parse expression failed" or "Invalid memory layout"
        assert!(
            !msg.contains("memory layout"),
            "Error was erased to MemoryError: {msg}"
        );
        assert!(
            !msg.contains("parse expression failed"),
            "Error was erased to generic message: {msg}"
        );
    }
}
```

- [ ] **Step 2: Run the test — expect FAIL**

Run: `cargo nextest run -p nebula-expression -- cache_preserves_parse_error_details`

Expected: FAIL — the error currently says "Invalid memory layout: parse expression failed" or similar.

- [ ] **Step 3: Rewrite `evaluate` to avoid `get_or_compute`**

In `crates/expression/src/engine.rs`, replace lines 162-172:

```rust
// old:
let ast = if let Some(cache) = &self.expr_cache {
    let key: Arc<str> = Arc::from(expression);
    cache.get_or_compute(key, || {
        self.parse_expression(expression).map_err(|_| {
            nebula_memory::MemoryError::invalid_layout("parse expression failed")
        })
    })?
} else {
    self.parse_expression(expression)?
};

// new:
let ast = if let Some(cache) = &self.expr_cache {
    let key: Arc<str> = Arc::from(expression);
    if let Some(cached) = cache.get(&key) {
        cached
    } else {
        let parsed = self.parse_expression(expression)?;
        // Best-effort cache insert — ignore eviction errors
        let _ = cache.insert(key, parsed.clone());
        parsed
    }
} else {
    self.parse_expression(expression)?
};
```

- [ ] **Step 4: Apply the same fix to `parse_template`**

In `crates/expression/src/engine.rs`, replace lines 189-196:

```rust
// old:
if let Some(cache) = &self.template_cache {
    let key: Arc<str> = Arc::from(source_str.as_str());
    let template = cache.get_or_compute(key, || {
        crate::Template::new(&source_str).map_err(|_| {
            nebula_memory::MemoryError::invalid_layout("template creation failed")
        })
    })?;
    Ok(template)

// new:
if let Some(cache) = &self.template_cache {
    let key: Arc<str> = Arc::from(source_str.as_str());
    if let Some(cached) = cache.get(&key) {
        Ok(cached)
    } else {
        let template = crate::Template::new(&source_str)?;
        let _ = cache.insert(key, template.clone());
        Ok(template)
    }
```

- [ ] **Step 5: Run the test — expect PASS**

Run: `cargo nextest run -p nebula-expression -- cache_preserves_parse_error_details`

Expected: PASS — error now preserves original `ExpressionError`.

- [ ] **Step 6: Run full test suite + clippy**

Run: `cargo nextest run -p nebula-expression && cargo clippy -p nebula-expression -- -D warnings`

Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add crates/expression/src/engine.rs
git commit -m "$(cat <<'EOF'
fix(expression): preserve parse errors instead of erasing to MemoryError

Use cache.get() + cache.insert() instead of get_or_compute() to
avoid the MemoryError type requirement. Parse errors now propagate
with full diagnostic information (position, expected tokens, etc.).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Fix `NodeDefinition::new` Panic on Invalid ActionKey

**Problem:** `crates/workflow/src/node.rs:44` calls `.parse().expect("valid ActionKey")` which panics if `action_key` is not valid (e.g., empty string, special chars). This is reachable from deserialized user JSON.

**Fix:** Change `NodeDefinition::new` to return `Result<Self, WorkflowError>`. Add a new `WorkflowError::InvalidActionKey` variant. Also add a `try_new` alias. **Breaking change** — all callers of `NodeDefinition::new` must handle the Result.

**Files:**
- Modify: `crates/workflow/src/error.rs` (new variant)
- Modify: `crates/workflow/src/node.rs:37-51` (return Result)

- [ ] **Step 1: Write a test for the panic case**

Add to `crates/workflow/src/node.rs` test module (or create one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::NodeId;

    #[test]
    fn new_rejects_invalid_action_key() {
        let result = NodeDefinition::new(NodeId::new(), "test", "INVALID KEY!!!");
        assert!(result.is_err());
    }

    #[test]
    fn new_accepts_valid_action_key() {
        let result = NodeDefinition::new(NodeId::new(), "test", "http_request");
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run the test — expect compilation failure**

Run: `cargo nextest run -p nebula-workflow -- new_rejects_invalid_action_key`

Expected: Compilation fails — `NodeDefinition::new` currently returns `Self`, not `Result`.

- [ ] **Step 3: Add `InvalidActionKey` variant to `WorkflowError`**

In `crates/workflow/src/error.rs`, add after the `GraphError` variant:

```rust
    /// Invalid action key format.
    #[classify(category = "validation", code = "WORKFLOW:INVALID_ACTION_KEY")]
    #[error("invalid action key `{key}`: {reason}")]
    InvalidActionKey {
        /// The invalid key string.
        key: String,
        /// Why it's invalid.
        reason: String,
    },
```

- [ ] **Step 4: Change `NodeDefinition::new` to return `Result`**

In `crates/workflow/src/node.rs`, replace lines 37-51:

```rust
impl NodeDefinition {
    /// Create a minimal node definition.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidActionKey`] if `action_key` is not a valid key
    /// (must be lowercase alphanumeric with underscores, dots, or hyphens).
    pub fn new(
        id: NodeId,
        name: impl Into<String>,
        action_key: impl AsRef<str>,
    ) -> Result<Self, crate::WorkflowError> {
        let key_str = action_key.as_ref();
        let parsed_key = key_str.parse().map_err(|e: domain_key::KeyParseError| {
            crate::WorkflowError::InvalidActionKey {
                key: key_str.to_string(),
                reason: e.to_string(),
            }
        })?;
        Ok(Self {
            id,
            name: name.into(),
            action_key: parsed_key,
            interface_version: None,
            parameters: HashMap::new(),
            retry_policy: None,
            timeout: None,
            description: None,
        })
    }
```

- [ ] **Step 5: Fix all callers across the workspace**

Run: `cargo check --workspace 2>&1 | head -80`

Every caller of `NodeDefinition::new(...)` now needs `.unwrap()` (in tests) or `?` (in production code). Common patterns:

In test code:
```rust
// old:
NodeDefinition::new(id, "name", "http_request")
// new:
NodeDefinition::new(id, "name", "http_request").unwrap()
```

In production code (builders, etc.):
```rust
// old:
NodeDefinition::new(id, name, key)
// new:
NodeDefinition::new(id, name, key)?
```

Search all files referencing `NodeDefinition::new`:
```bash
grep -rn "NodeDefinition::new" crates/
```

Fix each call site. This will touch files in nebula-workflow (builder, tests), nebula-engine (tests), nebula-sdk (if it exists), and potentially nebula-api.

- [ ] **Step 6: Run the tests**

Run: `cargo nextest run -p nebula-workflow -- new_rejects`

Expected: Both tests pass.

- [ ] **Step 7: Run full workspace check**

Run: `cargo check --workspace && cargo nextest run --workspace && cargo clippy --workspace -- -D warnings`

Expected: All pass.

- [ ] **Step 8: Commit**

```bash
git add crates/workflow/src/node.rs crates/workflow/src/error.rs
# Also add any other files with updated call sites
git commit -m "$(cat <<'EOF'
fix(workflow)!: NodeDefinition::new returns Result instead of panicking

BREAKING: NodeDefinition::new() now returns Result<Self, WorkflowError>.
Previously panicked via expect() on invalid ActionKey — reachable from
untrusted user input (deserialized workflow JSON).

New variant: WorkflowError::InvalidActionKey { key, reason }.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Add Missing Execution State Transitions

**Problem:** `crates/execution/src/transition.rs` is missing `Cancelling → Completed` and `Cancelling → TimedOut`. If all nodes finish after cancellation is requested (but before it propagates), the engine gets stuck — no valid transition out of `Cancelling` except `Cancelled` or `Failed`.

**Files:**
- Modify: `crates/execution/src/transition.rs:10-24, 72-180` (transitions + tests)

- [ ] **Step 1: Write tests for the missing transitions**

Add to `crates/execution/src/transition.rs` in the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn cancelling_can_transition_to_completed() {
    // If all nodes finish before cancellation propagates, execution completes.
    assert!(can_transition_execution(
        ExecutionStatus::Cancelling,
        ExecutionStatus::Completed
    ));
}

#[test]
fn cancelling_can_transition_to_timed_out() {
    // A timeout can fire while cancellation is in progress.
    assert!(can_transition_execution(
        ExecutionStatus::Cancelling,
        ExecutionStatus::TimedOut
    ));
}
```

- [ ] **Step 2: Run the tests — expect FAIL**

Run: `cargo nextest run -p nebula-execution -- cancelling_can_transition`

Expected: Both FAIL — these transitions are not in the `matches!` block.

- [ ] **Step 3: Add the missing transitions**

In `crates/execution/src/transition.rs:10-24`, add two new arms to the `matches!`:

```rust
pub fn can_transition_execution(from: ExecutionStatus, to: ExecutionStatus) -> bool {
    matches!(
        (from, to),
        (ExecutionStatus::Created, ExecutionStatus::Running)
            | (ExecutionStatus::Running, ExecutionStatus::Paused)
            | (ExecutionStatus::Running, ExecutionStatus::Cancelling)
            | (ExecutionStatus::Running, ExecutionStatus::Completed)
            | (ExecutionStatus::Running, ExecutionStatus::Failed)
            | (ExecutionStatus::Running, ExecutionStatus::TimedOut)
            | (ExecutionStatus::Paused, ExecutionStatus::Running)
            | (ExecutionStatus::Paused, ExecutionStatus::Cancelling)
            | (ExecutionStatus::Cancelling, ExecutionStatus::Cancelled)
            | (ExecutionStatus::Cancelling, ExecutionStatus::Failed)
            | (ExecutionStatus::Cancelling, ExecutionStatus::Completed)
            | (ExecutionStatus::Cancelling, ExecutionStatus::TimedOut)
    )
}
```

- [ ] **Step 4: Run the tests — expect PASS**

Run: `cargo nextest run -p nebula-execution`

Expected: All tests pass, including the two new ones.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p nebula-execution -- -D warnings`

Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add crates/execution/src/transition.rs
git commit -m "$(cat <<'EOF'
fix(execution): add Cancelling → Completed and Cancelling → TimedOut transitions

If all nodes finish before cancellation propagates, the execution should
complete normally. A timeout can also fire while cancellation is in
progress. Without these transitions the engine would get stuck in
Cancelling state.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Use Typed `CredentialId` in `CredentialEvent`

**Problem:** `crates/core/src/credential_event.rs` uses `credential_id: String` instead of `CredentialId` (a typed UUID wrapper). This undermines the entire typed-ID system — consumers must trust/parse raw strings.

**Fix:** Change `String` to `CredentialId`. **Breaking change** for anyone constructing or matching `CredentialEvent`.

**Files:**
- Modify: `crates/core/src/credential_event.rs` (the entire file)

- [ ] **Step 1: Write a test ensuring typed IDs compile**

Add to `crates/core/src/credential_event.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::CredentialId;

    #[test]
    fn credential_event_uses_typed_id() {
        let id = CredentialId::new();
        let event = CredentialEvent::Refreshed {
            credential_id: id,
        };
        assert_eq!(event.credential_id(), id);
    }

    #[test]
    fn display_shows_credential_id() {
        let id = CredentialId::new();
        let event = CredentialEvent::Revoked {
            credential_id: id,
        };
        let display = event.to_string();
        assert!(display.contains(&id.to_string()));
    }
}
```

- [ ] **Step 2: Run the test — expect compilation failure**

Run: `cargo nextest run -p nebula-core -- credential_event_uses_typed_id`

Expected: FAIL at compile time — `credential_id` is currently `String`, not `CredentialId`.

- [ ] **Step 3: Change the field type and update the entire file**

Replace `crates/core/src/credential_event.rs`:

```rust
//! Credential lifecycle events for cross-crate signaling.
//!
//! Emitted via `EventBus<CredentialEvent>` by the
//! credential resolver. Consumed by `nebula-resource` for pool invalidation
//! and by monitoring tools.
//!
//! Events carry credential ID only — **never credential data or secrets**.

use std::fmt;

use crate::CredentialId;

/// Cross-crate credential lifecycle event.
///
/// Emitted after credential state changes. All variants carry only
/// identifiers, never secret material.
///
/// # Usage
///
/// ```
/// use nebula_core::{CredentialEvent, CredentialId};
///
/// let id = CredentialId::new();
/// let event = CredentialEvent::Refreshed { credential_id: id };
/// assert_eq!(event.credential_id(), id);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialEvent {
    /// Auth material was refreshed (e.g., OAuth2 token refresh).
    ///
    /// Existing connections may still work. Pools should re-auth on next
    /// checkout.
    Refreshed {
        /// The credential instance ID.
        credential_id: CredentialId,
    },

    /// Credential was explicitly revoked.
    ///
    /// All connections using this credential **must** be terminated
    /// immediately.
    Revoked {
        /// The credential instance ID.
        credential_id: CredentialId,
    },
}

impl CredentialEvent {
    /// Returns the credential ID for all variants.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        match self {
            Self::Refreshed { credential_id } | Self::Revoked { credential_id } => *credential_id,
        }
    }
}

impl fmt::Display for CredentialEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refreshed { credential_id } => {
                write!(f, "credential refreshed: {credential_id}")
            }
            Self::Revoked { credential_id } => {
                write!(f, "credential revoked: {credential_id}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_event_uses_typed_id() {
        let id = CredentialId::new();
        let event = CredentialEvent::Refreshed {
            credential_id: id,
        };
        assert_eq!(event.credential_id(), id);
    }

    #[test]
    fn display_shows_credential_id() {
        let id = CredentialId::new();
        let event = CredentialEvent::Revoked {
            credential_id: id,
        };
        let display = event.to_string();
        assert!(display.contains(&id.to_string()));
    }
}
```

Key changes:
- `credential_id: String` → `credential_id: CredentialId`
- `credential_id(&self) -> &str` → `credential_id(&self) -> CredentialId` (Copy type, return by value)
- Doctest updated to use `CredentialId::new()`

- [ ] **Step 4: Fix downstream callers**

Run: `cargo check --workspace 2>&1 | head -80`

Find all places constructing `CredentialEvent` with a `String` and change to `CredentialId`:

```bash
grep -rn "CredentialEvent" crates/ --include="*.rs" | grep -v "credential_event.rs" | grep -v target
```

Common fix pattern:
```rust
// old:
CredentialEvent::Refreshed { credential_id: "some-id".to_string() }
// new:
CredentialEvent::Refreshed { credential_id: credential_id }  // already a CredentialId
```

For places that compared with `event.credential_id() == "some-string"`:
```rust
// old:
if event.credential_id() == stored_id_string { ... }
// new:
if event.credential_id() == stored_credential_id { ... }
```

- [ ] **Step 5: Run tests**

Run: `cargo nextest run -p nebula-core -- credential_event`

Expected: PASS.

- [ ] **Step 6: Run full workspace check**

Run: `cargo check --workspace && cargo clippy --workspace -- -D warnings`

Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/credential_event.rs
# Also add any other files with updated callers
git commit -m "$(cat <<'EOF'
fix(core)!: use typed CredentialId in CredentialEvent

BREAKING: CredentialEvent fields changed from String to CredentialId.
credential_id() now returns CredentialId (Copy) instead of &str.

This enforces type safety across the credential-resource event boundary
and prevents accidentally passing arbitrary strings as credential IDs.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final Verification

After all 5 tasks are complete:

- [ ] **Full workspace validation**

```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc
```

- [ ] **Update context files**

Update `.claude/crates/expression.md` — document Cow<str> in TokenKind, cache get+insert pattern.
Update `.claude/crates/workflow.md` — document NodeDefinition::new returns Result.
Update `.claude/crates/execution.md` — document new Cancelling transitions.
Update `.claude/crates/core.md` — document CredentialEvent uses typed CredentialId.
Update `.claude/active-work.md` — add Phase 0 completion.
