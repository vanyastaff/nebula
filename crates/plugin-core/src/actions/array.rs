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

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Property, Schema, ValidSchema, ValidationReport, field_key};
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

impl HasSchema for ArrayInput {
    #[instrument(name = "core.array.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA.get_or_init(|| {
            Schema::builder()
                .property(Property::list(field_key!("data"))
                    .description("Required array of arbitrary JSON values; an empty array is valid.")
                    .item(Property::dynamic(field_key!("item"))))
                .property(Property::list(field_key!("operations")).item(
                    Property::object(field_key!("item"))
                        .description("Tagged array operation; variant-specific field presence is checked by serde.")
                        .property(Property::select(field_key!("op"))
                            .option("chunk", "Chunk").option("flatten", "Flatten")
                            .option("take", "Take").option("skip", "Skip").required())
                        .property(Property::integer(field_key!("size")).min_int(1).max(usize::MAX))
                        .property(Property::integer(field_key!("depth")).min_int(0).max(usize::MAX))
                        .property(Property::integer(field_key!("count")).min_int(0).max(usize::MAX)),
                ))
                .root_rule(super::input_schema::array_present(field_key!("data"))?)
                .build()
        }).clone()
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

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.array"),
            nebula_action::metadata_name!("Array"),
            "Shape a JSON array with chunk/flatten/take/skip operations applied left-to-right",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
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
#[path = "array_tests.rs"]
mod tests;
