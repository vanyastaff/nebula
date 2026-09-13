//! Structural validation and the single validator rule/policy crossing.

use std::{
    cell::OnceCell,
    collections::{BTreeSet, HashSet},
    sync::LazyLock,
};

use nebula_validator::{
    DeferredReason, EvaluationOutcome, ExecutionMode, PredicateContext, Rule, ValueRule,
    foundation::ValidationErrors,
    policy::{
        FieldDirective, FieldPolicyDecl, RequiredPolicy, VisibilityPolicy, resolve_field_policies,
    },
};
use serde_json::Value;
use zeroize::Zeroize;

use super::{RootShape, ValidSchema};
use crate::{
    Field, RequiredMode, SecretValue, SelectOption, ValidationError, ValidationReport, ValuePath,
    ValueTree, VisibilityMode,
    commitment::{CommitmentKey, write_secret_commitment},
};

/// Work which has not been proven against runtime values yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingValidation {
    /// An admitted expression still has to produce a value of the declared type.
    Value {
        /// Exact data node awaiting evaluation or a structural check.
        path: ValuePath,
    },
    /// A rule requires a later execution phase or an unresolved context value.
    Rule {
        /// Data node to which the rule applies.
        path: ValuePath,
        /// The missing input or phase that prevents a complete verdict.
        reason: DeferredReason,
    },
    /// Visibility or requiredness depends on unresolved context.
    Policy {
        /// Field whose conditional policy awaits input.
        path: ValuePath,
    },
}

impl PendingValidation {
    /// Data location whose proof is incomplete.
    #[must_use]
    pub fn path(&self) -> &ValuePath {
        match self {
            Self::Value { path } | Self::Rule { path, .. } | Self::Policy { path } => path,
        }
    }
}

pub(super) struct CheckResult {
    pub(super) report: ValidationReport,
    pub(super) pending: Vec<PendingValidation>,
    pub(super) predicate_context: Option<PredicateContext>,
}

struct Checks {
    context: Option<PredicateContext>,
    mode: ExecutionMode,
    result: CheckResult,
}

pub(super) fn validate_tree<E>(
    schema: &ValidSchema,
    values: &ValueTree<E>,
    expression_paths: &[ValuePath],
    mode: ExecutionMode,
    predicate_context: Option<PredicateContext>,
) -> CheckResult {
    let context = predicate_context.or_else(|| {
        schema.has_contextual_rules().then(|| {
            crate::context::prepared_predicate_context(schema.fields(), values)
                .with_pending_paths(expression_paths.iter().cloned())
        })
    });
    let mut checks = Checks {
        context,
        mode,
        result: CheckResult {
            report: ValidationReport::new(),
            pending: Vec::new(),
            predicate_context: None,
        },
    };
    let path = ValuePath::root();
    match schema.root_shape() {
        RootShape::Any => {},
        RootShape::Scalar(scalar) => {
            if let Err(error) = scalar.validate_value(values, &path) {
                checks.result.report.push(error);
            }
        },
        RootShape::Record(_) | RootShape::Union(_) => match values {
            ValueTree::Object(values) => checks.level(
                schema
                    .fields()
                    .iter()
                    .map(|field| Entry {
                        field,
                        value: values.get(field.key().as_str()),
                        path: path.push(field.key().as_str()),
                    })
                    .collect(),
            ),
            _ => checks.type_error(&path, "object"),
        },
    }
    checks.rules(schema.root_rules(), values, &path, schema.fields());
    checks.result.predicate_context = checks.context.take();
    checks.result
}

struct Entry<'a, E> {
    field: &'a Field,
    value: Option<&'a ValueTree<E>>,
    path: ValuePath,
}

impl Checks {
    fn level<E>(&mut self, entries: Vec<Entry<'_, E>>) {
        let resolution = resolve_field_policies(
            entries.iter().map(|entry| {
                FieldPolicyDecl::new(
                    &entry.path,
                    match entry.field.visible() {
                        VisibilityMode::Always => VisibilityPolicy::Always,
                        VisibilityMode::Never => VisibilityPolicy::Never,
                        VisibilityMode::When(rule) => VisibilityPolicy::When(rule),
                    },
                    match entry.field.required() {
                        RequiredMode::Never => RequiredPolicy::Optional,
                        RequiredMode::Always => RequiredPolicy::Always,
                        RequiredMode::When(rule) => RequiredPolicy::When(rule),
                    },
                    !absent_for_required(entry.field, entry.value),
                    entry.value.is_some(),
                    entry,
                )
            }),
            predicate_context(self.context.as_ref()),
        );
        let resolution = match resolution {
            Ok(resolution) => resolution,
            Err(errors) => {
                merge_validator_errors(&errors, &ValuePath::root(), &mut self.result.report);
                return;
            },
        };
        merge_validator_errors(
            &resolution.required_failures,
            &ValuePath::root(),
            &mut self.result.report,
        );
        for plan in resolution.plans {
            let entry = plan.payload;
            tracing::trace!(target: "nebula_schema::validate", path = %entry.path,
                directive = ?plan.directive, "field policy resolved");
            match plan.directive {
                FieldDirective::Skip | FieldDirective::RequiredAbsent => {},
                FieldDirective::Deferred => {
                    self.result.pending.push(PendingValidation::Policy {
                        path: entry.path.clone(),
                    });
                    if let Some(value) = entry.value {
                        self.field(entry.field, value, &entry.path);
                    }
                },
                _ => {
                    if let Some(value) = entry.value {
                        self.field(entry.field, value, &entry.path);
                    }
                },
            }
        }
    }

    fn field<E>(&mut self, field: &Field, value: &ValueTree<E>, path: &ValuePath) {
        if matches!(value, ValueTree::Expression(_)) {
            self.result
                .pending
                .push(PendingValidation::Value { path: path.clone() });
            return;
        }
        match field {
            Field::String(_) | Field::Code(_) => {
                if value.as_str().is_none() {
                    self.type_error(path, "string");
                    return;
                }
            },
            Field::Secret(_) => {
                if !matches!(value, ValueTree::Secret(_)) {
                    self.type_error(path, "secret string");
                    return;
                }
            },
            Field::Number(number) => {
                let Some(Value::Number(value)) = value.as_literal() else {
                    self.type_error(path, "number");
                    return;
                };
                if number.integer
                    && !(value.is_i64()
                        || value.is_u64()
                        || value.as_f64().is_some_and(|number| number.fract() == 0.0))
                {
                    self.type_error(path, "integer");
                    return;
                }
            },
            Field::Boolean(_) => {
                if !value.as_literal().is_some_and(Value::is_boolean) {
                    self.type_error(path, "boolean");
                    return;
                }
            },
            Field::Object(object) => {
                let ValueTree::Object(values) = value else {
                    self.type_error(path, "object");
                    return;
                };
                self.level(
                    object
                        .fields
                        .iter()
                        .map(|field| Entry {
                            field,
                            value: values.get(field.key().as_str()),
                            path: path.push(field.key().as_str()),
                        })
                        .collect(),
                );
            },
            Field::List(list) => self.list(list, value, path),
            Field::Mode(mode) => self.mode(mode, value, path),
            Field::Select(select) => {
                if select.multiple != matches!(value, ValueTree::List(_)) {
                    self.type_error(
                        path,
                        if select.multiple {
                            "array of options"
                        } else {
                            "single option"
                        },
                    );
                    return;
                }
                if !select.allow_custom {
                    match RuleInput::new(value) {
                        Ok(input) => check_select_options(
                            &select.options,
                            select.multiple,
                            &input.0,
                            path,
                            &mut self.result.report,
                        ),
                        Err(error) => self.result.report.push(error.at(path.clone())),
                    }
                }
            },
            Field::File(file) => {
                let correct = if file.multiple {
                    matches!(value, ValueTree::List(items) if items.iter().all(|value| value.as_str().is_some()))
                } else {
                    value.as_str().is_some()
                };
                if !correct {
                    self.type_error(
                        path,
                        if file.multiple {
                            "array of file paths"
                        } else {
                            "file path"
                        },
                    );
                    return;
                }
            },
            Field::Computed(_) | Field::Dynamic(_) | Field::Notice(_) | Field::Unknown(_) => {},
        }
        self.rules(field.rules(), value, path, std::slice::from_ref(field));
    }

    fn list<E>(&mut self, list: &crate::ListField, value: &ValueTree<E>, path: &ValuePath) {
        let ValueTree::List(items) = value else {
            self.type_error(path, "array");
            return;
        };
        if let Some(minimum) = list.min_items
            && items.len() < minimum as usize
        {
            self.result.report.push(
                ValidationError::builder("items.min")
                    .at(path.clone())
                    .param("min", minimum)
                    .param("actual", items.len())
                    .message("array has too few items")
                    .build(),
            );
        }
        if let Some(maximum) = list.max_items
            && items.len() > maximum as usize
        {
            self.result.report.push(
                ValidationError::builder("items.max")
                    .at(path.clone())
                    .param("max", maximum)
                    .param("actual", items.len())
                    .message("array has too many items")
                    .build(),
            );
        }
        if list.unique {
            if contains_expression(value) {
                self.result
                    .pending
                    .push(PendingValidation::Value { path: path.clone() });
            } else {
                match first_duplicate_index(items) {
                    Ok(Some(index)) => self.result.report.push(
                        ValidationError::builder("items.unique")
                            .at(path.push(index.to_string()))
                            .param("index", index)
                            .message("array items must be unique")
                            .build(),
                    ),
                    Ok(None) => {},
                    Err(error) => self.result.report.push(error.at(path.clone())),
                }
            }
        }
        if let Some(field) = list.item.as_deref() {
            self.level(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, value)| Entry {
                        field,
                        value: Some(value),
                        path: path.push(index.to_string()),
                    })
                    .collect(),
            );
        }
    }

    fn mode<E>(&mut self, mode: &crate::ModeField, value: &ValueTree<E>, path: &ValuePath) {
        let ValueTree::Object(values) = value else {
            self.type_error(path, "mode envelope");
            return;
        };
        if values.keys().any(|key| key != "mode" && key != "value") {
            self.type_error(path, "closed mode envelope");
            return;
        }
        let selected = match values.get("mode") {
            Some(value) => {
                let Some(selected) = value.as_str() else {
                    self.type_error(&path.push("mode"), "string");
                    return;
                };
                Some(selected)
            },
            None => mode.default_variant.as_deref(),
        };
        let Some(selected) = selected else {
            self.result.report.push(
                ValidationError::builder("mode.required")
                    .at(path.clone())
                    .message("mode selector is required")
                    .build(),
            );
            return;
        };
        let Some(variant) = mode.variants.iter().find(|variant| variant.key == selected) else {
            self.result.report.push(
                ValidationError::builder("mode.invalid")
                    .at(path.clone())
                    .message("mode selector does not name a declared variant")
                    .build(),
            );
            return;
        };
        self.level(vec![Entry {
            field: &variant.field,
            value: values.get("value"),
            path: path.push("value"),
        }]);
    }

    #[tracing::instrument(level = "debug", skip_all, fields(rule_count = rules.len(), mode = ?self.mode, protected = tracing::field::Empty))]
    fn rules<E>(
        &mut self,
        rules: &[Rule],
        value: &ValueTree<E>,
        path: &ValuePath,
        declarations: &[Field],
    ) {
        if rules.is_empty() {
            return;
        }
        if contains_expression(value) {
            self.result
                .pending
                .push(PendingValidation::Value { path: path.clone() });
            return;
        }
        // A malformed or inactive secret-bearing subtree may have no Secret
        // tags. Declarations still govern disclosure and diagnostic custody.
        let protected = declarations
            .iter()
            .any(crate::context::field_subtree_has_secret)
            || value.first_secret_path().is_some();
        tracing::Span::current().record("protected", protected);
        let input = if protected {
            match secret_rule_access(rules, self.mode) {
                Ok(SecretRuleAccess::ContextOnly) => Ok(RuleInput(Value::Null)),
                Ok(SecretRuleAccess::Value) => RuleInput::new(value),
                Err(error) => Err(ValidationError::builder(error.code())
                    .message("rule execution is unavailable for protected values")
                    .private_source(error)
                    .build()),
            }
        } else {
            RuleInput::new(value)
        };
        let input = match input {
            Ok(input) => input,
            Err(error) => {
                self.result.report.push(error.at(path.clone()));
                return;
            },
        };
        match nebula_validator::validate_rules_with_ctx(
            &input.0,
            rules,
            self.context.as_ref(),
            self.mode,
            if protected {
                nebula_validator::DiagnosticDisclosure::OmitValue
            } else {
                nebula_validator::DiagnosticDisclosure::IncludeValue
            },
        ) {
            Ok(EvaluationOutcome::Satisfied) => {},
            Ok(EvaluationOutcome::Deferred(reasons)) => {
                self.result
                    .pending
                    .extend(reasons.into_iter().map(|reason| PendingValidation::Rule {
                        path: path.clone(),
                        reason,
                    }));
            },
            Err(errors) if protected => {
                // Move, rather than clone, payload-bearing causes into private custody.
                for error in errors {
                    self.result.report.push(
                        ValidationError::builder(error.code.to_string())
                            .at(validator_error_path(&error, path))
                            .message("secret value violates its validation rule")
                            .private_source(error)
                            .build(),
                    );
                }
            },
            Err(errors) => merge_validator_errors(&errors, path, &mut self.result.report),
        }
    }

    fn type_error(&mut self, path: &ValuePath, expected: &'static str) {
        self.result.report.push(
            ValidationError::builder("type_mismatch")
                .at(path.clone())
                .param("expected", expected)
                .message("value does not have the declared type")
                .build(),
        );
    }
}

fn predicate_context(context: Option<&PredicateContext>) -> &PredicateContext {
    static EMPTY_CONTEXT: LazyLock<PredicateContext> = LazyLock::new(PredicateContext::new);
    context.unwrap_or(&EMPTY_CONTEXT)
}

fn absent_for_required<E>(field: &Field, value: Option<&ValueTree<E>>) -> bool {
    let Some(value) = value else {
        return true;
    };
    if value.as_literal().is_some_and(Value::is_null) {
        return true;
    }
    match (field, value) {
        (Field::Secret(_), ValueTree::Secret(secret)) => secret.is_empty(),
        (Field::String(_) | Field::Secret(_) | Field::Code(_), _) => {
            value.as_str().is_some_and(str::is_empty)
        },
        (Field::File(file), _) if !file.multiple => value.as_str().is_some_and(str::is_empty),
        (Field::File(file), ValueTree::List(items)) if file.multiple => items.is_empty(),
        (Field::Select(select), ValueTree::List(items)) if select.multiple => items.is_empty(),
        (Field::List(_), ValueTree::List(items)) => items.is_empty(),
        _ => false,
    }
}

pub(super) fn contains_expression<E>(value: &ValueTree<E>) -> bool {
    match value {
        ValueTree::Expression(_) => true,
        ValueTree::List(values) => values.iter().any(contains_expression),
        ValueTree::Object(values) => values.values().any(contains_expression),
        _ => false,
    }
}

/// Public-data keys retain their hash-set fast path. Protected keys use an ordered
/// set: O(n log n) key comparisons in both average and worst cases, plus encoding
/// and per-item path sorting. Comparing keys costs their common-prefix length.
#[tracing::instrument(level = "debug", skip_all, fields(item_count = values.len()))]
fn first_duplicate_index<E>(values: &[ValueTree<E>]) -> Result<Option<usize>, ValidationError> {
    let mut canonical = HashSet::new();
    let mut protected = BTreeSet::new();
    let commitment_key = OnceCell::new();
    for (index, value) in values.iter().enumerate() {
        if value.first_secret_path().is_none() {
            if !canonical.insert(value.canonical_data_bytes()?) {
                return Ok(Some(index));
            }
            continue;
        }
        value.check_depth(&ValuePath::root(), 0)?;
        let key = commitment_key.get_or_init(CommitmentKey::ephemeral);
        // This transient index is not a wire/content ID. It never holds plaintext
        // secret bytes: the canonical shape is redacted and each leaf is keyed.
        let shape = crate::canonical_json_v1(&value.json_with(&|_| Value::Null))?;
        let commitments = secret_commitments(value, key);
        if !protected.insert((shape, commitments)) {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn secret_commitments<E>(value: &ValueTree<E>, key: &CommitmentKey) -> Vec<(String, Vec<u8>)> {
    let mut commitments = Vec::new();
    let mut pending = vec![(value, ValuePath::root())];
    while let Some((value, path)) = pending.pop() {
        match value {
            ValueTree::Secret(secret) => {
                let mut frame = Vec::new();
                write_secret_commitment(secret, key, &mut frame);
                commitments.push((path.to_string(), frame));
            },
            ValueTree::Object(values) => {
                pending.extend(values.iter().map(|(key, value)| (value, path.push(key))));
            },
            ValueTree::List(values) => pending.extend(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (value, path.push(index.to_string()))),
            ),
            _ => {},
        }
    }
    commitments.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    commitments
}

enum SecretRuleAccess {
    ContextOnly,
    Value,
}

#[derive(Debug, thiserror::Error)]
enum SecretRuleExecutionError {
    #[error("a deferred evaluator cannot receive protected input")]
    Deferred,
    #[error("the rule has not been audited for protected input")]
    Unsupported,
}

impl SecretRuleExecutionError {
    fn code(&self) -> &'static str {
        match self {
            Self::Deferred => "evaluation_unavailable",
            Self::Unsupported => "invalid_rule",
        }
    }
}

/// Audit the whole rule tree before disclosure, including branches under Any/Not.
/// StaticOnly never invokes deferred evaluators; Full refuses them before passing
/// the value to validator. New rule kinds fail closed until explicitly audited.
fn secret_rule_access(
    rules: &[Rule],
    mode: ExecutionMode,
) -> Result<SecretRuleAccess, SecretRuleExecutionError> {
    let mut access = SecretRuleAccess::ContextOnly;
    let mut pending: Vec<_> = rules.iter().map(Rule::root).collect();
    while let Some(rule) = pending.pop() {
        match rule.view() {
            nebula_validator::RuleView::Value(value) => {
                match value {
                    ValueRule::MinLength(_)
                    | ValueRule::MaxLength(_)
                    | ValueRule::Pattern(_)
                    | ValueRule::Min(_)
                    | ValueRule::Max(_)
                    | ValueRule::GreaterThan(_)
                    | ValueRule::LessThan(_)
                    | ValueRule::OneOf(_)
                    | ValueRule::MinItems(_)
                    | ValueRule::MaxItems(_)
                    | ValueRule::Email
                    | ValueRule::Url => {},
                    _ => return Err(SecretRuleExecutionError::Unsupported),
                }
                access = SecretRuleAccess::Value;
            },
            nebula_validator::RuleView::Predicate(_) => {},
            nebula_validator::RuleView::All(children)
            | nebula_validator::RuleView::Any(children) => pending.extend(children),
            nebula_validator::RuleView::Not(inner)
            | nebula_validator::RuleView::Described { inner, .. } => pending.push(inner),
            nebula_validator::RuleView::Deferred(_) if mode == ExecutionMode::StaticOnly => {},
            nebula_validator::RuleView::Deferred(_) => {
                return Err(SecretRuleExecutionError::Deferred);
            },
            _ => return Err(SecretRuleExecutionError::Unsupported),
        }
    }
    Ok(access)
}

/// A non-serializable temporary view for trusted value-rule execution only.
/// Owned JSON strings are wiped on normal return and unwinding. Validator-created
/// diagnostic buffers have separate ownership; private_source protects their API,
/// not their allocator residue. Neither this value nor its contents may be logged.
struct RuleInput(Value);

impl RuleInput {
    fn new<E>(value: &ValueTree<E>) -> Result<Self, ValidationError> {
        value.check_depth(&ValuePath::root(), 0)?;
        let mut input = Self(Value::Null);
        fill_rule_input(&mut input.0, value)?;
        Ok(input)
    }
}

impl std::fmt::Debug for RuleInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuleInput(<protected>)")
    }
}

impl Drop for RuleInput {
    fn drop(&mut self) {
        zeroize_json(&mut self.0);
    }
}

fn fill_rule_input<E>(output: &mut Value, value: &ValueTree<E>) -> Result<(), ValidationError> {
    match value {
        ValueTree::Literal(value) => *output = value.as_json().clone(),
        ValueTree::Secret(SecretValue::String(text)) => {
            *output = Value::String(text.expose().to_owned());
        },
        ValueTree::Secret(SecretValue::Bytes(bytes)) => {
            *output = Value::String(hex::encode(bytes.expose()));
        },
        ValueTree::Object(values) => {
            *output = Value::Object(serde_json::Map::new());
            if let Value::Object(output) = output {
                for (key, value) in values {
                    fill_rule_input(output.entry(key.clone()).or_insert(Value::Null), value)?;
                }
            }
        },
        ValueTree::List(values) => {
            *output = Value::Array(Vec::with_capacity(values.len()));
            if let Value::Array(output) = output {
                for value in values {
                    fill_rule_input(output.push_mut(Value::Null), value)?;
                }
            }
        },
        ValueTree::Expression(_) => {
            return Err(ValidationError::builder("expression.unresolved")
                .message("rule input must be resolved data")
                .build());
        },
    }
    Ok(())
}

fn zeroize_json(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json),
        Value::Object(values) => {
            for (mut key, mut value) in std::mem::take(values) {
                key.zeroize();
                zeroize_json(&mut value);
            }
        },
        _ => {},
    }
}

fn check_select_options(
    options: &[SelectOption],
    multiple: bool,
    value: &Value,
    path: &ValuePath,
    report: &mut ValidationReport,
) {
    let invalid = if multiple {
        value.as_array().and_then(|items| {
            items
                .iter()
                .position(|value| !options.iter().any(|option| option.value == *value))
        })
    } else {
        (!options.iter().any(|option| option.value == *value)).then_some(0)
    };
    if let Some(index) = invalid {
        let mut error = ValidationError::builder("option.invalid")
            .at(path.clone())
            .message("value is not in the allowed option set");
        if multiple {
            error = error.param("index", index);
        }
        report.push(error.build());
    }
}

/// Keep native rule codes and causes, using data pointers without lossy schema parsing.
pub(super) fn merge_validator_errors(
    errors: &ValidationErrors,
    fallback: &ValuePath,
    report: &mut ValidationReport,
) {
    for error in errors.errors() {
        report.push(
            ValidationError::builder(error.code.to_string())
                .at(validator_error_path(error, fallback))
                .message(error.message.to_string())
                .source(error.clone())
                .build(),
        );
    }
}

fn validator_error_path(
    error: &nebula_validator::foundation::ValidationError,
    fallback: &ValuePath,
) -> ValuePath {
    error
        .field_pointer()
        .as_deref()
        .and_then(|pointer| ValuePath::from_pointer(pointer).ok())
        .unwrap_or_else(|| fallback.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthoredValue;

    #[test]
    fn secret_rule_input_debug_is_redacted_and_owned_strings_are_wiped() {
        let mut value = AuthoredValue::object();
        value
            .insert(
                "token",
                AuthoredValue::Secret(SecretValue::string("PRIVATE_BUFFER".into())),
            )
            .unwrap();
        let mut input = RuleInput::new(&value).unwrap();
        assert_eq!(input.0, serde_json::json!({"token": "PRIVATE_BUFFER"}));
        assert_eq!(format!("{input:?}"), "RuleInput(<protected>)");
        zeroize_json(&mut input.0);
        assert_eq!(input.0, serde_json::json!({}));
        let mut list = serde_json::json!(["PRIVATE_BUFFER", ["PRIVATE_BUFFER"]]);
        zeroize_json(&mut list);
        assert_eq!(list, serde_json::json!(["", [""]]));
    }

    #[test]
    fn secret_rule_input_refuses_expression_placeholders() {
        let value = AuthoredValue::Expression(crate::Expression::new("PRIVATE_PROGRAM"));
        let error = RuleInput::new(&value).unwrap_err();
        assert_eq!(error.code(), "expression.unresolved");
        assert!(!format!("{error:?}").contains("PRIVATE_PROGRAM"));
    }
}
