//! Schema container and builder.
//!
//! `SchemaBuilder::build()` runs structural lint passes and produces a
//! `ValidSchema` proof-token.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    LoaderContext, LoaderRegistry, LoaderResult, Property, SelectOption,
    error::{ValidationError, ValidationReport},
    field_tree::{mode_variant_path, walk_schema_fields},
    path::{FieldPath, PathSegment},
    validated::{FieldHandle, RootShape, SchemaFlags, ValidSchema, ValidSchemaInner},
};

// ── Builder entry point ───────────────────────────────────────────────────────

/// Schema aggregate — a collection of typed property definitions.
///
/// Build a schema with `Schema::builder()` then call `SchemaBuilder::build()`
/// to get a `ValidSchema` proof-token.
///
/// # Example
///
/// ```rust
/// use nebula_schema::{Property, Schema, field_key};
///
/// let schema = Schema::builder()
///     .property(Property::string(field_key!("name")).required())
///     .property(Property::number(field_key!("score")))
///     .build()
///     .expect("valid schema");
///
/// assert_eq!(schema.properties().len(), 2);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered property list.
    #[serde(rename = "fields")]
    properties: Vec<Property>,
}

impl Schema {
    /// Create a new `SchemaBuilder`.
    #[must_use]
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    /// Number of top-level properties.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.properties.len()
    }

    /// Returns true when schema has no properties.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }

    /// Find property by key (string slice).
    #[must_use]
    pub fn find_property(&self, key: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|property| property.key().as_str() == key)
    }

    /// Borrow all top-level properties in insertion order.
    #[must_use]
    pub const fn properties(&self) -> &[Property] {
        self.properties.as_slice()
    }

    /// Run static lint checks for schema structure and references.
    ///
    /// Returns a [`ValidationReport`] — warnings are advisory, errors indicate
    /// structural problems.
    #[must_use]
    pub fn lint(&self) -> ValidationReport {
        let mut report = ValidationReport::new();
        // Same self-bounded depth/fan-out guard `build()` runs, BEFORE the
        // unbounded `lint_tree` recursion — so a `Schema` constructed or
        // deserialized outside the builder cannot drive lint into a stack
        // overflow. Stop on a structural error rather than recursing.
        validate_index_limits(&self.properties, &FieldPath::root(), 0, &mut report);
        if report.has_errors() {
            return report;
        }
        crate::lint::lint_current_secret_defaults(
            &self.properties,
            &FieldPath::root(),
            &mut report,
        );
        crate::lint::lint_tree(&self.properties, &FieldPath::root(), &mut report);
        report
    }

    /// Resolve dynamic options for a select field through loader registry.
    ///
    /// # Errors
    ///
    /// - `field.not_found` — schema has no field with this key.
    /// - `field.type_mismatch` — field exists but isn't a `Select`. Carries `expected` and `actual`
    ///   params.
    /// - `loader.missing_config` — field is a select but has no loader configured (static options
    ///   only).
    /// - `loader.not_registered` / `loader.failed` — propagated from the loader registry.
    pub async fn load_select_options(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let path = FieldPath::root().join(parse_top_level_key(key)?);
        self.load_select_options_at(&path, registry, context).await
    }

    /// Resolve dynamic options for a select field at a nested schema path.
    ///
    /// This path addresses the schema tree, not a concrete runtime instance.
    /// For example:
    ///
    /// - nested object child: `config.workspace`
    /// - list item child: `rows[0].workspace` or `rows.workspace`
    /// - mode variant child: `auth.oauth.workspace`
    ///
    /// # Errors
    ///
    /// - `field.not_found` — schema has no field at this path.
    /// - `field.type_mismatch` — field exists but isn't a `Select`. Carries `expected` and `actual`
    ///   params.
    /// - `loader.missing_config` — field is a select but has no loader configured (static options
    ///   only).
    /// - `loader.not_registered` / `loader.failed` — propagated from the loader registry.
    pub async fn load_select_options_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let loader_key = resolve_select_loader_path(self.properties(), path)?;
        registry
            .load_options(&loader_key, context.redacted(self.properties())?)
            .await
    }

    /// Resolve dynamic record payloads for a dynamic field through registry.
    ///
    /// # Errors
    ///
    /// Same taxonomy as [`Schema::load_select_options`] — `field.not_found`,
    /// `field.type_mismatch`, `loader.missing_config`, or the registry's
    /// `loader.not_registered` / `loader.failed`.
    pub async fn load_dynamic_records(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let path = FieldPath::root().join(parse_top_level_key(key)?);
        self.load_dynamic_records_at(&path, registry, context).await
    }

    /// Resolve dynamic record payloads for a field at a nested schema path.
    ///
    /// Uses the same schema-path addressing rules as [`Schema::load_select_options_at`].
    ///
    /// # Errors
    ///
    /// Same taxonomy as [`Schema::load_select_options_at`] — `field.not_found`,
    /// `field.type_mismatch`, `loader.missing_config`, or the registry's
    /// `loader.not_registered` / `loader.failed`.
    pub async fn load_dynamic_records_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let loader_key = resolve_dynamic_loader_path(self.properties(), path)?;
        registry
            .load_records(&loader_key, context.redacted(self.properties())?)
            .await
    }
}

// ── Loader resolution helpers ─────────────────────────────────────────────────

/// Parse a top-level key for loader APIs.
///
/// # Errors
///
/// Returns `invalid_key` when `key` does not satisfy [`FieldKey`](crate::FieldKey) constraints.
fn parse_top_level_key(key: &str) -> Result<crate::key::FieldKey, ValidationError> {
    crate::key::FieldKey::new(key)
        .map_err(|e| ValidationError::invalid_key(FieldPath::root(), key, e.message()))
}

pub(crate) fn resolve_select_loader_key(
    properties: &[Property],
    key: &str,
) -> Result<String, ValidationError> {
    let path = FieldPath::root().join(parse_top_level_key(key)?);
    resolve_select_loader_path(properties, &path)
}

pub(crate) fn resolve_select_loader_path(
    properties: &[Property],
    path: &FieldPath,
) -> Result<String, ValidationError> {
    let property = find_property_by_schema_path(properties, path)?;
    let Property::Select(select) = property else {
        return Err(loader_type_mismatch(path, "select", property.type_name()));
    };
    loader_key_or_error(select.loader.as_deref(), path)
}

pub(crate) fn resolve_dynamic_loader_key(
    properties: &[Property],
    key: &str,
) -> Result<String, ValidationError> {
    let path = FieldPath::root().join(parse_top_level_key(key)?);
    resolve_dynamic_loader_path(properties, &path)
}

pub(crate) fn resolve_dynamic_loader_path(
    properties: &[Property],
    path: &FieldPath,
) -> Result<String, ValidationError> {
    let property = find_property_by_schema_path(properties, path)?;
    let Property::Dynamic(dynamic) = property else {
        return Err(loader_type_mismatch(path, "dynamic", property.type_name()));
    };
    loader_key_or_error(dynamic.loader.as_deref(), path)
}

fn loader_key_or_error(loader: Option<&str>, path: &FieldPath) -> Result<String, ValidationError> {
    loader
        .filter(|loader| !loader.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ValidationError::loader_missing_config(path.clone()))
}

fn loader_type_mismatch(path: &FieldPath, expected: &str, actual: &str) -> ValidationError {
    ValidationError::field_type_mismatch(path.clone(), expected, actual)
}

/// Look up a named child property under `parent_path`, returning the child and its full path.
fn find_named_child<'a>(
    properties: &'a [Property],
    key: &crate::key::FieldKey,
    parent_path: &FieldPath,
) -> Result<(&'a Property, FieldPath), ValidationError> {
    let child_path = parent_path.clone().join(key.clone());
    match properties.iter().find(|property| property.key() == key) {
        Some(property) => Ok((property, child_path)),
        None => Err(ValidationError::field_not_found(child_path)),
    }
}

fn find_property_by_schema_path<'a>(
    properties: &'a [Property],
    path: &FieldPath,
) -> Result<&'a Property, ValidationError> {
    let mut segments = path.segments().iter();
    let Some(PathSegment::Key(first_key)) = segments.next() else {
        return Err(ValidationError::field_not_found(path.clone()));
    };

    let (mut current, mut current_path) =
        find_named_child(properties, first_key, &FieldPath::root())?;

    for segment in segments {
        match segment {
            PathSegment::Key(key) => match current {
                Property::Object(object) => {
                    (current, current_path) = find_named_child(&object.fields, key, &current_path)?;
                },
                Property::List(list) => {
                    let Some(item) = list.item.as_deref() else {
                        current_path = current_path.join(key.clone());
                        return Err(ValidationError::field_not_found(current_path));
                    };
                    if let Property::Object(object) = item {
                        (current, current_path) =
                            find_named_child(&object.fields, key, &current_path)?;
                    } else {
                        current_path = current_path.join(key.clone());
                        return Err(ValidationError::field_not_found(current_path));
                    }
                },
                Property::Mode(mode) => {
                    current_path = current_path.join(key.clone());
                    current = mode
                        .variants
                        .iter()
                        .find(|variant| variant.key == key.as_str())
                        .map(|variant| variant.field.as_ref())
                        .ok_or_else(|| ValidationError::field_not_found(current_path.clone()))?;
                },
                _ => {
                    current_path = current_path.join(key.clone());
                    return Err(ValidationError::field_not_found(current_path));
                },
            },
            PathSegment::Index(index) => {
                current_path = current_path.join(*index);
                match current {
                    Property::List(list) => {
                        current = list.item.as_deref().ok_or_else(|| {
                            ValidationError::field_not_found(current_path.clone())
                        })?;
                    },
                    _ => {
                        return Err(ValidationError::field_not_found(current_path));
                    },
                }
            },
        }
    }

    Ok(current)
}

// ── SchemaBuilder ─────────────────────────────────────────────────────────────

/// Mutable builder state. Consumed by `build()`.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    properties: Vec<Property>,
    root_rules: Vec<nebula_validator::Rule>,
}

impl SchemaBuilder {
    /// Append a semantic property to the builder.
    #[must_use]
    pub fn property(mut self, property: impl Into<Property>) -> Self {
        self.properties.push(property.into());
        self
    }

    /// Attach a schema-level rule evaluated against the full submitted value
    /// object after per-field validation succeeds.
    ///
    /// Rules are executed via [`nebula_validator::validate_rules_with_ctx`] with a
    /// [`nebula_validator::PredicateContext`] built from
    /// [`AuthoredValue::to_json`](crate::AuthoredValue::to_json).
    /// [`ExecutionMode::StaticOnly`](nebula_validator::ExecutionMode::StaticOnly) is used, so
    /// **deferred** rules (including [`Rule::custom`](nebula_validator::Rule::custom))
    /// are skipped here and remain wire hooks for the workflow engine.
    #[must_use]
    pub fn root_rule(mut self, rule: nebula_validator::Rule) -> Self {
        self.root_rules.push(rule);
        self
    }

    /// Append many semantic properties at once.
    ///
    /// Accepts `Vec<Property>`, `[Property; N]`, iterators, and anything
    /// `Into<Property>` per item. Preferred over chaining `.property(...)` for
    /// statically known bulk additions.
    #[must_use]
    pub fn properties<I, F>(mut self, properties: I) -> Self
    where
        I: IntoIterator<Item = F>,
        F: Into<Property>,
    {
        self.properties
            .extend(properties.into_iter().map(Into::into));
        self
    }

    /// Append a group of properties that share a common label and optional
    /// `visible_when` / `required_when` conditions.
    ///
    /// ```rust
    /// use nebula_schema::{PropertyCollector, Schema, StringWidget, field_key};
    /// use nebula_validator::{Predicate, Rule};
    ///
    /// let rule = Rule::predicate(Predicate::eq("method", "POST").unwrap()).unwrap();
    /// let schema = Schema::builder()
    ///     .string(field_key!("method"), |s| s.required())
    ///     .group("body_section", |g| {
    ///         g.visible_when(rule)
    ///             .string(field_key!("body"), |s| s.widget(StringWidget::Multiline))
    ///     })
    ///     .unwrap()
    ///     .build()
    ///     .unwrap();
    /// assert_eq!(schema.properties().len(), 2);
    /// ```
    pub fn group(
        mut self,
        name: impl Into<String>,
        f: impl FnOnce(crate::builder::GroupBuilder) -> crate::builder::GroupBuilder,
    ) -> Result<Self, nebula_validator::RuleBuildError> {
        let builder = f(crate::builder::GroupBuilder::new(name));
        self.properties.extend(builder.into_properties()?);
        Ok(self)
    }

    /// Borrow the properties currently staged on the builder.
    #[must_use]
    pub fn staged_properties(&self) -> &[Property] {
        &self.properties
    }

    /// Run lint passes and produce a validated runtime schema.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationReport`] when structural linting or index-limit
    /// checks fail.
    pub fn build(self) -> Result<ValidSchema, ValidationReport> {
        self.build_with_policy(None, crate::validated::SchemaPolicy::PropertiesV2)
    }

    pub(crate) fn build_with_policy(
        self,
        serde_tagging: Option<crate::SerdeTagging>,
        policy: crate::validated::SchemaPolicy,
    ) -> Result<ValidSchema, ValidationReport> {
        let mut properties = self.properties;
        let mut report = ValidationReport::new();

        // Depth / sibling-count bounds FIRST. `validate_index_limits` is
        // self-bounded — it stops recursing past `u8::MAX` depth via
        // `checked_add` — so it cannot overflow the stack. The structural lint
        // passes below recurse on the raw nesting depth with no internal cap, so
        // an over-deep tree must be rejected here before they run. This mirrors
        // the explicit `MAX_VALUE_DEPTH` guard the value tree already has.
        validate_index_limits(&properties, &FieldPath::root(), 0, &mut report);
        if report.has_errors() {
            return Err(report);
        }

        if matches!(policy, crate::validated::SchemaPolicy::PropertiesV2) {
            crate::lint::lint_current_secret_defaults(&properties, &FieldPath::root(), &mut report);
        }
        crate::lint::lint_tree(&properties, &FieldPath::root(), &mut report);
        // Root-rule diagnostics are best-effort while structural lint errors
        // are present; build still stops before indexing when any error exists.
        crate::lint::lint_root_rules(&self.root_rules, &properties, &mut report);

        if report.has_errors() {
            return Err(report);
        }

        normalize_depends_on_lists(&mut properties);

        // Build the flat path index for O(1) path lookup.
        let mut index: IndexMap<FieldPath, FieldHandle> = IndexMap::new();
        let mut flags = SchemaFlags::default();
        let mut has_contextual_rules =
            crate::validated::rules_use_predicate_context(&self.root_rules);
        build_index(
            &properties,
            &mut index,
            &mut flags,
            &mut has_contextual_rules,
        );

        let root = match serde_tagging {
            Some(tagging) => {
                if !self.root_rules.is_empty() {
                    return Err(ValidationError::builder("union.root_rules")
                        .message("tagged union roots cannot carry record rules")
                        .build()
                        .into());
                }
                RootShape::union(properties, tagging)?
            },
            None => RootShape::record(properties, self.root_rules),
        };
        Ok(ValidSchema::from_inner(ValidSchemaInner {
            policy,
            root,
            index,
            flags,
            has_contextual_rules,
        }))
    }
}

impl crate::builder::PropertyCollector for SchemaBuilder {
    fn push_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }
}

fn normalize_depends_on_lists(properties: &mut [Property]) {
    for property in properties {
        normalize_property_for_runtime(property);
    }
}

fn normalize_property_for_runtime(property: &mut Property) {
    match property {
        Property::String(string) => {
            dedupe_rules_and_transformers(&mut string.rules, &mut string.transformers);
        },
        Property::Secret(secret) => {
            dedupe_rules_and_transformers(&mut secret.rules, &mut secret.transformers);
        },
        Property::Number(number) => {
            dedupe_rules_and_transformers(&mut number.rules, &mut number.transformers);
        },
        Property::Boolean(boolean) => {
            dedupe_rules_and_transformers(&mut boolean.rules, &mut boolean.transformers);
        },
        Property::Select(select) => {
            dedupe_rules_and_transformers(&mut select.rules, &mut select.transformers);
            dedupe_depends_on_paths(&mut select.depends_on);
        },
        Property::Object(object) => {
            dedupe_rules_and_transformers(&mut object.rules, &mut object.transformers);
            normalize_depends_on_lists(&mut object.fields);
        },
        Property::List(list) => {
            dedupe_rules_and_transformers(&mut list.rules, &mut list.transformers);
            if let Some(item) = list.item.as_deref_mut() {
                normalize_property_for_runtime(item);
            }
        },
        Property::Mode(mode) => {
            dedupe_rules_and_transformers(&mut mode.rules, &mut mode.transformers);
            for variant in &mut mode.variants {
                normalize_property_for_runtime(variant.field.as_mut());
            }
        },
        Property::Code(code) => {
            dedupe_rules_and_transformers(&mut code.rules, &mut code.transformers);
        },
        Property::File(file) => {
            dedupe_rules_and_transformers(&mut file.rules, &mut file.transformers);
        },
        Property::Computed(computed) => {
            dedupe_rules_and_transformers(&mut computed.rules, &mut computed.transformers);
        },
        Property::Dynamic(dynamic) => {
            dedupe_rules_and_transformers(&mut dynamic.rules, &mut dynamic.transformers);
            dedupe_depends_on_paths(&mut dynamic.depends_on);
        },
        Property::Notice(notice) => {
            dedupe_rules_and_transformers(&mut notice.rules, &mut notice.transformers);
        },
        // An `Unknown` property's rules/transformers live untyped inside `raw`;
        // nothing to normalize.
        Property::Unknown(_) => {},
    }
}

fn dedupe_depends_on_paths(depends_on: &mut Vec<FieldPath>) {
    let mut seen = HashSet::new();
    depends_on.retain(|path| seen.insert(path.to_string()));
}

fn dedupe_rules_and_transformers(
    rules: &mut Vec<nebula_validator::Rule>,
    transformers: &mut Vec<crate::Transformer>,
) {
    dedupe_stable_eq(rules);
    dedupe_stable_eq(transformers);
}

fn dedupe_stable_eq<T: PartialEq>(items: &mut Vec<T>) {
    let mut unique = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if !unique.iter().any(|existing| existing == &item) {
            unique.push(item);
        }
    }
    *items = unique;
}

// ── Index builder ─────────────────────────────────────────────────────────────

fn build_index(
    properties: &[Property],
    index: &mut IndexMap<FieldPath, FieldHandle>,
    flags: &mut SchemaFlags,
    has_contextual_rules: &mut bool,
) {
    use crate::mode::ExpressionMode;

    // PRECONDITION: validate_index_limits must have succeeded before
    // build_index calls walk_schema_fields, otherwise invalid oversize sibling
    // groups can be skipped by the walker before insertion.
    walk_schema_fields(properties, |node| {
        flags.max_depth = flags.max_depth.max(node.depth);

        // Track expression usage.
        if !matches!(node.property.expression(), ExpressionMode::Forbidden) {
            flags.uses_expressions = true;
        }

        // Track async loader usage.
        let has_loader = match node.property {
            Property::Select(s) => s
                .loader
                .as_ref()
                .is_some_and(|loader| !loader.trim().is_empty()),
            Property::Dynamic(d) => d
                .loader
                .as_ref()
                .is_some_and(|loader| !loader.trim().is_empty()),
            _ => false,
        };
        if has_loader {
            flags.has_async_loaders = true;
        }

        *has_contextual_rules |= property_uses_predicate_context(node.property);

        index.insert(
            node.path,
            FieldHandle {
                cursor: node.cursor,
                depth: node.depth,
            },
        );
    });
}

fn property_uses_predicate_context(property: &Property) -> bool {
    crate::validated::rules_use_predicate_context(property.rules())
        || matches!(property.required(), crate::RequiredMode::When(_))
}

/// Maximum schema-tree nesting depth accepted by [`SchemaBuilder::build`] and
/// [`Schema::lint`].
///
/// The schema-tree analogue of [`crate::value::MAX_VALUE_DEPTH`]. Enforced by
/// `validate_index_limits`, which runs **before** the recursive lint passes in
/// `build()` / `lint()`, so an over-deep schema is rejected with
/// `schema.depth_limit` before any unbounded recursion (lint cycle DFS,
/// `validate_field`, `promote_secrets_in_value`) can overflow the stack. The
/// guard walks every nested field shape — object fields, list items of any kind
/// (`List<List<…>>`, `List<Mode<…>>`, not just `List<Object>`), and mode-variant
/// payloads. A schema deeper than this could not have its values validated
/// anyway - authored value ingestion caps at the same `MAX_VALUE_DEPTH`.
///
/// This guards the *logical* tree once it has been deserialized. Untrusted
/// schema bytes reach the system as JSON (plugin protocol, API), whose parser
/// self-limits recursion; a non-self-limiting binary deserializer
/// (`StorageFormat::MessagePack`, off by default) is only used for trusted
/// at-rest data and is not fed attacker-controlled schema trees.
pub const MAX_SCHEMA_DEPTH: u8 = 64;

/// Per-level fan-out cap: the property-handle cursor is `u16`, so a single level
/// can index at most `u16::MAX + 1` siblings / variants.
const MAX_INDEXABLE_SIBLINGS: usize = u16::MAX as usize + 1;

fn schema_depth_limit_error(path: &FieldPath) -> ValidationError {
    ValidationError::builder("schema.depth_limit")
        .at(path.clone())
        .param("limit", Value::from(MAX_SCHEMA_DEPTH))
        .message(format!(
            "schema nesting depth exceeds the {MAX_SCHEMA_DEPTH}-level limit at `{path}`"
        ))
        .build()
}

fn sibling_overflow_error(path: &FieldPath, kind: &str, actual: usize) -> ValidationError {
    ValidationError::builder("schema.index_overflow")
        .at(path.clone())
        .param("limit", Value::from(MAX_INDEXABLE_SIBLINGS))
        .param("actual", Value::from(actual))
        .message(format!(
            "too many {kind} at `{path}`: {actual} > {MAX_INDEXABLE_SIBLINGS}"
        ))
        .build()
}

/// Bound schema-tree depth and per-level sibling/variant counts BEFORE the
/// unbounded recursive lint / validate / resolve passes run.
///
/// Walks **every** nested field shape those passes can traverse — object
/// fields, list items of any kind (including `List<List<…>>` and
/// `List<Mode<…>>`, not just `List<Object>`), and mode-variant payloads — so no
/// nesting path escapes `MAX_SCHEMA_DEPTH`.
fn validate_index_limits(
    properties: &[Property],
    path: &FieldPath,
    depth: u8,
    report: &mut ValidationReport,
) {
    if depth > MAX_SCHEMA_DEPTH {
        report.push(schema_depth_limit_error(path));
        return;
    }
    if properties.len() > MAX_INDEXABLE_SIBLINGS {
        report.push(sibling_overflow_error(
            path,
            "sibling properties",
            properties.len(),
        ));
    }
    for property in properties {
        let child_path = path.clone().join(property.key().clone());
        validate_property_subtree_limits(property, &child_path, depth, report);
    }
}

/// Recurse a single property (at nesting level `depth`) into every container kind,
/// bounding depth and fan-out. Mirrors the structural recursion the lint /
/// `validate_field` / `promote_secrets_in_value` passes perform, so the cap
/// covers exactly the paths those unbounded passes can take.
fn validate_property_subtree_limits(
    property: &Property,
    path: &FieldPath,
    depth: u8,
    report: &mut ValidationReport,
) {
    if depth > MAX_SCHEMA_DEPTH {
        report.push(schema_depth_limit_error(path));
        return;
    }
    let Some(child_depth) = depth.checked_add(1) else {
        // Unreachable while MAX_SCHEMA_DEPTH < u8::MAX, but keep the backstop so
        // a future cap raise cannot silently overflow the u8 depth counter.
        report.push(schema_depth_limit_error(path));
        return;
    };
    match property {
        Property::Object(object) => {
            if object.fields.len() > MAX_INDEXABLE_SIBLINGS {
                report.push(sibling_overflow_error(
                    path,
                    "sibling properties",
                    object.fields.len(),
                ));
            }
            for child in &object.fields {
                let child_path = path.clone().join(child.key().clone());
                validate_property_subtree_limits(child, &child_path, child_depth, report);
            }
        },
        Property::List(list) => {
            // A list item is one nesting level deeper and may be ANY property kind;
            // descend it so `List<List<…>>` / `List<Mode<…>>` cannot bypass the cap.
            if let Some(item) = list.item.as_deref() {
                validate_property_subtree_limits(item, path, child_depth, report);
            }
        },
        Property::Mode(mode) => {
            if mode.variants.len() > MAX_INDEXABLE_SIBLINGS {
                report.push(sibling_overflow_error(
                    path,
                    "mode variants",
                    mode.variants.len(),
                ));
            }
            for variant in &mode.variants {
                let Some(variant_path) = mode_variant_path(path, variant.key.as_str()) else {
                    continue;
                };
                validate_property_subtree_limits(
                    &variant.field,
                    &variant_path,
                    child_depth,
                    report,
                );
            }
        },
        _ => {},
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
