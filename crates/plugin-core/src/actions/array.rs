//! `core.array` — shape a JSON array with a sequence of structural operations.
//!
//! Each [`ArrayOp`] in the `operations` list is applied left-to-right to the
//! running array of values. Operations compose on whatever the previous one
//! produced: a [`ArrayOp::Chunk`] yields an array-of-arrays, so a following
//! [`ArrayOp::Flatten`] with `depth: 1` undoes it. Chaining is the point —
//! `[skip 1, chunk 2]` skips the first element and then groups the rest in pairs.
//!
//! ## Scope
//!
//! This action is a pure **transform** over its input array. It never generates
//! elements: there is no `range`/sequence op, because a generator would have no
//! input to compose against and would break the left-to-right pipeline. A future
//! standalone `core.range` action could fill the generation gap without
//! distorting this transform's semantics.
//!
//! `core.array` complements the `{{ }}` expression language, whose `flatten`
//! builtin is one-level-only and which has no `chunk`/`take`/`skip` over array
//! values composed as a pipeline.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [1, 2, 3, 4, 5],
//!   "operations": [
//!     { "op": "skip",    "count": 1 },
//!     { "op": "chunk",   "size": 2 }
//!   ]
//! }
//! ```
//!
//! ## Output
//!
//! The final JSON array. For the input above: `[[2, 3], [4, 5]]`.
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal** naming the actual type.
//!   Shaping a non-array is always an authoring mistake; this matches the sibling
//!   array nodes (`core.map`, `core.filter`, `core.sort`), which also reject a
//!   missing array rather than defaulting to `[]`.
//! - `Chunk { size: 0 }` → **Fatal**: a zero-width chunk has no meaning and would
//!   never make progress.
//! - `Flatten { depth: 0 }` → no-op (the array passes through unchanged).
//! - `Take` / `Skip` saturate: a `count` at or beyond the length keeps the whole
//!   array (`Take`) or empties it (`Skip`); neither is an error.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::util::ValueTypeNameStr;

// ── Config types ──────────────────────────────────────────────────────────────

/// Default flatten depth when `depth` is omitted: a single level.
fn default_depth() -> usize {
    1
}

/// A single structural step applied to the running array of values.
///
/// Operations are applied in declaration order. Forward-compatibility for new
/// optional fields is handled via `#[serde(default)]` in future versions, not
/// `#[non_exhaustive]`, because these types are deserialized from workflow JSON
/// rather than literal-constructed by external Rust code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ArrayOp {
    /// Split the array into consecutive sub-arrays of `size` elements.
    ///
    /// The final sub-array may be shorter when the length is not a multiple of
    /// `size`. Produces an array-of-arrays. `size == 0` is a fatal error.
    Chunk {
        /// Number of elements per sub-array; must be greater than zero.
        size: usize,
    },

    /// Flatten nested array elements up to `depth` levels deep.
    ///
    /// Non-array elements pass through unchanged at every level. `depth == 1`
    /// (the default) merges one level of nesting; higher values flatten deeper.
    /// `depth == 0` is a no-op.
    Flatten {
        /// Number of nesting levels to merge; `0` is a no-op, default `1`.
        #[serde(default = "default_depth")]
        depth: usize,
    },

    /// Keep only the first `count` elements.
    ///
    /// A `count` at or beyond the array length keeps the whole array; `count == 0`
    /// produces an empty array.
    Take {
        /// Number of leading elements to retain.
        count: usize,
    },

    /// Drop the first `count` elements, keeping the rest.
    ///
    /// A `count` at or beyond the array length produces an empty array;
    /// `count == 0` leaves the array unchanged.
    Skip {
        /// Number of leading elements to discard.
        count: usize,
    },
}

/// Resolved input for [`ArrayAction`].
///
/// The engine resolves `NodeDefinition::parameters` into this struct before
/// dispatching. `data` must be a JSON array; operations are applied in
/// declaration order, each transforming whatever the previous one produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayInput {
    /// Array of values to shape. Must be a JSON array when present;
    /// `null` / absent is rejected with a Fatal error.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered list of structural operations applied left-to-right.
    #[serde(default)]
    pub operations: Vec<ArrayOp>,
}

// `data` is a fully dynamic JSON array and `operations` carry only integer
// counts — no closed-form schema can be emitted. Empty schema is the honest
// declaration; the module doc describes the expected structure out-of-band.
impl HasSchema for ArrayInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Operation appliers ─────────────────────────────────────────────────────────

/// Split `elements` into consecutive chunks of `size`, producing an
/// array-of-arrays. The last chunk may be shorter than `size`.
///
/// # Errors
///
/// Returns [`ActionError::Fatal`] when `size == 0`: a zero-width chunk never
/// makes progress and has no meaningful output.
fn apply_chunk(elements: Vec<Value>, size: usize) -> Result<Vec<Value>, ActionError> {
    if size == 0 {
        return Err(ActionError::fatal("core.array: chunk size must be > 0"));
    }
    let chunked = elements
        .chunks(size)
        .map(|chunk| Value::Array(chunk.to_vec()))
        .collect();
    Ok(chunked)
}

/// Flatten array elements of `elements` up to `depth` levels.
///
/// Each level merges one tier of nesting: an array element is spliced in place
/// of itself, a non-array element passes through unchanged. `depth == 0` returns
/// the input untouched.
fn apply_flatten(elements: Vec<Value>, depth: usize) -> Vec<Value> {
    let mut current = elements;
    for _ in 0..depth {
        // Stop early once no element is an array — further passes are no-ops.
        if !current.iter().any(Value::is_array) {
            break;
        }
        let mut flattened: Vec<Value> = Vec::with_capacity(current.len());
        for value in current {
            match value {
                Value::Array(inner) => flattened.extend(inner),
                other => flattened.push(other),
            }
        }
        current = flattened;
    }
    current
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that shapes a JSON array with a sequence of chunk/flatten/take/skip
/// operations applied left-to-right.
///
/// Keyed `core.array`. No I/O, no credentials, no resources.
///
/// # Example
///
/// Operations serialize to a tagged JSON object; the `"op"` field drives
/// deserialization back to the correct variant:
///
/// ```rust
/// use nebula_plugin_core::actions::array::ArrayOp;
/// use serde_json::json;
///
/// let op = ArrayOp::Chunk { size: 2 };
///
/// // Wire shape: {"op":"chunk","size":2}
/// let wire = serde_json::to_value(&op).unwrap();
/// assert_eq!(wire, json!({"op": "chunk", "size": 2}));
///
/// // Round-trip: deserialize back to the same variant
/// let restored: ArrayOp = serde_json::from_value(wire).unwrap();
/// assert_eq!(restored, op);
/// ```
///
/// Wire the action into the engine via [`CorePlugin`](crate::CorePlugin) and
/// `WorkflowEngine::with_plugin` — see the crate-level docs for a complete
/// wiring example.
#[derive(Debug)]
pub struct ArrayAction;

impl nebula_action::action::Action for ArrayAction {
    type Input = ArrayInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.array"),
            "Array",
            "Shape a JSON array with chunk/flatten/take/skip operations applied left-to-right",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for ArrayAction {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(ArrayAction)
    }
}

impl StatelessAction for ArrayAction {
    #[instrument(name = "core.array", skip_all, fields(op_count = input.operations.len()))]
    async fn execute(
        &self,
        input: ArrayInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data is a JSON array ──────────────────────────────────
        let mut working: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "core.array: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "core.array: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        // ── 2. Apply operations left-to-right ─────────────────────────────────
        //
        // Each op consumes the working vec and produces the next one. Ops compose
        // on whatever the previous op produced (e.g. Chunk yields an
        // array-of-arrays that a following Flatten{1} can merge back).
        for operation in input.operations {
            working = match operation {
                ArrayOp::Chunk { size } => apply_chunk(working, size)?,
                ArrayOp::Flatten { depth } => apply_flatten(working, depth),
                ArrayOp::Take { count } => {
                    working.truncate(count);
                    working
                },
                ArrayOp::Skip { count } => {
                    let drop = count.min(working.len());
                    working.drain(..drop);
                    working
                },
            };
        }

        Ok(ActionResult::success(Value::Array(working)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use serde_json::json;

    use super::*;

    fn run(input: ArrayInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = ArrayAction;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn extract_output(result: ActionResult<Value>) -> Value {
        result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .expect("ActionResult must carry a primary output value")
    }

    // ── Chunk ─────────────────────────────────────────────────────────────────

    // Even split: 4 elements / size 2 → two full chunks.
    #[tokio::test]
    async fn chunk_even_split() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3, 4])),
            operations: vec![ArrayOp::Chunk { size: 2 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([[1, 2], [3, 4]]));
    }

    // Uneven split: 5 elements / size 2 → last chunk is shorter.
    #[tokio::test]
    async fn chunk_uneven_last_chunk_shorter() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3, 4, 5])),
            operations: vec![ArrayOp::Chunk { size: 2 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([[1, 2], [3, 4], [5]]));
    }

    // RED witness: drop the `size == 0` guard in `apply_chunk` and this test
    // fails — `slice::chunks(0)` panics, so the action would panic instead of
    // returning Fatal.
    #[tokio::test]
    async fn chunk_size_zero_is_fatal() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3])),
            operations: vec![ArrayOp::Chunk { size: 0 }],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for chunk size 0; got: {err:?}"
        );
    }

    // ── Flatten ───────────────────────────────────────────────────────────────

    // One level: [[1,2],[3,4]] → [1,2,3,4].
    #[tokio::test]
    async fn flatten_one_level() {
        let input = ArrayInput {
            data: Some(json!([[1, 2], [3, 4]])),
            operations: vec![ArrayOp::Flatten { depth: 1 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2, 3, 4]));
    }

    // Deep: depth 2 merges two nesting levels; depth 1 would only merge one.
    #[tokio::test]
    async fn flatten_deep_depth_two() {
        let input = ArrayInput {
            data: Some(json!([[1, [2, 3]], [4, [5]]])),
            operations: vec![ArrayOp::Flatten { depth: 2 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2, 3, 4, 5]));
    }

    // depth 1 leaves the inner nesting intact (contrast with the deep test).
    #[tokio::test]
    async fn flatten_depth_one_leaves_inner_nesting() {
        let input = ArrayInput {
            data: Some(json!([[1, [2, 3]], [4, [5]]])),
            operations: vec![ArrayOp::Flatten { depth: 1 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, [2, 3], 4, [5]]));
    }

    // Non-array elements pass through unchanged at every level.
    #[tokio::test]
    async fn flatten_non_array_elements_pass_through() {
        let input = ArrayInput {
            data: Some(json!([1, [2, 3], "x", [4]])),
            operations: vec![ArrayOp::Flatten { depth: 1 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2, 3, "x", 4]));
    }

    // depth 0 is a no-op: the array passes through unchanged.
    #[tokio::test]
    async fn flatten_depth_zero_is_noop() {
        let input = ArrayInput {
            data: Some(json!([[1, 2], [3]])),
            operations: vec![ArrayOp::Flatten { depth: 0 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([[1, 2], [3]]));
    }

    // Default depth (omitted on the wire) is 1.
    #[tokio::test]
    async fn flatten_default_depth_is_one() {
        let op: ArrayOp = serde_json::from_value(json!({"op": "flatten"})).unwrap();
        let input = ArrayInput {
            data: Some(json!([[1], [2, 3]])),
            operations: vec![op],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2, 3]));
    }

    // ── Take ──────────────────────────────────────────────────────────────────

    // count < len keeps the leading prefix.
    #[tokio::test]
    async fn take_count_less_than_len() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3, 4])),
            operations: vec![ArrayOp::Take { count: 2 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2]));
    }

    // count >= len keeps the whole array (saturating).
    #[tokio::test]
    async fn take_count_at_or_beyond_len_keeps_all() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3])),
            operations: vec![ArrayOp::Take { count: 10 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2, 3]));
    }

    // count == 0 produces an empty array.
    #[tokio::test]
    async fn take_count_zero_is_empty() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3])),
            operations: vec![ArrayOp::Take { count: 0 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([]));
    }

    // ── Skip ──────────────────────────────────────────────────────────────────

    // count < len drops the leading prefix.
    #[tokio::test]
    async fn skip_count_less_than_len() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3, 4])),
            operations: vec![ArrayOp::Skip { count: 1 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([2, 3, 4]));
    }

    // count >= len produces an empty array (saturating, never panics on drain).
    #[tokio::test]
    async fn skip_count_at_or_beyond_len_is_empty() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3])),
            operations: vec![ArrayOp::Skip { count: 10 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([]));
    }

    // ── Composition ─────────────────────────────────────────────────────────────

    // [skip 1, chunk 2]: drop the first element, then group the rest in pairs.
    #[tokio::test]
    async fn compose_skip_then_chunk() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3, 4, 5])),
            operations: vec![ArrayOp::Skip { count: 1 }, ArrayOp::Chunk { size: 2 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([[2, 3], [4, 5]]));
    }

    // Chunk then Flatten{1} round-trips back to the original flat array — proving
    // ops compose on whatever the previous op produced (array-of-arrays here).
    #[tokio::test]
    async fn compose_chunk_then_flatten_round_trips() {
        let input = ArrayInput {
            data: Some(json!([1, 2, 3, 4, 5])),
            operations: vec![ArrayOp::Chunk { size: 2 }, ArrayOp::Flatten { depth: 1 }],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!([1, 2, 3, 4, 5]));
    }

    // ── Data / operations edge cases ────────────────────────────────────────────

    // Empty operations list returns the data unchanged.
    #[tokio::test]
    async fn empty_operations_returns_data_unchanged() {
        let data = json!([1, 2, 3]);
        let input = ArrayInput {
            data: Some(data.clone()),
            operations: vec![],
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, data);
    }

    // RED witness: drop the non-array guard arm and `unwrap_err()` panics on the
    // Ok result. Non-array `data` must be Fatal.
    #[tokio::test]
    async fn non_array_data_is_fatal() {
        let input = ArrayInput {
            data: Some(json!({"a": 1})),
            operations: vec![],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for object data; got: {err:?}"
        );
    }

    // null data is Fatal (consistent with the sibling array nodes).
    #[tokio::test]
    async fn null_data_is_fatal() {
        let input = ArrayInput {
            data: Some(Value::Null),
            operations: vec![],
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for null data; got: {err:?}"
        );
    }

    // ── Metadata ────────────────────────────────────────────────────────────────

    #[test]
    fn action_key_is_core_dot_array() {
        use nebula_action::action::Action;
        assert_eq!(ArrayAction::metadata().base.key.as_str(), "core.array");
    }

    // ── Serde round-trip for each ArrayOp wire shape ────────────────────────────

    #[test]
    fn serde_roundtrip_chunk() {
        let op = ArrayOp::Chunk { size: 2 };
        let wire = serde_json::to_value(&op).unwrap();
        assert_eq!(wire, json!({"op": "chunk", "size": 2}));
        let restored: ArrayOp = serde_json::from_value(wire).unwrap();
        assert_eq!(restored, op);
    }

    #[test]
    fn serde_roundtrip_flatten() {
        let op = ArrayOp::Flatten { depth: 3 };
        let wire = serde_json::to_value(&op).unwrap();
        assert_eq!(wire, json!({"op": "flatten", "depth": 3}));
        let restored: ArrayOp = serde_json::from_value(wire).unwrap();
        assert_eq!(restored, op);
    }

    #[test]
    fn serde_roundtrip_take() {
        let op = ArrayOp::Take { count: 5 };
        let wire = serde_json::to_value(&op).unwrap();
        assert_eq!(wire, json!({"op": "take", "count": 5}));
        let restored: ArrayOp = serde_json::from_value(wire).unwrap();
        assert_eq!(restored, op);
    }

    #[test]
    fn serde_roundtrip_skip() {
        let op = ArrayOp::Skip { count: 1 };
        let wire = serde_json::to_value(&op).unwrap();
        assert_eq!(wire, json!({"op": "skip", "count": 1}));
        let restored: ArrayOp = serde_json::from_value(wire).unwrap();
        assert_eq!(restored, op);
    }
}
