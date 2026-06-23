//! Validated schema handles — proof-tokens.

use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock},
};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{
    error::{Severity, ValidationError, ValidationReport},
    expression::ExpressionContext,
    field::{Field, ListField, NumberField, ObjectField},
    key::FieldKey,
    loader::{LoaderContext, LoaderRegistry, LoaderResult},
    mode::ExpressionMode,
    option::SelectOption,
    path::FieldPath,
    schema::{
        resolve_dynamic_loader_key, resolve_dynamic_loader_path, resolve_select_loader_key,
        resolve_select_loader_path,
    },
    secret::SecretValue,
    value::{FieldValue, FieldValues},
};

/// `Mode` payload uses two well-known nested keys: `"mode"` (variant
/// selector) and `"value"` (variant payload). Both are interned via
/// `LazyLock` so per-call `FieldKey::new` does not re-allocate the
/// `Arc<str>` every time `validate_field` / `resolve_value` /
/// `promote_secrets_in_value` recurses through a `Field::Mode`.
pub(crate) static MODE_SELECTOR_KEY: LazyLock<FieldKey> =
    LazyLock::new(|| FieldKey::new("mode").expect("static mode selector key is valid"));
pub(crate) static MODE_PAYLOAD_KEY: LazyLock<FieldKey> =
    LazyLock::new(|| FieldKey::new("value").expect("static mode payload key is valid"));

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

/// Whether a schema describes a concrete record of typed fields, or the
/// gradual-typing `Any` whose shape is unknown.
///
/// `()` and a `#[derive(Schema)]` struct (even one with zero fields) are
/// [`Record`](SchemaKind::Record); `serde_json::Value` / [`FieldValues`] — inputs
/// that advertise no fixed shape — are [`Any`](SchemaKind::Any). The two were
/// previously indistinguishable (both an empty schema), which let an `Any` and an
/// empty record compare equal and collapse in the type-DAG (the Top/Bottom
/// collapse). Keeping them apart is the foundation the assignability lattice
/// builds on: an `Any` producer is gradually permissive, while a `Record`
/// producer that emits no fields does *not* satisfy a consumer that requires them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SchemaKind {
    /// A concrete record of typed fields (a derived struct, an empty struct, `()`).
    #[default]
    Record,
    /// Gradual-typing `Any` — the shape is unknown (`serde_json::Value`).
    Any,
}

/// Shared interior of a `ValidSchema`.
///
/// An implementation detail: the only constructor is the crate-private
/// `ValidSchema::from_inner`, so this struct is never built or matched outside
/// `nebula-schema`. `#[non_exhaustive]` records that intent and lets new fields
/// (like `kind`) be added without it being an external breaking change.
#[derive(Debug)]
#[non_exhaustive]
pub struct ValidSchemaInner {
    /// Whether this is a concrete record or the gradual-typing `Any`.
    pub kind: SchemaKind,
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
/// Serde: a [`SchemaKind::Record`] serializes as `{"fields": [...]}` (plus
/// `"root_rules": [...]` when [`ValidSchemaInner::root_rules`] is non-empty) —
/// no `kind` tag, so the wire shape is identical to before `kind` existed and a
/// payload with a missing `kind` deserializes back as a record. A
/// [`SchemaKind::Any`] serializes as `{"kind": "any", "fields": []}`.
/// Deserialization rebuilds a record through [`SchemaBuilder`](crate::schema::SchemaBuilder)
/// (invalid wire data returns a [`serde::de::Error`]; lint failures are not
/// panics), and **fails closed** if a payload tagged `kind: "any"` carries any
/// `fields`/`root_rules` instead of silently dropping those constraints.
#[derive(Debug, Clone)]
pub struct ValidSchema(pub(crate) Arc<ValidSchemaInner>);

impl PartialEq for ValidSchema {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.kind == other.0.kind
                && self.0.fields == other.0.fields
                && self.0.root_rules == other.0.root_rules)
    }
}

impl Eq for ValidSchema {}

impl Serialize for ValidSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        // `kind` is emitted only for `Any` (the non-default), so every existing
        // `Record` schema keeps its `{"fields": [...]}` wire shape and a reader
        // that pre-dates `kind` still round-trips it as a record.
        let emit_kind = self.0.kind != SchemaKind::Record;
        let has_rules = !self.0.root_rules.is_empty();
        let len = 1 + usize::from(emit_kind) + usize::from(has_rules);
        let mut s = serializer.serialize_struct("ValidSchema", len)?;
        if emit_kind {
            s.serialize_field("kind", &self.0.kind)?;
        }
        s.serialize_field("fields", &self.0.fields)?;
        if has_rules {
            s.serialize_field("root_rules", &self.0.root_rules)?;
        }
        s.end()
    }
}

impl<'de> Deserialize<'de> for ValidSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Transparent wrapper that mirrors `Schema`'s serde representation.
        #[derive(Deserialize)]
        struct ValidSchemaRepr {
            /// Defaults to [`SchemaKind::Record`], so wire data that pre-dates
            /// `kind` (`{"fields": [...]}`) round-trips as a record.
            #[serde(default)]
            kind: SchemaKind,
            #[serde(default)]
            fields: Vec<Field>,
            #[serde(default)]
            root_rules: Vec<nebula_validator::Rule>,
        }
        let repr = ValidSchemaRepr::deserialize(deserializer)?;
        if repr.kind == SchemaKind::Any {
            // Fail closed: an `Any` schema carries no constraints. A payload
            // tagged `Any` that also lists `fields`/`root_rules` is malformed
            // (mistagged, or from a mixed-version producer); silently returning
            // the unconstrained `any()` would drop every constraint and make
            // validation fully permissive. Reject it instead.
            if !repr.fields.is_empty() || !repr.root_rules.is_empty() {
                return Err(serde::de::Error::custom(
                    "schema tagged `kind: \"any\"` must not carry `fields` or `root_rules`",
                ));
            }
            return Ok(Self::any());
        }
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
                    kind: SchemaKind::Record,
                    fields: Vec::new(),
                    index: IndexMap::new(),
                    flags: SchemaFlags::default(),
                    root_rules: Vec::new(),
                })
            })
            .clone()
    }

    /// Shared gradual-typing `Any` `ValidSchema` — cheap `Arc` clone.
    ///
    /// Use this for inputs whose shape is unknown at the type level
    /// (`serde_json::Value`, [`FieldValues`], primitives that carry no record
    /// structure). It is field-less like [`empty`](Self::empty),
    /// but its [`kind`](Self::kind) is [`SchemaKind::Any`], so the assignability
    /// lattice treats it as the gradual `Any` rather than an empty record —
    /// keeping the two from collapsing into one another in the type-DAG.
    pub fn any() -> Self {
        use std::sync::OnceLock;
        static ANY: OnceLock<ValidSchema> = OnceLock::new();
        ANY.get_or_init(|| {
            Self::from_inner(ValidSchemaInner {
                kind: SchemaKind::Any,
                fields: Vec::new(),
                index: IndexMap::new(),
                flags: SchemaFlags::default(),
                root_rules: Vec::new(),
            })
        })
        .clone()
    }

    /// Whether this schema is a concrete [`Record`](SchemaKind::Record) or the
    /// gradual-typing [`Any`](SchemaKind::Any).
    #[must_use]
    pub fn kind(&self) -> SchemaKind {
        self.0.kind
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
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.0.fields
    }

    /// Borrow the build-time flags.
    #[must_use]
    pub fn flags(&self) -> &SchemaFlags {
        &self.0.flags
    }

    /// Schema-level rules run after per-field validation (see [`ValidSchema::validate`]).
    #[must_use]
    pub fn root_rules(&self) -> &[nebula_validator::Rule] {
        &self.0.root_rules
    }

    /// Find a top-level field by key.
    #[must_use]
    pub fn find(&self, key: &FieldKey) -> Option<&Field> {
        self.0.fields.iter().find(|f| f.key() == key)
    }

    /// Find a field by dotted path using the O(1) index.
    #[must_use]
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

    /// Resolve dynamic options for a select field through loader registry.
    ///
    /// Same error taxonomy as [`crate::Schema::load_select_options`], but this
    /// entrypoint guarantees validated schema invariants.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key`, `loader.not_registered`, or `loader.failed`.
    pub async fn load_select_options(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let loader_key = resolve_select_loader_key(self.fields(), key)?;
        registry.load_options(&loader_key, context).await
    }

    /// Resolve dynamic options for a select field at a nested schema path.
    ///
    /// Uses the same schema-path addressing rules as
    /// [`crate::Schema::load_select_options_at`].
    ///
    /// # Errors
    ///
    /// Returns `field.not_found`, `field.type_mismatch`, `loader.missing_config`,
    /// `loader.not_registered`, or `loader.failed`.
    pub async fn load_select_options_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let loader_key = resolve_select_loader_path(self.fields(), path)?;
        registry.load_options(&loader_key, context).await
    }

    /// Resolve dynamic record payloads for a dynamic field through registry.
    ///
    /// Same error taxonomy as [`crate::Schema::load_dynamic_records`], but this
    /// entrypoint guarantees validated schema invariants.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key`, `loader.not_registered`, or `loader.failed`.
    pub async fn load_dynamic_records(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<serde_json::Value>, ValidationError> {
        let loader_key = resolve_dynamic_loader_key(self.fields(), key)?;
        registry.load_records(&loader_key, context).await
    }

    /// Resolve dynamic record payloads for a field at a nested schema path.
    ///
    /// Uses the same schema-path addressing rules as
    /// [`crate::Schema::load_dynamic_records_at`].
    ///
    /// # Errors
    ///
    /// Returns `field.not_found`, `field.type_mismatch`, `loader.missing_config`,
    /// `loader.not_registered`, or `loader.failed`.
    pub async fn load_dynamic_records_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<serde_json::Value>, ValidationError> {
        let loader_key = resolve_dynamic_loader_path(self.fields(), path)?;
        registry.load_records(&loader_key, context).await
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
    #[tracing::instrument(
        level = "debug",
        target = "nebula_schema::validate",
        skip(self, values),
        fields(
            field_count = self.0.fields.len(),
            has_root_rules = !self.0.root_rules.is_empty(),
        )
    )]
    pub fn validate(&self, values: &FieldValues) -> Result<ValidValues, ValidationReport> {
        use crate::context::predicate_context_for;

        let mut report = ValidationReport::new();

        // One whole-tree predicate context. Built once and shared across all
        // nesting levels: a nested field's `When` may reference a sibling at
        // any depth via an absolute RFC-6901 pointer, so per-level rebuilds
        // would make cross-level predicates fail open.
        let ctx = predicate_context_for(&self.0.fields, values);

        // Top-level fields are the first field-set level. Visibility/required
        // for every level (this one and each nested object/list/mode level)
        // goes through the single policy resolver in `gate_and_validate_level`.
        let entries: Vec<LevelEntry<'_>> = self
            .0
            .fields
            .iter()
            .map(|field| {
                let schema_path = FieldPath::root().join(field.key().clone());
                let validator_path = validator_path_from_schema_path(&schema_path);
                LevelEntry {
                    field,
                    raw: values.get(field.key()),
                    schema_path,
                    validator_path,
                }
            })
            .collect();
        gate_and_validate_level(&entries, &ctx, &mut report);

        // Schema-level rules run against the full submission
        // (`values.to_json()`), but the predicate context is the
        // secret-scrubbed `root_predicate_context_for`: a value-comparing root
        // predicate can never read a `Field::Secret` plaintext, while legal
        // non-secret nested predicates still resolve (no fail-open).
        if !self.0.root_rules.is_empty() {
            let json = values.to_json();
            let pred_ctx = crate::context::root_predicate_context_from_json(&self.0.fields, &json);
            if let Err(errs) = nebula_validator::validate_rules_with_ctx(
                &json,
                &self.0.root_rules,
                Some(&pred_ctx),
                nebula_validator::ExecutionMode::StaticOnly,
            ) {
                merge_validator_errors(&errs, &FieldPath::root(), &mut report);
            }
        }

        if report.has_errors() {
            tracing::warn!(
                target: "nebula_schema::validate",
                error_count = report.errors().count(),
                "validate produced errors"
            );
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
/// Produced by `ValidSchema::validate()`. Proof-token that values
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
    #[must_use]
    pub const fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Borrow the raw value tree.
    #[must_use]
    pub const fn raw(&self) -> &FieldValues {
        &self.values
    }

    /// Borrow the raw value tree (alias for [`raw`](Self::raw)).
    #[deprecated(note = "use raw() instead")]
    #[must_use]
    pub const fn raw_values(&self) -> &FieldValues {
        &self.values
    }

    /// Iterate validation warnings that were non-fatal.
    #[must_use]
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a top-level value by key.
    #[must_use]
    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> {
        self.values.get(key)
    }

    /// Look up a value by dotted path.
    #[must_use]
    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> {
        self.values.get_path(path)
    }

    /// Resolve all `FieldValue::Expression` entries by evaluating them through
    /// `ctx`, then **promote** `Field::Secret` string literals to
    /// `FieldValue::SecretLiteral` before the final schema-validate pass.
    ///
    /// > **Status: latent.** This seam is structurally complete and test-proven
    /// > (the proof-token chain `ValidSchema → ValidValues → ResolvedValues` is
    /// > enforced by the types), but has **no production consumer** in this
    /// > version: no crate calls `resolve` or implements [`ExpressionContext`]
    /// > outside tests and examples. It becomes load-bearing only when
    /// > action-input expressions move from the test pipeline into the engine.
    /// > Until then its behavior under a real evaluator — performance, error
    /// > taxonomy, cancellation interplay — is unproven against production load.
    ///
    /// **Expression fast path:** when `schema.flags().uses_expressions == false`,
    /// expression resolution is skipped; **secret promotion still runs** so
    /// `ResolvedValues` is consistent for secret fields.
    ///
    /// After evaluating each expression the tree is re-validated on resolved
    /// literals. If any expression evaluation or post-resolve type/rule check
    /// fails, errors are returned as a [`ValidationReport`].
    ///
    /// # Errors
    ///
    /// Returns `Err(ValidationReport)` when any expression evaluation fails
    /// or when a resolved value violates a field rule.
    ///
    /// # Cancellation
    ///
    /// cancel-safe: yes. `resolve` performs no external side effects of its own
    /// — it rewrites an owned in-memory value tree and accumulates a report — so
    /// dropping the future at any `.await` discards the partially-resolved tree
    /// with nothing persisted. The only `.await` points are calls into the
    /// caller-supplied [`ExpressionContext::evaluate`]; any side effects there
    /// are that impl's responsibility (see the trait's cancel-safety note).
    #[tracing::instrument(
        level = "debug",
        target = "nebula_schema::resolve",
        skip(self, ctx),
        fields(
            uses_expressions = self.schema.flags().uses_expressions,
            field_count = self.schema.fields().len(),
        )
    )]
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
            return Err(remap_expression_type_mismatch(
                report,
                &resolved_expression_paths,
            ));
        }

        // Re-run schema validation on resolved + promoted literals. Any type
        // mismatches at paths produced by expression evaluation are surfaced
        // as `expression.type_mismatch`.
        //
        // Fast path: if no expressions were resolved AND the schema doesn't
        // declare any expression-bearing fields, the values are unchanged
        // since the prior `validate()` call (the one that produced `self`).
        // Skip the second walk in that case — it amounts to ~50% of the
        // wall-clock cost of `resolve()` on schemas without expressions.
        let resolve_warnings: Vec<ValidationError> = report
            .iter()
            .filter(|e| e.severity == Severity::Warning)
            .cloned()
            .collect();

        let skip_revalidate = resolved_expression_paths.is_empty()
            && !self.schema.flags().uses_expressions
            && resolve_warnings.is_empty();

        let post_resolve_warnings: Vec<ValidationError> = if skip_revalidate {
            tracing::trace!(
                target: "nebula_schema::resolve",
                "skipping post-resolve revalidate (no expressions / warnings)"
            );
            Vec::new()
        } else {
            tracing::debug!(
                target: "nebula_schema::resolve",
                resolved_expression_paths = resolved_expression_paths.len(),
                "running post-resolve revalidate"
            );
            match self.schema.validate(&values) {
                Ok(post_resolve_valid) => post_resolve_valid.warnings().to_vec(),
                Err(mut post_resolve_report) => {
                    post_resolve_report.extend(self.warnings.iter().cloned());
                    post_resolve_report.extend(resolve_warnings.iter().cloned());
                    return Err(remap_expression_type_mismatch(
                        post_resolve_report,
                        &resolved_expression_paths,
                    ));
                },
            }
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
/// Produced by `ValidValues::resolve()`. Proof-token that no
/// expression placeholders remain in the value tree.
///
/// > **Status: latent.** Only [`ValidValues::resolve`](crate::ValidValues::resolve)
/// > constructs this, and that seam has no production consumer yet (see its status
/// > note) — so this proof-token, while sound, is exercised only by tests and
/// > examples today.
///
/// Owns an `Arc`-backed clone of the schema so it is freely `Send + 'static`
/// and safe to persist or hand off to runtime.
#[derive(Debug, Clone)]
pub struct ResolvedValues {
    pub(crate) schema: ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

/// Typed lookup result for [`ResolvedValues`] accessors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedLookup<'a> {
    /// Field is not present in the value map.
    Missing,
    /// Field is present as a non-secret JSON literal.
    Literal(&'a serde_json::Value),
    /// Field is present as secret material.
    Secret(&'a SecretValue),
    /// Field is present but not a plain JSON literal (object/list/mode).
    Complex(&'a FieldValue),
}

impl ResolvedValues {
    /// Borrow the schema these values were resolved against.
    #[must_use]
    pub const fn schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Borrow the resolved value tree.
    ///
    /// Guaranteed to contain no `FieldValue::Expression` variants after
    /// successful resolution.
    #[must_use]
    pub const fn values(&self) -> &FieldValues {
        &self.values
    }

    /// Iterate resolution warnings.
    #[must_use]
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a resolved **non-secret** literal by key.
    ///
    /// Returns `None` if the key is missing, the value is not a JSON literal, or
    /// the field is a [`Field::Secret`] (use [`Self::get_secret`](Self::get_secret) instead).
    #[must_use]
    pub fn get(&self, key: &FieldKey) -> Option<&serde_json::Value> {
        match self.lookup(key) {
            ResolvedLookup::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow the secret material for a `Field::Secret` key, if present.
    #[must_use]
    pub fn get_secret(&self, key: &FieldKey) -> Option<&SecretValue> {
        match self.lookup(key) {
            ResolvedLookup::Secret(secret) => Some(secret),
            _ => None,
        }
    }

    /// Typed lookup that differentiates missing keys, secret values, and
    /// non-literal container values.
    #[must_use]
    pub fn lookup(&self, key: &FieldKey) -> ResolvedLookup<'_> {
        let Some(value) = self.values.get(key) else {
            return ResolvedLookup::Missing;
        };

        if matches!(self.schema.find(key), Some(Field::Secret(_))) {
            return match value {
                FieldValue::SecretLiteral(secret) => ResolvedLookup::Secret(secret),
                _ => ResolvedLookup::Complex(value),
            };
        }

        match value {
            FieldValue::Literal(literal) => ResolvedLookup::Literal(literal),
            FieldValue::SecretLiteral(secret) => ResolvedLookup::Secret(secret),
            _ => ResolvedLookup::Complex(value),
        }
    }

    /// Consume into a flat JSON object.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        self.values.to_json()
    }

    /// Consume and deserialize into a typed value.
    ///
    /// # Errors
    ///
    /// Returns `type_mismatch` when deserialization fails or when the resolved
    /// value tree still contains secret material. Secret-bearing schemas must
    /// cross an explicit boundary via [`Self::get_secret`] / `SecretWire`,
    /// rather than generic typed extraction.
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, Box<ValidationError>> {
        if let Some(path) = first_secret_path_in_values(&self.values) {
            return Err(Box::new(
                ValidationError::builder("type_mismatch")
                    .at(path)
                    .message(
                        "typed extraction refused because resolved values contain secret material; use get_secret() / SecretWire across an explicit boundary instead"
                            .to_owned(),
                    )
                    .build(),
            ));
        }

        serde_json::from_value(self.into_json()).map_err(|e| {
            Box::new(
                ValidationError::builder("type_mismatch")
                    .message(format!("deserialize failed: {e}"))
                    .build(),
            )
        })
    }
}

fn first_secret_path_in_values(values: &FieldValues) -> Option<FieldPath> {
    for (key, value) in values.iter() {
        let path = FieldPath::root().join(key.clone());
        if let Some(secret_path) = first_secret_path_in_field_value(value, &path) {
            return Some(secret_path);
        }
    }
    None
}

fn first_secret_path_in_field_value(value: &FieldValue, path: &FieldPath) -> Option<FieldPath> {
    match value {
        FieldValue::SecretLiteral(_) => Some(path.clone()),
        FieldValue::Object(map) => {
            for (key, child) in map {
                let child_path = path.clone().join(key.clone());
                if let Some(secret_path) = first_secret_path_in_field_value(child, &child_path) {
                    return Some(secret_path);
                }
            }
            None
        },
        FieldValue::List(items) => {
            for (index, child) in items.iter().enumerate() {
                let child_path = path.clone().join(index);
                if let Some(secret_path) = first_secret_path_in_field_value(child, &child_path) {
                    return Some(secret_path);
                }
            }
            None
        },
        FieldValue::Mode {
            value: Some(payload),
            ..
        } => {
            let payload_path = path.clone().join((*MODE_PAYLOAD_KEY).clone());
            first_secret_path_in_field_value(payload, &payload_path)
        },
        FieldValue::Literal(_)
        | FieldValue::Expression(_)
        | FieldValue::Mode { value: None, .. } => None,
    }
}

/// Shape label for validation errors (no `Debug` of full values — avoids leaking
/// secret-adjacent subtrees into messages).
const fn field_value_shape_for_errors(v: &FieldValue) -> &'static str {
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

/// Promote string literals to [`FieldValue::SecretLiteral`] for secret fields.
/// Recurses through object/list/mode containers.
fn promote_secrets_in_value(
    field: &Field,
    value: &mut FieldValue,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    match (field, &mut *value) {
        (Field::Secret(_), v) => promote_secret_value(v, path, report),
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
            let p = path.clone().join((*MODE_PAYLOAD_KEY).clone());
            promote_secrets_in_value(&var.field, mv.as_mut(), &p, report);
        },
        (Field::Mode(mode), FieldValue::Object(map)) => {
            let mode_selector_key = &*MODE_SELECTOR_KEY;
            let payload_key = &*MODE_PAYLOAD_KEY;
            let resolved_key = match map.get(mode_selector_key) {
                Some(FieldValue::Literal(serde_json::Value::String(mode_key))) => {
                    Some(mode_key.clone())
                },
                Some(_) => None,
                None => mode.default_variant.clone(),
            };
            let Some(var) = resolved_key
                .as_deref()
                .and_then(|mode_key| mode.variants.iter().find(|v| v.key == mode_key))
            else {
                return;
            };
            let Some(mv) = map.get_mut(payload_key) else {
                return;
            };
            let p = path.clone().join(payload_key.clone());
            promote_secrets_in_value(&var.field, mv, &p, report);
        },
        _ => {},
    }
}

fn promote_secret_value(value: &mut FieldValue, path: &FieldPath, report: &mut ValidationReport) {
    use serde_json::Value;
    match value {
        FieldValue::Literal(Value::String(s)) => {
            // Move the plaintext out of the JSON literal into a zeroizing
            // `SecretString` (no copy left behind: `mem::take` empties the
            // source `String`, which is then dropped as part of the replaced
            // `Literal`). Hashing/KDF is a credential-layer concern
            // (`nebula-credential`), not the schema layer's.
            let password = std::mem::take(s);
            *value = FieldValue::SecretLiteral(SecretValue::string(password));
        },
        FieldValue::Literal(_) => report.push(
            ValidationError::builder("type_mismatch")
                .at(path.clone())
                .message("secret field value must be a string")
                .build(),
        ),
        FieldValue::SecretLiteral(_) => {},
        FieldValue::Expression(_) => report.push(
            ValidationError::builder("expression.unresolved")
                .at(path.clone())
                .message("secret field still has an expression value at resolve time".to_owned())
                .build(),
        ),
        other => {
            let shape = field_value_shape_for_errors(other);
            report.push(
                ValidationError::builder("type_mismatch")
                    .at(path.clone())
                    .message(format!(
                        "secret field has incompatible value shape: {shape}"
                    ))
                    .build(),
            );
        },
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
) -> Pin<Box<dyn Future<Output = FieldValue> + Send + 'v>> {
    Box::pin(async move {
        match value {
            FieldValue::Expression(ref expr) => {
                tracing::debug!(
                    target: "nebula_schema::resolve",
                    path = %path,
                    "evaluating expression"
                );
                match expr.parse_at(path) {
                    Ok(ast) => match ctx.evaluate(ast).await {
                        Ok(v) => {
                            tracing::trace!(
                                target: "nebula_schema::resolve",
                                path = %path,
                                "expression resolved"
                            );
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
                            tracing::warn!(
                                target: "nebula_schema::resolve",
                                path = %path,
                                code = %e.code,
                                "expression evaluation failed"
                            );
                            report.push(e);
                            FieldValue::Literal(serde_json::Value::Null)
                        },
                    },
                    Err(e) => {
                        tracing::warn!(
                            target: "nebula_schema::resolve",
                            path = %path,
                            code = %e.code,
                            "expression parse failed at resolve"
                        );
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
                    let inner_path = path.clone().join((*MODE_PAYLOAD_KEY).clone());
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

/// Convert an absolute schema path to the validator's RFC-6901 `FieldPath`.
///
/// The shared predicate context is keyed by absolute `/a/b` pointers, so a
/// nested field's policy decl must use the same absolute pointer for its
/// `When` predicate lookups to resolve across nesting levels.
fn validator_path_from_schema_path(path: &FieldPath) -> nebula_validator::foundation::FieldPath {
    nebula_validator::foundation::FieldPath::from_segments(path.segments().iter().map(|seg| {
        match seg {
            crate::path::PathSegment::Key(k) => k.as_str().to_owned(),
            crate::path::PathSegment::Index(i) => i.to_string(),
        }
    }))
    .unwrap_or_else(|| nebula_validator::foundation::FieldPath::single(""))
}

/// One field of a level, paired with its value and the schema/validator
/// paths for that exact position in the tree.
struct LevelEntry<'a> {
    field: &'a Field,
    raw: Option<&'a FieldValue>,
    /// Schema-side path (dotted/indexed) used by value-rule reporting.
    schema_path: FieldPath,
    /// Validator-side RFC-6901 path used for the policy decl + the field
    /// pointer carried on a `required` failure.
    validator_path: nebula_validator::foundation::FieldPath,
}

/// Resolve visibility/required for one field-set level against the shared
/// whole-tree predicate context, then dispatch the per-field validation the
/// validator decided.
///
/// This is the SOLE route into per-field value validation at every nesting
/// level: a field reaches its value rules only through the `FieldPlan`
/// produced by `resolve_field_policies`. Each plan carries the `LevelEntry` it
/// was computed for as its opaque payload, so this runner cannot pair a plan
/// with the wrong field — it is a dumb dispatcher on `plan.directive` with no
/// policy logic of its own.
///
/// Each decl's `value_present` is computed here as
/// `!is_absent_for_required(field, raw)` and `raw_present` as `raw.is_some()`:
/// the schema owns the emptiness verdict (an empty string / empty collection /
/// null counts as ABSENT for the required check — HTML-form parity), feeds it
/// to the validator as data, and the validator decides and emits `required`.
fn gate_and_validate_level(
    entries: &[LevelEntry<'_>],
    ctx: &nebula_validator::PredicateContext,
    report: &mut ValidationReport,
) {
    use nebula_validator::policy::{
        FieldDirective, FieldPolicyDecl, RequiredPolicy, VisibilityPolicy, resolve_field_policies,
    };

    fn vis_policy(m: &crate::mode::VisibilityMode) -> VisibilityPolicy<'_> {
        match m {
            crate::mode::VisibilityMode::Always => VisibilityPolicy::Always,
            crate::mode::VisibilityMode::Never => VisibilityPolicy::Never,
            crate::mode::VisibilityMode::When(r) => VisibilityPolicy::When(r),
        }
    }
    fn req_policy(m: &crate::mode::RequiredMode) -> RequiredPolicy<'_> {
        match m {
            crate::mode::RequiredMode::Never => RequiredPolicy::Optional,
            crate::mode::RequiredMode::Always => RequiredPolicy::Always,
            crate::mode::RequiredMode::When(r) => RequiredPolicy::When(r),
        }
    }

    let decls = entries.iter().map(|e| {
        FieldPolicyDecl::new(
            &e.validator_path,
            vis_policy(e.field.visible()),
            req_policy(e.field.required()),
            !is_absent_for_required(e.field, e.raw),
            e.raw.is_some(),
            e,
        )
    });
    let resolution = resolve_field_policies(decls, ctx);

    // `required_failures` are validator errors carrying the field pointer;
    // merge them verbatim (code stays `required`, schema path resolved from
    // the carried pointer). The fallback path is unused because every decl
    // carries an explicit validator path. The validator is the SOLE
    // `required` emitter — this runner never synthesizes one.
    merge_validator_errors(&resolution.required_failures, &FieldPath::root(), report);

    // Dumb dispatcher: act on `plan.directive` only. Each plan carries the
    // `LevelEntry` it was computed for as its payload, so a plan can never be
    // paired with the wrong field; `FieldDirective` is cross-crate
    // `#[non_exhaustive]`, so an unknown future variant takes the wildcard arm,
    // which fails closed by still running structural validation (validate-more
    // — never silently skip a present value). Every audit line records only the
    // field PATH and the resolved policy enums — never the value, `entry.raw`,
    // or the predicate context — so secret-shaped values stay out of logs.
    for plan in &resolution.plans {
        let entry = plan.payload;
        match plan.directive {
            FieldDirective::Skip => {
                tracing::debug!(
                    target: "nebula_schema::validate",
                    field = %entry.schema_path,
                    presence = ?plan.presence,
                    requiredness = ?plan.requiredness,
                    decision = "skipped",
                    "field-gate decision"
                );
            },
            FieldDirective::RequiredAbsent => {
                tracing::debug!(
                    target: "nebula_schema::validate",
                    field = %entry.schema_path,
                    presence = ?plan.presence,
                    requiredness = ?plan.requiredness,
                    decision = "required-emitted",
                    "field-gate decision"
                );
            },
            FieldDirective::Validate => {
                tracing::debug!(
                    target: "nebula_schema::validate",
                    field = %entry.schema_path,
                    presence = ?plan.presence,
                    requiredness = ?plan.requiredness,
                    decision = "value-validated",
                    "field-gate decision"
                );
                validate_field(entry.field, entry.raw, &entry.schema_path, ctx, report);
            },
            _ => {
                // Conservative fallback for an unknown future directive: still
                // run structural validation. For a validation seam the safe
                // direction is to validate more, not less — silently skipping
                // would let a present value (e.g. a smuggled expression) reach
                // resolve unchecked. The validator remains the sole `required`
                // emitter, so this never synthesizes a `required`.
                tracing::debug!(
                    target: "nebula_schema::validate",
                    field = %entry.schema_path,
                    presence = ?plan.presence,
                    requiredness = ?plan.requiredness,
                    decision = "unknown-directive-validated",
                    "field-gate decision"
                );
                validate_field(entry.field, entry.raw, &entry.schema_path, ctx, report);
            },
        }
    }
}

/// Validate a single field against an optional raw value and a context.
///
/// Recurses for `Object`, `List`, and `Mode` containers.
fn validate_field(
    field: &Field,
    raw: Option<&FieldValue>,
    path: &FieldPath,
    ctx: &nebula_validator::PredicateContext,
    report: &mut ValidationReport,
) {
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
    validate_literal_value(field, value, path, ctx, report);
}

/// Type-check and rule-run a literal (non-expression) value.
#[expect(
    clippy::too_many_lines,
    reason = "field-type dispatch table — splitting into smaller fns reduces clarity"
)]
fn validate_literal_value(
    field: &Field,
    value: &FieldValue,
    path: &FieldPath,
    ctx: &nebula_validator::PredicateContext,
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
            run_value_rules(field.rules(), &transformed, path, report);
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
                run_value_rules(field.rules(), &v_for_rules, path, report);
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
            run_value_rules(field.rules(), &transformed, path, report);
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
            run_value_rules(field.rules(), &transformed, path, report);
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
            run_value_rules(rules, &transformed, path, report);
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
            run_value_rules(rules, &transformed, path, report);
            if !allow_custom && !options.is_empty() {
                check_select_options(options, *multiple, &transformed, path, report);
            }
        },
        Field::List(ListField {
            min_items,
            max_items,
            item,
            unique,
            transformers,
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
            let list_json = match value {
                FieldValue::List(items) => {
                    serde_json::Value::Array(items.iter().map(FieldValue::to_json).collect())
                },
                FieldValue::Literal(serde_json::Value::Array(arr)) => {
                    serde_json::Value::Array(arr.clone())
                },
                _ => return,
            };
            let transformed_list_json = apply_transformers(transformers, list_json);
            run_value_rules(rules, &transformed_list_json, path, report);
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
                let duplicate_index = items_typed.map_or_else(
                    || {
                        if let FieldValue::Literal(serde_json::Value::Array(arr)) = value {
                            first_duplicate_index(
                                arr.iter().map(|item| FieldValue::Literal(item.clone())),
                            )
                        } else {
                            None
                        }
                    },
                    |items_fv| first_duplicate_index(items_fv.iter().cloned()),
                );
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
            // Recurse into typed items when schema is present. Each element
            // is the same item schema at a distinct index — one level, gated
            // through the shared policy resolver.
            if let (Some(item_field), Some(items_fv)) = (item.as_deref(), items_typed) {
                let entries: Vec<LevelEntry<'_>> = items_fv
                    .iter()
                    .enumerate()
                    .map(|(i, item_val)| {
                        let item_path = path.clone().join(i);
                        let validator_path = validator_path_from_schema_path(&item_path);
                        LevelEntry {
                            field: item_field,
                            raw: Some(item_val),
                            schema_path: item_path,
                            validator_path,
                        }
                    })
                    .collect();
                gate_and_validate_level(&entries, ctx, report);
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
            let transformed = apply_transformers(transformers, value.to_json());
            run_value_rules(rules, &transformed, path, report);
            // Nested object children are their own field-set level, gated
            // against the shared whole-tree context (a child's `When` may
            // reference a sibling at any depth via an absolute pointer).
            let entries: Vec<LevelEntry<'_>> = child_fields
                .iter()
                .map(|child| {
                    let child_path = path.clone().join(child.key().clone());
                    let validator_path = validator_path_from_schema_path(&child_path);
                    LevelEntry {
                        field: child,
                        raw: map.get(child.key()),
                        schema_path: child_path,
                        validator_path,
                    }
                })
                .collect();
            gate_and_validate_level(&entries, ctx, report);
        },
        Field::Mode(ModeField {
            variants,
            default_variant,
            rules,
            ..
        }) => {
            let mode_selector_key = &*MODE_SELECTOR_KEY;
            let payload_key = &*MODE_PAYLOAD_KEY;

            let (resolved_key, mode_value) = match value {
                FieldValue::Mode {
                    mode: mode_key,
                    value: mode_value,
                } => (Some(mode_key.as_str()), mode_value.as_deref()),
                FieldValue::Object(map) => {
                    if map
                        .keys()
                        .any(|key| key.as_str() != "mode" && key.as_str() != "value")
                    {
                        report.push(
                            ValidationError::builder("type_mismatch")
                                .at(path.clone())
                                .message(format!(
                                    "field `{path}` expects a mode value ({{\"mode\": \"...\", ...}})"
                                ))
                                .build(),
                        );
                        return;
                    }

                    match map.get(mode_selector_key) {
                        Some(FieldValue::Literal(serde_json::Value::String(mode))) => {
                            (Some(mode.as_str()), map.get(payload_key))
                        },
                        Some(other) => {
                            report.push(
                                ValidationError::builder("type_mismatch")
                                    .at(path.clone().join(mode_selector_key.clone()))
                                    .message(format!(
                                        "field `{path}.mode` expects a string value, got {}",
                                        field_value_shape_for_errors(other)
                                    ))
                                    .build(),
                            );
                            return;
                        },
                        None => (None, map.get(payload_key)),
                    }
                },
                _ => {
                    report.push(
                        ValidationError::builder("type_mismatch")
                            .at(path.clone())
                            .message(format!(
                                "field `{path}` expects a mode value ({{\"mode\": \"...\", ...}})"
                            ))
                            .build(),
                    );
                    return;
                },
            };
            run_value_rules(rules, &value.to_json(), path, report);
            let resolved_key = resolved_key.or(default_variant.as_deref());
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
                // The mode payload is a single-field level; gate it through
                // the shared resolver so the variant field's own
                // visibility/required is honored too.
                let payload_path = path.clone().join(payload_key.clone());
                let validator_path = validator_path_from_schema_path(&payload_path);
                let entries = [LevelEntry {
                    field: &variant.field,
                    raw: mode_value,
                    schema_path: payload_path,
                    validator_path,
                }];
                gate_and_validate_level(&entries, ctx, report);
            }
        },
        // File, Computed, Dynamic, Notice — no type-check rule at schema time.
        Field::File(f) => {
            let raw_json = match value {
                FieldValue::Literal(lit) => lit.clone(),
                FieldValue::List(items) => {
                    serde_json::Value::Array(items.iter().map(FieldValue::to_json).collect())
                },
                _ => {
                    report.push(
                        ValidationError::builder("type_mismatch")
                            .at(path.clone())
                            .message(format!(
                                "field `{path}` expects {}",
                                if f.multiple {
                                    "an array of file paths"
                                } else {
                                    "a string file path"
                                }
                            ))
                            .build(),
                    );
                    return;
                },
            };
            let transformed = apply_transformers(&f.transformers, raw_json);
            if f.multiple {
                if let serde_json::Value::Array(items) = &transformed {
                    if items.iter().any(|v| !v.is_string()) {
                        report.push(
                            ValidationError::builder("type_mismatch")
                                .at(path.clone())
                                .message(format!(
                                    "field `{path}` expects an array of string file paths"
                                ))
                                .build(),
                        );
                        return;
                    }
                } else {
                    report.push(
                        ValidationError::builder("type_mismatch")
                            .at(path.clone())
                            .message(format!("field `{path}` expects an array of file paths"))
                            .build(),
                    );
                    return;
                }
            } else if !transformed.is_string() {
                report.push(
                    ValidationError::builder("type_mismatch")
                        .at(path.clone())
                        .message(format!("field `{path}` expects a string file path"))
                        .build(),
                );
                return;
            }

            run_value_rules(field.rules(), &transformed, path, report);
        },
        // `Unknown` is opaque: `rules()` is empty, so this type-checks nothing
        // and accepts the value (this version cannot validate a future field kind).
        Field::Computed(_) | Field::Dynamic(_) | Field::Notice(_) | Field::Unknown(_) => {
            run_value_rules(field.rules(), &value.to_json(), path, report);
        },
    }
}

fn first_duplicate_index(values: impl IntoIterator<Item = FieldValue>) -> Option<usize> {
    // Bucket by injective `canonical_bytes`, so `1` and `1.0` (and key-permuted
    // objects) count as equal — fixing the `"1"`-vs-`"1.0"` false negative the
    // old `serde_json::to_string` bucketing missed.
    let mut seen_canon: HashSet<Vec<u8>> = HashSet::new();
    // Secret-bearing items have no canonical form; fall back to structural
    // `PartialEq` (constant-time inside `SecretValue`) so a unique list that
    // contains secrets is still validated rather than erroring.
    let mut seen_opaque: Vec<FieldValue> = Vec::new();
    for (idx, value) in values.into_iter().enumerate() {
        if let Ok(canon) = value.canonical_bytes() {
            if !seen_canon.insert(canon) {
                return Some(idx);
            }
        } else if seen_opaque.contains(&value) {
            // No canonical form (e.g. a secret) — fall back to structural PartialEq.
            return Some(idx);
        } else {
            seen_opaque.push(value);
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
    options: &[SelectOption],
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

/// Merge validator errors into the schema [`ValidationReport`] verbatim.
///
/// The validator is the sole emitter of rule-failure codes: each error's code
/// flows through unchanged (no schema-side namespace remap), the message is
/// preserved, and the issue path is the validator error's own RFC-6901 field
/// pointer parsed back into a schema [`FieldPath`]. When the validator error
/// carries no parsable pointer the caller-supplied `fallback` path is used.
fn merge_validator_errors(
    errs: &nebula_validator::foundation::ValidationErrors,
    fallback: &FieldPath,
    report: &mut ValidationReport,
) {
    for e in errs.errors() {
        let code: String = e.code.as_ref().to_owned();
        let msg: String = e.message.as_ref().to_owned();
        let issue_path = match e.field_pointer().as_deref() {
            Some(pointer) => {
                crate::rule_ref::field_path_from_json_pointer(pointer).unwrap_or_else(|| {
                    tracing::warn!(
                        target: "nebula_schema::validate",
                        pointer,
                        fallback = %fallback,
                        "validator error carried unparsable field pointer; falling back"
                    );
                    fallback.clone()
                })
            },
            None => fallback.clone(),
        };
        report.push(
            ValidationError::builder(code)
                .at(issue_path)
                .message(msg)
                .build(),
        );
    }
}

/// Apply a slice of rules to a JSON value through the single validator
/// crossing, merging any failures into `report`.
fn run_value_rules(
    rules: &[nebula_validator::Rule],
    value: &serde_json::Value,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    if let Err(errs) = nebula_validator::validate_rules_with_ctx(
        value,
        rules,
        None,
        nebula_validator::ExecutionMode::StaticOnly,
    ) {
        merge_validator_errors(&errs, path, report);
    }
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, Schema, field_key};

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
            .add(
                Field::object(FieldKey::new("user").unwrap())
                    .add(Field::string(field_key!("email"))),
            )
            .add(Field::mode(FieldKey::new("auth").unwrap()).variant(
                "token",
                "Token",
                Field::string(field_key!("value")),
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
    fn find_by_path_handles_list_object_children() {
        let schema = Schema::builder()
            .add(
                Field::list(FieldKey::new("items").unwrap()).item(
                    Field::object(FieldKey::new("item").unwrap())
                        .add(Field::string(field_key!("name"))),
                ),
            )
            .build()
            .unwrap();

        let field = schema
            .find_by_path(&FieldPath::parse("items.name").unwrap())
            .expect("list item child should be indexed");
        assert_eq!(field.key().as_str(), "name");
        assert!(
            schema
                .find_by_path(&FieldPath::parse("items[0].name").unwrap())
                .is_none(),
            "schema paths use the canonical anonymous list-item path"
        );
        assert!(
            schema
                .find_by_path(&FieldPath::parse("items.missing").unwrap())
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
    fn root_rule_error_preserves_validator_field_path() {
        use nebula_validator::{Predicate, Rule};
        use serde_json::json;

        let schema = Schema::builder()
            .add(
                Field::object(FieldKey::new("config").unwrap())
                    .add(Field::string(FieldKey::new("tier").unwrap())),
            )
            .root_rule(Rule::predicate(
                Predicate::eq("/config/tier", json!("pro")).unwrap(),
            ))
            .build()
            .unwrap();

        let bad = FieldValues::from_json(json!({"config": {"tier": "free"}})).unwrap();
        let report = schema.validate(&bad).unwrap_err();
        assert!(
            report.errors().any(|e| e.path.to_string() == "config.tier"),
            "expected root-rule error at config.tier, got: {report:?}"
        );
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

    #[test]
    fn deserialize_bare_any_is_accepted() {
        use serde_json::json;

        let decoded: ValidSchema = serde_json::from_value(json!({"kind": "any"})).unwrap();
        assert_eq!(decoded.kind(), SchemaKind::Any);
        assert!(decoded.fields().is_empty());
    }

    #[test]
    fn deserialize_any_carrying_fields_is_rejected() {
        use serde_json::json;

        // Build a real typed schema, then mistag it as `Any` while keeping its
        // `fields`. Accepting this would silently drop every field constraint.
        let typed = Schema::builder()
            .add(Field::string(field_key!("x")).required())
            .build()
            .unwrap();
        let mut wire = serde_json::to_value(&typed).unwrap();
        wire["kind"] = json!("any");

        let result: Result<ValidSchema, _> = serde_json::from_value(wire);
        assert!(
            result.is_err(),
            "a schema tagged `kind: any` that still carries fields must be rejected, not \
             silently coerced to the unconstrained `Any`"
        );
    }

    #[test]
    fn deserialize_any_carrying_root_rules_is_rejected() {
        use nebula_validator::{Predicate, Rule};
        use serde_json::json;

        let with_rules = Schema::builder()
            .add(Field::string(field_key!("x")))
            .root_rule(Rule::predicate(Predicate::eq("x", json!("a")).unwrap()))
            .build()
            .unwrap();
        let mut wire = serde_json::to_value(&with_rules).unwrap();
        wire["kind"] = json!("any");
        // Drop fields so only `root_rules` remains to prove the rule branch fires.
        wire["fields"] = json!([]);

        let result: Result<ValidSchema, _> = serde_json::from_value(wire);
        assert!(
            result.is_err(),
            "a schema tagged `kind: any` that still carries root_rules must be rejected"
        );
    }

    #[test]
    fn list_field_custom_rules_are_enforced() {
        use nebula_validator::Rule;
        use serde_json::json;

        let schema = Schema::builder()
            .add(
                Field::list(FieldKey::new("tags").unwrap())
                    .item(Field::string(FieldKey::new("tag").unwrap()))
                    .with_rule(Rule::max_items(1)),
            )
            .build()
            .unwrap();

        let values = FieldValues::from_json(json!({"tags": ["a", "b"]})).unwrap();
        let report = schema.validate(&values).expect_err("list rule must fail");
        assert!(
            report.has_errors(),
            "expected list-level custom rule to produce an error"
        );
    }

    #[test]
    fn object_field_custom_rules_are_enforced() {
        use nebula_validator::Rule;
        use serde_json::json;

        let schema = Schema::builder()
            .add(
                Field::object(FieldKey::new("config").unwrap())
                    .add(Field::boolean(FieldKey::new("enabled").unwrap()))
                    .with_rule(Rule::one_of([json!({"enabled": true})])),
            )
            .build()
            .unwrap();

        let values = FieldValues::from_json(json!({"config": {"enabled": false}})).unwrap();
        let report = schema.validate(&values).expect_err("object rule must fail");
        assert!(report.errors().any(|e| e.code == "one_of"));
    }
}
