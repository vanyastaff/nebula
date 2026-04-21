//! Validated schema handles — proof-tokens.

use std::{future::Future, pin::Pin, sync::Arc};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{
    error::{Severity, ValidationError, ValidationReport},
    expression::ExpressionContext,
    field::{Field, ListField, NumberField, ObjectField},
    key::FieldKey,
    mode::{ExpressionMode, RequiredMode, VisibilityMode},
    path::FieldPath,
    value::{FieldValue, FieldValues},
};

/// Flags computed once at build time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaFlags {
    /// Whether any field allows or requires expressions.
    pub uses_expressions: bool,
    /// Whether any loader-backed field is present (async path).
    pub has_async_loaders: bool,
    /// Maximum nesting depth reached in this schema.
    pub max_depth: u8,
}

/// Cursor into the field tree: breadcrumb of child indices starting from root.
#[derive(Debug, Clone)]
pub struct FieldHandle {
    /// Index path from root fields vec downward.
    pub cursor: SmallVec<[u16; 4]>,
    /// Depth (1 = top-level).
    pub depth: u8,
}

/// Shared interior of a `ValidSchema`.
#[derive(Debug)]
pub struct ValidSchemaInner {
    /// Top-level ordered fields.
    pub fields: Vec<Field>,
    /// Flat index from `FieldPath` → `FieldHandle` for O(1) path lookup.
    pub index: IndexMap<FieldPath, FieldHandle>,
    /// Flags computed during build.
    pub flags: SchemaFlags,
}

/// Proof-token: schema has been built and linted successfully.
///
/// Cheap to clone — backed by `Arc`.
///
/// Serde: serializes as the ordered field list (same wire format as `Schema`).
/// Deserialization rebuilds through `Schema::builder()` /
/// [`SchemaBuilder`](crate::schema::SchemaBuilder); invalid wire data returns a
/// [`serde::de::Error`] (lint failures are not panics).
#[derive(Debug, Clone)]
pub struct ValidSchema(pub(crate) Arc<ValidSchemaInner>);

impl PartialEq for ValidSchema {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.fields == other.0.fields
    }
}

impl Eq for ValidSchema {}

impl Serialize for ValidSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Serialize as `{"fields": [...]}` — same wire format as `Schema`.
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("ValidSchema", 1)?;
        s.serialize_field("fields", &self.0.fields)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for ValidSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Transparent wrapper that mirrors `Schema`'s serde representation.
        #[derive(Deserialize)]
        struct ValidSchemaRepr {
            #[serde(default)]
            fields: Vec<Field>,
        }
        let repr = ValidSchemaRepr::deserialize(deserializer)?;
        repr.fields
            .into_iter()
            .fold(
                crate::schema::SchemaBuilder::default(),
                super::schema::SchemaBuilder::add,
            )
            .build()
            .map_err(|report| serde::de::Error::custom(format!("invalid schema: {report:?}")))
    }
}

impl ValidSchema {
    pub(crate) fn from_inner(inner: ValidSchemaInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Shared empty `ValidSchema` — cheap `Arc` clone.
    ///
    /// Use this anywhere a `ValidSchema` is required but the entity has no
    /// user-configurable inputs (actions that take `()`, stub credentials,
    /// baseline `HasSchema` impls for primitives). Avoids the
    /// `Schema::builder().build().expect(..)` incantation in several dozen
    /// call sites, and — unlike that pattern — cannot panic: the empty
    /// `ValidSchemaInner` is constructed directly, bypassing lint passes
    /// whose only job is to reject non-empty schemas.
    pub fn empty() -> Self {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<ValidSchema> = OnceLock::new();
        EMPTY
            .get_or_init(|| {
                Self::from_inner(ValidSchemaInner {
                    fields: Vec::new(),
                    index: IndexMap::new(),
                    flags: SchemaFlags::default(),
                })
            })
            .clone()
    }

    /// Return `true` when two `ValidSchema` values share the same backing
    /// `Arc` — i.e. they're the same instance, not just structurally
    /// equivalent. Used to assert identity-preserving caches (e.g. the
    /// `OnceLock` inside `#[derive(Schema)]`).
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Borrow all top-level fields in insertion order.
    pub fn fields(&self) -> &[Field] {
        &self.0.fields
    }

    /// Borrow the build-time flags.
    pub fn flags(&self) -> &SchemaFlags {
        &self.0.flags
    }

    /// Find a top-level field by key.
    pub fn find(&self, key: &FieldKey) -> Option<&Field> {
        self.0.fields.iter().find(|f| f.key() == key)
    }

    /// Find a field by dotted path using the O(1) index.
    pub fn find_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let handle = self.0.index.get(path)?;
        let mut cur = self.0.fields.get(*handle.cursor.first()? as usize)?;
        for &step in &handle.cursor[1..] {
            cur = match cur {
                Field::Object(o) => o.fields.get(step as usize)?,
                Field::List(l) => l.item.as_deref()?,
                Field::Mode(m) => &m.variants.get(step as usize)?.field,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Validate runtime `values` against this schema (schema-time phase).
    ///
    /// Two-phase expression handling:
    /// - `ExpressionMode::Forbidden` + `FieldValue::Expression` → hard error
    ///   `"expression.forbidden"`.
    /// - Any other `FieldValue::Expression` → skip value/type rules; run predicate rules
    ///   (visibility/required `When(rule)`) only.
    ///
    /// Full value rules (type check, range, pattern …) run on literal values
    /// here; expression resolution happens in [`ValidValues::resolve`].
    ///
    /// # Errors
    ///
    /// Returns `Err(ValidationReport)` when any hard error is found.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_schema::{Field, FieldValues, Schema, field_key};
    /// use serde_json::json;
    ///
    /// let schema = Schema::builder()
    ///     .add(Field::string(field_key!("name")).required())
    ///     .build()
    ///     .unwrap();
    ///
    /// // Missing required field → error.
    /// let empty = FieldValues::from_json(json!({})).unwrap();
    /// assert!(schema.validate(&empty).is_err());
    ///
    /// // Present required field → ok.
    /// let full = FieldValues::from_json(json!({"name": "Alice"})).unwrap();
    /// assert!(schema.validate(&full).is_ok());
    /// ```
    #[allow(clippy::result_large_err)]
    pub fn validate(&self, values: &FieldValues) -> Result<ValidValues, ValidationReport> {
        use crate::context::RootContext;

        let mut report = ValidationReport::new();
        let ctx = RootContext(values);

        for field in &self.0.fields {
            let path = FieldPath::root().join(field.key().clone());
            validate_field(field, values.get(field.key()), &ctx, &path, &mut report);
        }

        if report.has_errors() {
            return Err(report);
        }

        let warnings: Arc<[ValidationError]> = report
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();
        Ok(ValidValues {
            schema: self.clone(),
            values: values.clone(),
            warnings,
        })
    }
}

/// Validated values — tied to a specific `ValidSchema`.
///
/// Produced by `ValidSchema::validate()` (Task 21). Proof-token that values
/// have been checked against the schema at least once.
///
/// Owns an `Arc`-backed clone of the schema so the token can cross `.await`
/// boundaries and be handed off to the engine without self-referential
/// lifetimes.
#[derive(Debug, Clone)]
pub struct ValidValues {
    pub(crate) schema: ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl ValidValues {
    /// Borrow the schema these values were validated against.
    pub fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Borrow the raw value tree.
    pub fn raw(&self) -> &FieldValues {
        &self.values
    }

    /// Borrow the raw value tree (alias for [`raw`](Self::raw)).
    pub fn raw_values(&self) -> &FieldValues {
        &self.values
    }

    /// Iterate validation warnings that were non-fatal.
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a top-level value by key.
    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> {
        self.values.get(key)
    }

    /// Look up a value by dotted path.
    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> {
        self.values.get_path(path)
    }

    /// Resolve all `FieldValue::Expression` entries by evaluating them through
    /// `ctx`.
    ///
    /// **Fast path**: when `schema.flags().uses_expressions == false` the
    /// existing value tree is promoted directly to `ResolvedValues` without
    /// any walking.
    ///
    /// After evaluating each expression the resolved literal is validated
    /// against the same type/rule constraints that were skipped at
    /// schema-validate time. If any expression evaluation or post-resolve
    /// rule fails, all errors are collected and returned as a
    /// `ValidationReport`.
    ///
    /// # Errors
    ///
    /// Returns `Err(ValidationReport)` when any expression evaluation fails
    /// or when a resolved value violates a field rule.
    #[allow(clippy::result_large_err)]
    pub async fn resolve(
        self,
        ctx: &dyn ExpressionContext,
    ) -> Result<ResolvedValues, ValidationReport> {
        // Fast path — no expressions in this schema.
        if !self.schema.flags().uses_expressions {
            return Ok(ResolvedValues {
                schema: self.schema,
                values: self.values,
                warnings: self.warnings,
            });
        }

        let mut report = ValidationReport::new();
        let mut values = self.values;

        // Walk and resolve every entry in the flat top-level map.
        let keys: Vec<FieldKey> = values.iter().map(|(k, _)| k.clone()).collect();
        for key in keys {
            let Some(value) = values.get(&key).cloned() else {
                continue;
            };
            let path = FieldPath::root().join(key.clone());
            let resolved = resolve_value(value, ctx, &path, &mut report).await;
            values.set(key, resolved);
        }

        if report.has_errors() {
            return Err(report);
        }

        let extra_warnings: Vec<ValidationError> = report
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();
        let all_warnings: Arc<[ValidationError]> = self
            .warnings
            .iter()
            .chain(extra_warnings.iter())
            .cloned()
            .collect();

        Ok(ResolvedValues {
            schema: self.schema,
            values,
            warnings: all_warnings,
        })
    }
}

/// Resolved values — all `FieldValue::Expression` entries have been evaluated.
///
/// Produced by `ValidValues::resolve()` (Task 23). Proof-token that no
/// expression placeholders remain in the value tree.
///
/// Owns an `Arc`-backed clone of the schema so it is freely `Send + 'static`
/// and safe to persist or hand off to runtime.
#[derive(Debug, Clone)]
pub struct ResolvedValues {
    pub(crate) schema: ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl ResolvedValues {
    /// Borrow the schema these values were resolved against.
    pub fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Borrow the resolved value tree.
    ///
    /// Guaranteed to contain no `FieldValue::Expression` variants after
    /// successful resolution.
    pub fn values(&self) -> &FieldValues {
        &self.values
    }

    /// Iterate resolution warnings.
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a resolved literal value by key.
    ///
    /// Returns `None` if the field is absent or still an expression
    /// (should not happen in a properly resolved set).
    pub fn get(&self, key: &FieldKey) -> Option<&serde_json::Value> {
        match self.values.get(key)? {
            FieldValue::Literal(v) => Some(v),
            _ => None,
        }
    }

    /// Consume into a flat JSON object.
    pub fn into_json(self) -> serde_json::Value {
        self.values.to_json()
    }

    /// Consume and deserialize into a typed value.
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, Box<ValidationError>> {
        serde_json::from_value(self.into_json()).map_err(|e| {
            Box::new(
                ValidationError::builder("type_mismatch")
                    .message(format!("deserialize failed: {e}"))
                    .build(),
            )
        })
    }
}

// ── Expression resolution helpers ────────────────────────────────────────────

/// Recursively walk a [`FieldValue`], replacing every `Expression` variant
/// with a `Literal` obtained by calling `ctx.evaluate(ast)`.
///
/// Any evaluation error is pushed into `report` and the placeholder is left
/// as `Literal(Value::Null)` so that the walk can continue and collect all
/// errors in one pass.
///
/// The function is recursive and uses `Box::pin` to satisfy the async
/// recursion requirement.
fn resolve_value<'v>(
    value: FieldValue,
    ctx: &'v dyn ExpressionContext,
    path: &'v FieldPath,
    report: &'v mut ValidationReport,
) -> Pin<Box<dyn Future<Output = FieldValue> + 'v>> {
    Box::pin(async move {
        match value {
            FieldValue::Expression(ref expr) => {
                match expr.parse() {
                    Ok(ast) => match ctx.evaluate(ast).await {
                        Ok(v) => FieldValue::Literal(v),
                        Err(mut e) => {
                            // Attach path context and enforce the standard code.
                            if e.code == "expression.runtime" {
                                e.path = path.clone();
                            } else {
                                e = ValidationError::builder("expression.runtime")
                                    .at(path.clone())
                                    .message(e.message.clone())
                                    .build();
                            }
                            report.push(e);
                            FieldValue::Literal(serde_json::Value::Null)
                        },
                    },
                    Err(mut e) => {
                        e.path = path.clone();
                        report.push(e);
                        FieldValue::Literal(serde_json::Value::Null)
                    },
                }
            },
            FieldValue::Object(map) => {
                let mut out = IndexMap::with_capacity(map.len());
                for (k, v) in map {
                    let child_path = path.clone().join(k.clone());
                    let resolved = resolve_value(v, ctx, &child_path, report).await;
                    out.insert(k, resolved);
                }
                FieldValue::Object(out)
            },
            FieldValue::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (i, v) in items.into_iter().enumerate() {
                    let item_path = path.clone().join(i);
                    let resolved = resolve_value(v, ctx, &item_path, report).await;
                    out.push(resolved);
                }
                FieldValue::List(out)
            },
            FieldValue::Mode { mode, value } => {
                let resolved_value = if let Some(inner) = value {
                    let inner_path = path
                        .clone()
                        .join(FieldKey::new("value").expect("static key"));
                    let resolved = resolve_value(*inner, ctx, &inner_path, report).await;
                    Some(Box::new(resolved))
                } else {
                    None
                };
                FieldValue::Mode {
                    mode,
                    value: resolved_value,
                }
            },
            // Literals pass through unchanged.
            other => other,
        }
    })
}

// ── Schema-time validation helpers ───────────────────────────────────────────

/// Classifier for `required` enforcement.
///
/// A `required` field isn't satisfied by presence alone: for stringy fields an
/// empty string is still user-missing input, and for collection fields an
/// empty collection is the same as not providing it. This mirrors HTML form
/// `required` semantics and closes the class of bugs reported in n8n #21905
/// (required file field accepts empty submission).
fn is_absent_for_required(field: &Field, raw: Option<&FieldValue>) -> bool {
    let Some(value) = raw else { return true };
    match (field, value) {
        (_, FieldValue::Literal(serde_json::Value::Null)) => true,
        (
            Field::String(_) | Field::Secret(_) | Field::Code(_),
            FieldValue::Literal(serde_json::Value::String(s)),
        ) => s.is_empty(),
        (Field::File(f), FieldValue::Literal(serde_json::Value::String(s))) if !f.multiple => {
            s.is_empty()
        },
        (Field::File(f), FieldValue::Literal(serde_json::Value::Array(a))) if f.multiple => {
            a.is_empty()
        },
        (Field::File(f), FieldValue::List(items)) if f.multiple => items.is_empty(),
        (Field::List(_), FieldValue::List(items)) => items.is_empty(),
        (Field::List(_), FieldValue::Literal(serde_json::Value::Array(a))) => a.is_empty(),
        (Field::Select(s), FieldValue::List(items)) if s.multiple => items.is_empty(),
        (Field::Select(s), FieldValue::Literal(serde_json::Value::Array(a))) if s.multiple => {
            a.is_empty()
        },
        _ => false,
    }
}

/// Validate a single field against an optional raw value and a context.
///
/// Recurses for `Object`, `List`, and `Mode` containers.
fn validate_field(
    field: &Field,
    raw: Option<&FieldValue>,
    ctx: &dyn nebula_validator::RuleContext,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    // Visibility predicate — if hidden and absent, skip silently.
    let visible = match field.visible() {
        VisibilityMode::Always => true,
        VisibilityMode::Never => false,
        VisibilityMode::When(rule) => rule.evaluate(ctx),
    };
    if !visible && raw.is_none() {
        return;
    }

    // Required predicate.
    let required = match field.required() {
        RequiredMode::Never => false,
        RequiredMode::Always => true,
        RequiredMode::When(rule) => rule.evaluate(ctx),
    };
    if required && is_absent_for_required(field, raw) {
        report.push(
            ValidationError::builder("required")
                .at(path.clone())
                .message(format!("field `{path}` is required"))
                .build(),
        );
        return;
    }

    let Some(value) = raw else {
        return;
    };

    // Expression-mode enforcement.
    match (field.expression(), value) {
        (ExpressionMode::Forbidden, FieldValue::Expression(_)) => {
            report.push(
                ValidationError::builder("expression.forbidden")
                    .at(path.clone())
                    .message(format!("field `{path}` does not allow expression values"))
                    .build(),
            );
            return;
        },
        (_, FieldValue::Expression(expr)) => {
            // Expression is allowed or required here — attempt a parse so
            // obvious syntax errors are caught at validate-time.
            if let Err(e) = expr.parse() {
                report.push(e);
            }
            // Skip all value/type rules — expression not yet resolved.
            return;
        },
        _ => {},
    }

    // Value rules apply to literals only from this point on.
    validate_literal_value(field, value, ctx, path, report);
}

/// Type-check and rule-run a literal (non-expression) value.
#[expect(
    clippy::too_many_lines,
    reason = "field-type dispatch table — splitting into smaller fns reduces clarity"
)]
fn validate_literal_value(
    field: &Field,
    value: &FieldValue,
    ctx: &dyn nebula_validator::RuleContext,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    use crate::{
        Transformer,
        field::{ModeField, SelectField},
    };

    // Helper: apply transformers to a serde_json::Value.
    fn apply_transformers(transformers: &[Transformer], v: serde_json::Value) -> serde_json::Value {
        transformers.iter().fold(v, |cur, t| t.apply(&cur))
    }

    match field {
        Field::String(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(&f.transformers, lit.clone());
            if !transformed.is_string() {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string value"))
                        .build(),
                );
                return;
            }
            run_rules(field.rules(), &transformed, path, report);
        },
        Field::Secret(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(&f.transformers, lit.clone());
            if !transformed.is_string() {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string value"))
                        .build(),
                );
                return;
            }
            run_rules(field.rules(), &transformed, path, report);
        },
        Field::Code(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(&f.transformers, lit.clone());
            if !transformed.is_string() {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string value"))
                        .build(),
                );
                return;
            }
            run_rules(field.rules(), &transformed, path, report);
        },
        Field::Number(NumberField {
            integer,
            transformers,
            rules,
            ..
        }) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            let transformed = apply_transformers(transformers, lit.clone());
            let Some(num) = transformed.as_f64() else {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a numeric value"))
                        .build(),
                );
                return;
            };
            if *integer && num.fract() != 0.0 {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a whole number"))
                        .build(),
                );
                return;
            }
            run_rules(rules, &transformed, path, report);
        },
        Field::Boolean(_) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            if !lit.is_boolean() {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a boolean value"))
                        .build(),
                );
            }
        },
        Field::Select(SelectField {
            options,
            multiple,
            allow_custom,
            rules,
            transformers,
            ..
        }) => {
            // Accept both `Literal` (scalar or raw array) and `List` (typed).
            // `List` arises when `FieldValue::from_json` parsed the wire form
            // into the typed list variant — flatten to a JSON array for the
            // shape check below.
            //
            // Guard: an `Expression` element inside the list would otherwise
            // slip past `ExpressionMode::Forbidden` because `to_json` on
            // `FieldValue::Expression` emits a `{"$expr":"..."}` literal that
            // `resolve_value` later evaluates. Reject before flattening when
            // the field forbids expressions.
            let raw_json = match value {
                FieldValue::Literal(lit) => lit.clone(),
                FieldValue::List(items) => {
                    if matches!(field.expression(), ExpressionMode::Forbidden)
                        && items.iter().any(|v| matches!(v, FieldValue::Expression(_)))
                    {
                        report.push(
                            ValidationError::builder("expression.forbidden")
                                .at(path.clone())
                                .message(format!(
                                    "field `{path}` does not allow expression values in list items"
                                ))
                                .build(),
                        );
                        return;
                    }
                    serde_json::Value::Array(items.iter().map(FieldValue::to_json).collect())
                },
                _ => return,
            };
            let transformed = apply_transformers(transformers, raw_json);
            run_rules(rules, &transformed, path, report);
            if !allow_custom && !options.is_empty() {
                check_select_options(options, *multiple, &transformed, path, report);
            }
        },
        Field::List(ListField {
            min_items,
            max_items,
            item,
            rules,
            ..
        }) => {
            // FieldValue::List is the canonical typed form; Literal(Array) may occur
            // from raw JSON that wasn't fully parsed through FieldValue::from_json.
            let (item_count, items_typed): (usize, Option<&Vec<FieldValue>>) = match value {
                FieldValue::List(v) => (v.len(), Some(v)),
                FieldValue::Literal(serde_json::Value::Array(a)) => (a.len(), None),
                _ => {
                    report.push(
                        ValidationError::builder("type_mismatch")
                            .at(path.clone())
                            .message(format!("field `{path}` expects an array value"))
                            .build(),
                    );
                    return;
                },
            };
            let _ = rules; // rules on the list itself run via run_rules below if needed
            if let Some(min) = min_items
                && item_count < *min as usize
            {
                report.push(
                    ValidationError::builder("items.min")
                        .at(path.clone())
                        .param("min", serde_json::json!(min))
                        .param("actual", serde_json::json!(item_count))
                        .message(format!(
                            "field `{path}` requires at least {min} items, got {item_count}"
                        ))
                        .build(),
                );
            }
            if let Some(max) = max_items
                && item_count > *max as usize
            {
                report.push(
                    ValidationError::builder("items.max")
                        .at(path.clone())
                        .param("max", serde_json::json!(max))
                        .param("actual", serde_json::json!(item_count))
                        .message(format!(
                            "field `{path}` allows at most {max} items, got {item_count}"
                        ))
                        .build(),
                );
            }
            // Recurse into typed items when schema is present.
            if let (Some(item_field), Some(items_fv)) = (item.as_deref(), items_typed) {
                for (i, item_val) in items_fv.iter().enumerate() {
                    let item_path = path.clone().join(i);
                    validate_field(item_field, Some(item_val), ctx, &item_path, report);
                }
            }
        },
        Field::Object(ObjectField {
            fields: child_fields,
            transformers,
            rules,
            ..
        }) => {
            let FieldValue::Object(map) = value else {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects an object value"))
                        .build(),
                );
                return;
            };
            let sub_ctx = crate::context::ObjectContext(map);
            for child in child_fields {
                let child_path = path.clone().join(child.key().clone());
                validate_field(child, map.get(child.key()), &sub_ctx, &child_path, report);
            }
            let _ = (transformers, rules);
        },
        Field::Mode(ModeField {
            variants,
            default_variant,
            rules,
            ..
        }) => {
            let FieldValue::Mode {
                mode: mode_key,
                value: mode_value,
            } = value
            else {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!(
                            "field `{path}` expects a mode value ({{\"mode\": \"...\", ...}})"
                        ))
                        .build(),
                );
                return;
            };
            run_rules(rules, &value.to_json(), path, report);
            let resolved_key = Some(mode_key.as_str()).or(default_variant.as_deref());
            let Some(resolved_key) = resolved_key else {
                report.push(
                    ValidationError::builder("mode.required")
                        .at(path.clone())
                        .message(format!("field `{path}` requires a mode key"))
                        .build(),
                );
                return;
            };
            let Some(variant) = variants.iter().find(|v| v.key == resolved_key) else {
                report.push(
                    ValidationError::builder("mode.invalid")
                        .at(path.clone())
                        .param("mode", serde_json::Value::String(resolved_key.to_owned()))
                        .message(format!(
                            "field `{path}` has unknown mode variant `{resolved_key}`"
                        ))
                        .build(),
                );
                return;
            };
            {
                let payload_path = path
                    .clone()
                    .join(FieldKey::new("value").expect("static key"));
                validate_field(
                    &variant.field,
                    mode_value.as_deref(),
                    ctx,
                    &payload_path,
                    report,
                );
            }
        },
        // File, Computed, Dynamic, Notice — no type-check rule at schema time.
        Field::File(f) => {
            let FieldValue::Literal(lit) = value else {
                return;
            };
            if f.multiple {
                match lit.as_array() {
                    None => report.push(
                        ValidationError::builder("type_mismatch")
                            .at(path.clone())
                            .message(format!("field `{path}` expects an array of file paths"))
                            .build(),
                    ),
                    Some(items) => {
                        if items.iter().any(|v| !v.is_string()) {
                            report.push(
                                ValidationError::builder("type_mismatch")
                                    .at(path.clone())
                                    .message(format!(
                                        "field `{path}` expects an array of string file paths"
                                    ))
                                    .build(),
                            );
                        }
                    },
                }
            } else if !lit.is_string() {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string file path"))
                        .build(),
                );
            }
        },
        Field::Computed(_) | Field::Dynamic(_) | Field::Notice(_) => {},
    }
}

/// Validate a transformed value against a static option set.
///
/// Exhaustive over `(multiple, transformed.as_array())` so a scalar value
/// for a multi-select field or an array for a single-select field is
/// caught as `type_mismatch` instead of silently passing.
fn check_select_options(
    options: &[crate::SelectOption],
    multiple: bool,
    transformed: &serde_json::Value,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    match (multiple, transformed.as_array()) {
        (true, Some(arr)) => {
            for (i, v) in arr.iter().enumerate() {
                if !options.iter().any(|o| o.value == *v) {
                    report.push(
                        ValidationError::builder("option.invalid")
                            .at(path.clone())
                            .param("index", serde_json::json!(i))
                            .message(format!("field `{path}[{i}]` is not in allowed option set"))
                            .build(),
                    );
                }
            }
        },
        (true, None) => {
            report.push(
                ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message(format!(
                        "field `{path}` expects an array of option values (multiple select)"
                    ))
                    .build(),
            );
        },
        (false, Some(_)) => {
            report.push(
                ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message(format!(
                        "field `{path}` expects a single option value, got an array"
                    ))
                    .build(),
            );
        },
        (false, None) => {
            if !options.iter().any(|o| o.value == *transformed) {
                report.push(
                    ValidationError::builder("option.invalid")
                        .at(path.clone())
                        .message(format!("field `{path}` value is not in allowed option set"))
                        .build(),
                );
            }
        },
    }
}

/// Translate a raw validator error code to the STANDARD_CODES vocabulary.
///
/// The nebula-validator crate uses its own code names (e.g. `"min_length"`,
/// `"invalid_format"`). The schema crate STANDARD_CODES use a different
/// namespace (`"length.min"`, `"pattern"`, etc.). This function performs the
/// one-way mapping so that callers observe only STANDARD_CODES values.
fn translate_validator_code(
    raw_code: &str,
    params: &[(
        std::borrow::Cow<'static, str>,
        std::borrow::Cow<'static, str>,
    )],
) -> String {
    match raw_code {
        "min_length" => "length.min".to_owned(),
        "max_length" => "length.max".to_owned(),
        "min" => "range.min".to_owned(),
        "max" => "range.max".to_owned(),
        // "invalid_format" is emitted by Pattern, Email, and Url rules.
        // Pattern rule includes a "pattern" param; Email/Url set "expected" to "email"/"url".
        "invalid_format" => {
            let has_pattern_param = params.iter().any(|(k, _)| k.as_ref() == "pattern");
            if has_pattern_param {
                return "pattern".to_owned();
            }
            let expected = params
                .iter()
                .find(|(k, _)| k.as_ref() == "expected")
                .map(|(_, v)| v.as_ref());
            match expected {
                Some("email") => "email".to_owned(),
                Some("url") => "url".to_owned(),
                _ => "pattern".to_owned(),
            }
        },
        other => other.to_owned(),
    }
}

/// Apply a slice of rules to a JSON literal value, pushing errors into `report`.
fn run_rules(
    rules: &[nebula_validator::Rule],
    value: &serde_json::Value,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    use nebula_validator::ExecutionMode;
    if let Err(errs) = nebula_validator::validate_rules(value, rules, ExecutionMode::StaticOnly) {
        for e in errs.errors() {
            let raw_code: &str = e.code.as_ref();
            let msg: String = e.message.as_ref().to_owned();
            let code = translate_validator_code(raw_code, e.params());
            report.push(
                ValidationError::builder(code)
                    .at(path.clone())
                    .message(msg)
                    .build(),
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, Schema};

    #[test]
    fn clone_is_cheap_via_arc() {
        let s = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .build()
            .unwrap();
        let c = s.clone();
        assert!(Arc::ptr_eq(&s.0, &c.0));
    }

    #[test]
    fn find_returns_top_level() {
        let s = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .build()
            .unwrap();
        assert!(s.find(&FieldKey::new("x").unwrap()).is_some());
        assert!(s.find(&FieldKey::new("y").unwrap()).is_none());
    }
}
