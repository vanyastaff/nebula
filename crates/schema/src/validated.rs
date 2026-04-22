//! Validated schema handles — proof-tokens.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use zeroize::Zeroize;

use crate::{
    error::{Severity, ValidationError, ValidationReport},
    expression::ExpressionContext,
    field::{Field, ListField, NumberField, ObjectField},
    key::FieldKey,
    mode::{ExpressionMode, RequiredMode, VisibilityMode},
    path::FieldPath,
    secret::SecretValue,
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
    /// Rules evaluated once per validate call against the full value object
    /// (after per-field checks). Deferred rules are skipped in
    /// [`ExecutionMode::StaticOnly`](nebula_validator::ExecutionMode::StaticOnly).
    pub root_rules: Vec<nebula_validator::Rule>,
}

/// Proof-token: schema has been built and linted successfully.
///
/// Cheap to clone — backed by `Arc`.
///
/// Serde: serializes as `{"fields": [...]}` when there are no root rules, or
/// `{"fields": [...], "root_rules": [...]}` when [`ValidSchemaInner::root_rules`]
/// is non-empty. Deserialization rebuilds through [`SchemaBuilder`](crate::schema::SchemaBuilder);
/// invalid wire data returns a [`serde::de::Error`] (lint failures are not panics).
#[derive(Debug, Clone)]
pub struct ValidSchema(pub(crate) Arc<ValidSchemaInner>);

impl PartialEq for ValidSchema {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.fields == other.0.fields && self.0.root_rules == other.0.root_rules)
    }
}

impl Eq for ValidSchema {}

impl Serialize for ValidSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.0.root_rules.is_empty() {
            let mut s = serializer.serialize_struct("ValidSchema", 1)?;
            s.serialize_field("fields", &self.0.fields)?;
            s.end()
        } else {
            let mut s = serializer.serialize_struct("ValidSchema", 2)?;
            s.serialize_field("fields", &self.0.fields)?;
            s.serialize_field("root_rules", &self.0.root_rules)?;
            s.end()
        }
    }
}

impl<'de> Deserialize<'de> for ValidSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Transparent wrapper that mirrors `Schema`'s serde representation.
        #[derive(Deserialize)]
        struct ValidSchemaRepr {
            #[serde(default)]
            fields: Vec<Field>,
            #[serde(default)]
            root_rules: Vec<nebula_validator::Rule>,
        }
        let repr = ValidSchemaRepr::deserialize(deserializer)?;
        let mut b = repr.fields.into_iter().fold(
            crate::schema::SchemaBuilder::default(),
            super::schema::SchemaBuilder::add,
        );
        for rule in repr.root_rules {
            b = b.root_rule(rule);
        }
        b.build()
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
                    root_rules: Vec::new(),
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

    /// Schema-level rules run after per-field validation (see [`ValidSchema::validate`]).
    #[must_use]
    pub fn root_rules(&self) -> &[nebula_validator::Rule] {
        &self.0.root_rules
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

        run_root_rules(&self.0.root_rules, values, &mut report);

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
    /// `ctx`, then **promote** `Field::Secret` string literals to
    /// `FieldValue::SecretLiteral` (and optional KDF) before the final
    /// schema-validate pass.
    ///
    /// **Expression fast path:** when `schema.flags().uses_expressions == false`,
    /// expression resolution is skipped; **secret promotion still runs** so
    /// `ResolvedValues` is consistent for secret fields.
    ///
    /// After evaluating each expression the tree is re-validated on resolved
    /// literals. If any expression evaluation, KDF, or post-resolve type/rule
    /// check fails, errors are returned as a [`ValidationReport`].
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
        let mut report = ValidationReport::new();
        let mut values = self.values;
        let mut resolved_expression_paths: HashSet<FieldPath> = HashSet::new();

        if self.schema.flags().uses_expressions {
            // Walk and resolve every entry in the flat top-level map.
            let keys: Vec<FieldKey> = values.iter().map(|(k, _)| k.clone()).collect();
            for key in keys {
                let Some(value) = values.get(&key).cloned() else {
                    continue;
                };
                let path = FieldPath::root().join(key.clone());
                let resolved = resolve_value(
                    value,
                    ctx,
                    &path,
                    &mut report,
                    &mut resolved_expression_paths,
                )
                .await;
                values.set(key, resolved);
            }

            if report.has_errors() {
                return Err(report);
            }
        }

        for field in self.schema.fields() {
            let path = FieldPath::root().join(field.key().clone());
            if let Some(v) = values.get_mut(field.key()) {
                promote_secrets_in_value(field, v, &path, &mut report);
            }
        }

        if report.has_errors() {
            return Err(report);
        }

        // Re-run schema validation on resolved + promoted literals. Any type
        // mismatches at paths produced by expression evaluation are surfaced
        // as `expression.type_mismatch`.
        let resolve_warnings: Vec<ValidationError> = report
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();
        let post_resolve_warnings: Vec<ValidationError> = match self.schema.validate(&values) {
            Ok(post_resolve_valid) => post_resolve_valid.warnings().to_vec(),
            Err(mut post_resolve_report) => {
                post_resolve_report.extend(self.warnings.iter().cloned());
                post_resolve_report.extend(resolve_warnings.iter().cloned());
                return Err(remap_expression_type_mismatch(
                    post_resolve_report,
                    &resolved_expression_paths,
                ));
            },
        };

        let all_warnings: Arc<[ValidationError]> = self
            .warnings
            .iter()
            .chain(resolve_warnings.iter())
            .chain(post_resolve_warnings.iter())
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

    /// Look up a resolved **non-secret** literal by key.
    ///
    /// Returns `None` if the key is missing, the value is not a JSON literal, or
    /// the field is a [`Field::Secret`] (use [`Self::get_secret`](Self::get_secret) instead).
    pub fn get(&self, key: &FieldKey) -> Option<&serde_json::Value> {
        if matches!(self.schema.find(key), Some(Field::Secret(_))) {
            return None;
        }
        match self.values.get(key)? {
            FieldValue::Literal(v) => Some(v),
            FieldValue::SecretLiteral(_) => None,
            _ => None,
        }
    }

    /// Borrow the secret material for a `Field::Secret` key, if present.
    pub fn get_secret(&self, key: &FieldKey) -> Option<&SecretValue> {
        if !matches!(self.schema.find(key), Some(Field::Secret(_))) {
            return None;
        }
        match self.values.get(key)? {
            FieldValue::SecretLiteral(s) => Some(s),
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

/// Shape label for validation errors (no `Debug` of full values — avoids leaking
/// secret-adjacent subtrees into messages).
fn field_value_shape_for_errors(v: &FieldValue) -> &'static str {
    use serde_json::Value;
    match v {
        FieldValue::Literal(val) => match val {
            Value::Null => "literal(null)",
            Value::Bool(_) => "literal(bool)",
            Value::Number(_) => "literal(number)",
            Value::String(_) => "literal(string)",
            Value::Array(_) => "literal(array)",
            Value::Object(_) => "literal(object)",
        },
        FieldValue::Expression(_) => "expression",
        FieldValue::Object(_) => "object",
        FieldValue::List(_) => "list",
        FieldValue::Mode { .. } => "mode",
        FieldValue::SecretLiteral(_) => "secret_literal",
    }
}

/// Promote string literals to [`FieldValue::SecretLiteral`] for secret fields, invoking KDFs when
/// configured. Recurses through object/list/mode containers.
fn promote_secrets_in_value(
    field: &Field,
    value: &mut FieldValue,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    use serde_json::Value;
    match (field, &mut *value) {
        (Field::Secret(secret), FieldValue::Literal(Value::String(s))) => {
            let mut password = std::mem::take(s);
            *value = if let Some(kdf) = &secret.kdf {
                match kdf.hash_password(password.as_bytes()) {
                    Ok(sv) => {
                        password.zeroize();
                        FieldValue::SecretLiteral(sv)
                    },
                    Err(e) => {
                        *s = password;
                        report.push(
                            ValidationError::builder("secret.kdf")
                                .at(path.clone())
                                .message(e.to_string())
                                .build(),
                        );
                        return;
                    },
                }
            } else {
                FieldValue::SecretLiteral(SecretValue::string(password))
            };
        },
        (Field::Secret(_), FieldValue::Literal(_)) => {
            report.push(
                ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message("secret field value must be a string")
                    .build(),
            );
        },
        (Field::Secret(_), FieldValue::SecretLiteral(_)) => {},
        (Field::Secret(_), FieldValue::Expression(_)) => {
            report.push(
                ValidationError::builder("expression.unresolved")
                    .at(path.clone())
                    .message(
                        "secret field still has an expression value at resolve time".to_owned(),
                    )
                    .build(),
            );
        },
        (Field::Secret(_), v) => {
            let shape = field_value_shape_for_errors(v);
            report.push(
                ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message(format!(
                        "secret field has incompatible value shape: {shape}"
                    ))
                    .build(),
            );
        },
        (Field::Object(obj), FieldValue::Object(map)) => {
            for ch in &obj.fields {
                if let Some(v) = map.get_mut(ch.key()) {
                    let p = path.clone().join(ch.key().clone());
                    promote_secrets_in_value(ch, v, &p, report);
                }
            }
        },
        (Field::List(list), FieldValue::List(items)) => {
            if let Some(item_field) = list.item.as_deref() {
                for (i, v) in items.iter_mut().enumerate() {
                    let p = path.clone().join(i);
                    promote_secrets_in_value(item_field, v, &p, report);
                }
            }
        },
        (
            Field::Mode(mode),
            FieldValue::Mode {
                mode: mode_key,
                value: Some(mv),
            },
        ) => {
            let Some(var) = mode.variants.iter().find(|v| v.key == mode_key.as_str()) else {
                return;
            };
            let k = match FieldKey::new("value") {
                Ok(k) => k,
                Err(e) => {
                    report.push(
                        ValidationError::builder("invalid_key")
                            .at(path.clone())
                            .message(format!(
                                "invalid static mode payload key `value`: {}",
                                e.message
                            ))
                            .build(),
                    );
                    return;
                },
            };
            let p = path.clone().join(k);
            promote_secrets_in_value(&var.field, mv.as_mut(), &p, report);
        },
        _ => {},
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
    resolved_expression_paths: &'v mut HashSet<FieldPath>,
) -> Pin<Box<dyn Future<Output = FieldValue> + 'v>> {
    Box::pin(async move {
        match value {
            FieldValue::Expression(ref expr) => {
                match expr.parse_at(path) {
                    Ok(ast) => match ctx.evaluate(ast).await {
                        Ok(v) => {
                            resolved_expression_paths.insert(path.clone());
                            FieldValue::Literal(v)
                        },
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
                    Err(e) => {
                        report.push(e);
                        FieldValue::Literal(serde_json::Value::Null)
                    },
                }
            },
            FieldValue::Object(map) => {
                let mut out = IndexMap::with_capacity(map.len());
                for (k, v) in map {
                    let child_path = path.clone().join(k.clone());
                    let resolved =
                        resolve_value(v, ctx, &child_path, report, resolved_expression_paths).await;
                    out.insert(k, resolved);
                }
                FieldValue::Object(out)
            },
            FieldValue::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (i, v) in items.into_iter().enumerate() {
                    let item_path = path.clone().join(i);
                    let resolved =
                        resolve_value(v, ctx, &item_path, report, resolved_expression_paths).await;
                    out.push(resolved);
                }
                FieldValue::List(out)
            },
            FieldValue::Mode { mode, value } => {
                let resolved_value = if let Some(inner) = value {
                    let Ok(payload_key) = FieldKey::new("value") else {
                        return FieldValue::Mode {
                            mode,
                            value: Some(inner),
                        };
                    };
                    let inner_path = path.clone().join(payload_key);
                    let resolved =
                        resolve_value(*inner, ctx, &inner_path, report, resolved_expression_paths)
                            .await;
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

fn remap_expression_type_mismatch(
    report: ValidationReport,
    expression_paths: &HashSet<FieldPath>,
) -> ValidationReport {
    let remapped = report.into_iter().map(|mut issue| {
        let from_expression = expression_paths
            .iter()
            .any(|expression_path| issue.path.starts_with(expression_path));
        if issue.code == "type_mismatch" && from_expression {
            issue.code = "expression.type_mismatch".into();
        }
        issue
    });

    let mut out = ValidationReport::new();
    out.extend(remapped);
    out
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
        (Field::Secret(_), FieldValue::SecretLiteral(sv)) => sv.is_empty(),
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
    if let FieldValue::Expression(expr) = value {
        match field.expression() {
            ExpressionMode::Forbidden => {
                report.push(
                    ValidationError::builder("expression.forbidden")
                        .at(path.clone())
                        .message(format!("field `{path}` does not allow expression values"))
                        .build(),
                );
            },
            ExpressionMode::Allowed | ExpressionMode::Required => {
                // Expression is allowed/required here — parse eagerly so syntax
                // errors surface at validate-time.
                if let Err(e) = expr.parse_at(path) {
                    report.push(e);
                }
            },
        }
        // Skip all value/type rules — expression not yet resolved.
        return;
    }

    if matches!(field.expression(), ExpressionMode::Required) {
        report.push(
            ValidationError::builder("expression.required")
                .at(path.clone())
                .message(format!("field `{path}` requires an expression value"))
                .build(),
        );
        return;
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
            if let FieldValue::SecretLiteral(sv) = value {
                let v_for_rules = match sv {
                    SecretValue::String(st) => {
                        let transformed = apply_transformers(
                            &f.transformers,
                            serde_json::Value::String(st.expose().to_owned()),
                        );
                        if !transformed.is_string() {
                            report.push(
                                ValidationError::builder("type_mismatch")
                                    .at(path.clone())
                                    .message(format!("field `{path}` expects a string value"))
                                    .build(),
                            );
                            return;
                        }
                        transformed
                    },
                    SecretValue::Bytes(b) => {
                        let transformed = apply_transformers(
                            &f.transformers,
                            serde_json::Value::String(hex::encode(b.expose())),
                        );
                        if !transformed.is_string() {
                            report.push(
                                ValidationError::builder("type_mismatch")
                                    .at(path.clone())
                                    .message(format!("field `{path}` expects a string value"))
                                    .build(),
                            );
                            return;
                        }
                        transformed
                    },
                };
                run_rules(field.rules(), &v_for_rules, path, report);
                return;
            }
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
            unique,
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
            if *unique {
                let duplicate_index = if let Some(items_fv) = items_typed {
                    first_duplicate_index(items_fv.iter().map(FieldValue::to_json))
                } else if let FieldValue::Literal(serde_json::Value::Array(arr)) = value {
                    first_duplicate_index(arr.iter().cloned())
                } else {
                    None
                };
                if let Some(idx) = duplicate_index {
                    report.push(
                        ValidationError::builder("items.unique")
                            .at(path.clone().join(idx))
                            .param("index", serde_json::json!(idx))
                            .message(format!(
                                "field `{path}` requires unique items; duplicate found at index {idx}"
                            ))
                            .build(),
                    );
                }
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
                let Ok(payload_key) = FieldKey::new("value") else {
                    return;
                };
                let payload_path = path.clone().join(payload_key);
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
            if f.multiple {
                match value {
                    FieldValue::Literal(lit) => match lit.as_array() {
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
                    },
                    FieldValue::List(items) => {
                        if items.iter().any(|v| {
                            !matches!(v, FieldValue::Literal(serde_json::Value::String(_)))
                        }) {
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
                    _ => report.push(
                        ValidationError::builder("type_mismatch")
                            .at(path.clone())
                            .message(format!("field `{path}` expects an array of file paths"))
                            .build(),
                    ),
                }
            } else if !matches!(value, FieldValue::Literal(serde_json::Value::String(_))) {
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

fn first_duplicate_index(values: impl IntoIterator<Item = serde_json::Value>) -> Option<usize> {
    let mut seen: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for (idx, value) in values.into_iter().enumerate() {
        // serde_json::Value does not implement Hash. Bucket by serialized form,
        // then confirm equality within the bucket to preserve exact semantics.
        let key = serde_json::to_string(&value)
            .unwrap_or_else(|_| format!("__fallback_non_serializable__:{value:?}"));
        if let Some(bucket) = seen.get_mut(&key) {
            if bucket.iter().any(|prior| prior == &value) {
                return Some(idx);
            }
            bucket.push(value);
        } else {
            seen.insert(key, vec![value]);
        }
    }
    None
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
        push_validator_rule_errors(errs, path, report);
    }
}

/// Run schema-level rules against the full submitted JSON object.
fn run_root_rules(
    rules: &[nebula_validator::Rule],
    values: &FieldValues,
    report: &mut ValidationReport,
) {
    use nebula_validator::{ExecutionMode, PredicateContext};

    if rules.is_empty() {
        return;
    }

    let json = values.to_json();
    let pred_ctx = PredicateContext::from_json(&json);
    if let Err(errs) = nebula_validator::validate_rules_with_ctx(
        &json,
        rules,
        Some(&pred_ctx),
        ExecutionMode::StaticOnly,
    ) {
        push_validator_rule_errors(errs, &FieldPath::root(), report);
    }
}

fn push_validator_rule_errors(
    errs: nebula_validator::foundation::ValidationErrors,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
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

    #[test]
    fn find_by_path_handles_nested_object_and_mode_variant() {
        let schema = Schema::builder()
            .add(Field::object(FieldKey::new("user").unwrap()).add(Field::string("email")))
            .add(Field::mode(FieldKey::new("auth").unwrap()).variant(
                "token",
                "Token",
                Field::string("value"),
            ))
            .build()
            .unwrap();

        assert!(
            schema
                .find_by_path(&FieldPath::parse("user.email").unwrap())
                .is_some()
        );
        assert!(
            schema
                .find_by_path(&FieldPath::parse("auth.token").unwrap())
                .is_some()
        );
        assert!(
            schema
                .find_by_path(&FieldPath::parse("user.missing").unwrap())
                .is_none()
        );
    }

    #[test]
    fn root_rule_runs_after_fields() {
        use nebula_validator::{Predicate, Rule};
        use serde_json::json;

        let schema = Schema::builder()
            .add(Field::string(FieldKey::new("tier").unwrap()))
            .root_rule(Rule::predicate(
                Predicate::eq("tier", json!("pro")).unwrap(),
            ))
            .build()
            .unwrap();

        let bad = FieldValues::from_json(json!({"tier": "free"})).unwrap();
        assert!(schema.validate(&bad).is_err());

        let ok = FieldValues::from_json(json!({"tier": "pro"})).unwrap();
        assert!(schema.validate(&ok).is_ok());
    }

    #[test]
    fn valid_schema_serde_roundtrips_root_rules() {
        use nebula_validator::{Predicate, Rule};
        use serde_json::json;

        let schema = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .root_rule(Rule::predicate(Predicate::eq("x", json!("a")).unwrap()))
            .build()
            .unwrap();

        let wire = serde_json::to_value(&schema).unwrap();
        let back: ValidSchema = serde_json::from_value(wire).unwrap();
        assert_eq!(schema.root_rules(), back.root_rules());
        assert_eq!(schema.fields().len(), back.fields().len());
    }
}
