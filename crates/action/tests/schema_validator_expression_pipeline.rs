//! Phase 9 / Task 9.1 — Action input pipeline integration test.
//!
//! Exercises the full execution-time pipeline an engine traverses for one action node:
//!
//!   1. **Schema** — `Self::Input::schema()` (Phase 3 — `<Name>Input` is a separate
//!      `#[derive(Schema)]` companion struct).
//!   2. **Validator** — `ValidSchema::validate(&FieldValues)` proves the `{{ }}` placeholders are
//!      syntactically valid AND that literal fields satisfy `#[validate(...)]` rules.
//!   3. **Expression** — `ValidValues::resolve(&dyn ExpressionContext)` replaces every
//!      `FieldValue::Expression` with a literal evaluated against the runtime context.
//!   4. **Typed deserialize** — the resolved JSON tree is deserialized into the typed `Self::Input`
//!      via `serde_json::from_value`.
//!   5. **Execute** — the action's `StatelessAction::execute` receives the typed input and runs.
//!
//! The test pins fail-fast semantics at each stage:
//!
//! - validation rejects bad input before expression evaluation runs;
//! - expression evaluation rejects unevaluable templates before the typed `from_value` cast runs;
//! - the action body never sees bad input.

use std::sync::OnceLock;

use nebula_action::{
    ActionContext, action::Action, error::ActionError, metadata::ActionMetadata,
    result::ActionResult, stateless::StatelessAction, testing::TestContextBuilder,
};
use nebula_core::{Dependencies, action_key};
use nebula_expression::{EvaluationContext, ExpressionEngine};
use nebula_schema::{
    EvalFuture, ExpressionAst, ExpressionContext, Field, FieldValues, HasSchema, Schema,
    ValidSchema, field_key,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

// ── Action input/output ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PipelineInput {
    /// Required string with min-length rule. Tests literal-rule validation.
    name: String,
    /// Optional number that may be supplied as a `{{ ... }}` template.
    count: i64,
    /// Optional message — has no template, exercises the literal fast path.
    #[serde(default)]
    message: Option<String>,
}

impl HasSchema for PipelineInput {
    fn schema() -> ValidSchema {
        Schema::builder()
            .add(Field::string(field_key!("name")).required())
            .add(Field::number(field_key!("count")).integer())
            .add(Field::string(field_key!("message")))
            .build()
            .expect("PipelineInput schema is valid")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PipelineOutput {
    seen_name: String,
    seen_count: i64,
    seen_message: Option<String>,
}

impl HasSchema for PipelineOutput {
    fn schema() -> ValidSchema {
        ValidSchema::empty()
    }
}

// ── Action implementation ─────────────────────────────────────────────────

struct PipelineProbe;

impl Action for PipelineProbe {
    type Input = PipelineInput;
    type Output = PipelineOutput;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.phase9.pipeline_probe"),
            "PipelineProbe",
            "Phase 9 / Task 9.1 schema → validator → expression pipeline probe",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for PipelineProbe {
    async fn execute(
        &self,
        input: PipelineInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<PipelineOutput>, ActionError> {
        Ok(ActionResult::success(PipelineOutput {
            seen_name: input.name,
            seen_count: input.count,
            seen_message: input.message,
        }))
    }
}

// ── Bridge between nebula_expression and nebula_schema ────────────────────

/// Evaluates `{{ ... }}` AST nodes via a real `ExpressionEngine`.
///
/// `nebula_schema::ExpressionContext` is the dyn-safe seam for resolution.
/// `ExpressionAst` exposes only the source string (per design) so the bridge
/// re-routes through `engine.evaluate(source, &eval_ctx)` rather than
/// trying to share parsed AST nodes across crate boundaries.
struct EngineBridge {
    engine: ExpressionEngine,
    ctx: EvaluationContext,
}

impl ExpressionContext for EngineBridge {
    fn evaluate<'a>(&'a self, ast: &'a ExpressionAst) -> EvalFuture<'a> {
        let source = ast.source().to_owned();
        Box::pin(async move {
            self.engine.evaluate(&source, &self.ctx).map_err(|e| {
                nebula_schema::ValidationError::builder("expression.runtime")
                    .message(format!("expression `{source}` failed: {e}"))
                    .build()
            })
        })
    }
}

fn make_engine_bridge(input_var: serde_json::Value) -> EngineBridge {
    let mut ctx = EvaluationContext::new();
    ctx.set_input(input_var);
    EngineBridge {
        engine: ExpressionEngine::new(),
        ctx,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Happy path: literals + a `{{ $input.value }}` expression resolve, deserialize,
/// and execute end-to-end. Verifies the validator does not reject expression
/// values while the type rules wait for resolution.
#[tokio::test]
async fn pipeline_resolves_template_then_executes() {
    let schema = <PipelineInput as HasSchema>::schema();
    let raw = json!({
        "name": "alice",
        "count": { "$expr": "{{ $input.value }}" },
        "message": "static-text",
    });
    let values = FieldValues::from_json(raw).expect("ingest");

    // 1+2. Schema-time validation: literal `name` checked, `count` deferred
    //      because it is an expression, `message` checked.
    let validated = schema.validate(&values).expect("validate must pass");

    // 3. Expression resolution against $input.value = 42.
    let bridge = make_engine_bridge(json!({"value": 42}));
    let resolved = validated.resolve(&bridge).await.expect("resolve must pass");

    // 4. Typed deserialize from resolved JSON.
    let typed: PipelineInput =
        serde_json::from_value(resolved.into_json()).expect("typed deserialize");
    assert_eq!(typed.name, "alice");
    assert_eq!(typed.count, 42);
    assert_eq!(typed.message.as_deref(), Some("static-text"));

    // 5. Execute with the typed input.
    let action = PipelineProbe;
    let ctx = TestContextBuilder::new().build();
    let result = action.execute(typed, &ctx).await.expect("execute");
    let output = match result {
        ActionResult::Success { output, .. } => {
            output.into_value().expect("Success carries a typed value")
        },
        other => panic!("expected Success, got {other:?}"),
    };
    assert_eq!(output.seen_name, "alice");
    assert_eq!(output.seen_count, 42);
    assert_eq!(output.seen_message.as_deref(), Some("static-text"));
}

/// Stage 1 fail-fast: validation rejects a missing required field before
/// the engine ever invokes expression resolution.
#[tokio::test]
async fn pipeline_validation_rejects_missing_required_before_resolve() {
    let schema = <PipelineInput as HasSchema>::schema();
    // `name` is required but omitted.
    let raw = json!({
        "count": 1,
    });
    let values = FieldValues::from_json(raw).expect("ingest");
    let report = schema.validate(&values).expect_err("must fail");
    assert!(
        report
            .errors()
            .any(|e| e.code.as_ref() == "required" && e.path.to_string() == "name"),
        "expected `required` error on `name`, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

/// Stage 1 fail-fast: validation rejects a bad literal type before resolve.
/// `count` is declared as integer; supplying a literal string short-circuits
/// before any `{{ … }}` evaluation could happen.
#[tokio::test]
async fn pipeline_validation_rejects_bad_literal_type_before_resolve() {
    let schema = <PipelineInput as HasSchema>::schema();
    let raw = json!({
        "name": "alice",
        "count": "not-a-number",
    });
    let values = FieldValues::from_json(raw).expect("ingest");
    let report = schema.validate(&values).expect_err("must fail");
    assert!(
        report.errors().any(|e| e.code.as_ref() == "type_mismatch"),
        "expected type_mismatch, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

/// Stage 3 fail-fast: an expression that evaluates to an incompatible type
/// is caught at resolve time as `expression.type_mismatch`. The action's
/// `execute` is never reached.
#[tokio::test]
async fn pipeline_expression_type_mismatch_caught_before_execute() {
    let schema = <PipelineInput as HasSchema>::schema();
    let raw = json!({
        "name": "alice",
        // Integer field — but the expression resolves to a string.
        "count": { "$expr": "{{ $input.text }}" },
    });
    let values = FieldValues::from_json(raw).expect("ingest");
    let validated = schema.validate(&values).expect("validate must pass");

    // Bridge resolves `$input.text` → "not-a-number". Schema's post-resolve
    // re-validation fires.
    let bridge = make_engine_bridge(json!({"text": "not-a-number"}));
    let report = validated
        .resolve(&bridge)
        .await
        .expect_err("resolve must fail because resolved value is wrong type");
    assert!(
        report
            .errors()
            .any(|e| e.code.as_ref() == "expression.type_mismatch"),
        "expected expression.type_mismatch, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}

/// Stage 3 fail-fast: an expression whose evaluation itself errors out
/// surfaces as `expression.runtime` and never reaches `execute`.
#[tokio::test]
async fn pipeline_expression_runtime_error_caught_before_execute() {
    let schema = <PipelineInput as HasSchema>::schema();
    let raw = json!({
        "name": "alice",
        // A function call that doesn't exist — runtime failure.
        "count": { "$expr": "{{ $does_not_exist.field }}" },
    });
    let values = FieldValues::from_json(raw).expect("ingest");
    let validated = schema.validate(&values).expect("validate must pass");

    let bridge = make_engine_bridge(json!({}));
    let report = validated
        .resolve(&bridge)
        .await
        .expect_err("resolve must fail with runtime error");
    assert!(
        report
            .errors()
            .any(|e| e.code.as_ref().starts_with("expression.")),
        "expected an expression.* code, got: {:?}",
        report.errors().collect::<Vec<_>>()
    );
}
