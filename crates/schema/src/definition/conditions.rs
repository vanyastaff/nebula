//! Conditions admitted against the exact retained inbound graph.

use nebula_validator::{Condition, Predicate, Rule, RuleRef, RuleView};
use serde_json::Value;

use super::{
    InputContract, MAX_GRAPH_COMPARISON_STEPS,
    model::{Body, NullPolicy, UseSiteCore, ValueProtection},
};
use crate::{SerdeTagging, ValidationError, ValidationReport, ValuePath};

impl InputContract {
    /// Admit a condition whose references already use canonical inbound names.
    ///
    /// # Errors
    /// Rejects undeclared, protected, ambiguous, nonscalar and incompatible
    /// targets. Read aliases require [`Self::canonical_condition`] first.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn admit_condition(&self, condition: &Condition) -> Result<(), ValidationReport> {
        let canonical = self.canonical_condition(condition)?;
        if canonical != *condition {
            return Err(error("schema.condition.noncanonical_reference"));
        }
        Ok(())
    }

    /// Canonicalize read aliases and admit all condition references and operands.
    ///
    /// The returned condition reads the same canonical tree retained by input
    /// preparation. No value is read or executed during admission.
    ///
    /// # Errors
    /// Rejects unknown paths, protected ancestors, ambiguous domains and operands
    /// incompatible with the declared public scalar target.
    pub fn canonical_condition(
        &self,
        condition: &Condition,
    ) -> Result<Condition, ValidationReport> {
        let rule = rewrite(self, &self.graph.0.graph.root.0, condition.as_rule().root())?;
        Condition::try_from(rule).map_err(|_| error("schema.condition.invalid"))
    }

    /// Resolve a named declaration and admit it against this exact input graph.
    ///
    /// `x-nebula-conditions` retains a name-to-Rule declaration table. Stored
    /// rules are evidence; this method checks the subset, paths and domains
    /// before returning a condition usable by an owning leaf factory.
    ///
    /// # Errors
    /// Rejects missing names, malformed tables, unsupported rules and inadmissible
    /// predicate targets. Diagnostics contain no declaration or operand values.
    pub fn named_condition(&self, name: &str) -> Result<Condition, ValidationReport> {
        crate::FieldKey::new(name).map_err(|_| error("schema.condition.invalid_name"))?;
        let table = self
            .graph
            .0
            .document
            .raw()
            .get("x-nebula-conditions")
            .and_then(Value::as_object)
            .ok_or_else(|| error("schema.condition.unknown_name"))?;
        if table.len() > nebula_validator::MAX_RULE_NODES {
            return Err(error("schema.condition.table_budget"));
        }
        let declaration = table
            .get(name)
            .ok_or_else(|| error("schema.condition.unknown_name"))?;
        let condition: Condition = serde_json::from_value(declaration.clone())
            .map_err(|_| error("schema.condition.invalid"))?;
        self.canonical_condition(&condition)
    }
}

fn rewrite(
    contract: &InputContract,
    scope: &UseSiteCore,
    rule: RuleRef<'_>,
) -> Result<Rule, ValidationReport> {
    match rule.view() {
        RuleView::Predicate(predicate) => {
            Rule::predicate(admit_predicate(contract, scope, predicate)?)
                .map_err(|_| error("schema.condition.rule_budget"))
        },
        RuleView::All(children) => {
            let children = children
                .map(|child| rewrite(contract, scope, child))
                .collect::<Result<Vec<_>, _>>()?;
            Rule::all(children).map_err(|_| error("schema.condition.rule_budget"))
        },
        RuleView::Any(children) => {
            let children = children
                .map(|child| rewrite(contract, scope, child))
                .collect::<Result<Vec<_>, _>>()?;
            Rule::any(children).map_err(|_| error("schema.condition.rule_budget"))
        },
        RuleView::Not(inner) => Rule::not(rewrite(contract, scope, inner)?)
            .map_err(|_| error("schema.condition.rule_budget")),
        _ => Err(error("schema.condition.invalid")),
    }
}

fn admit_predicate(
    contract: &InputContract,
    scope: &UseSiteCore,
    predicate: &Predicate,
) -> Result<Predicate, ValidationReport> {
    let path = canonical_path(contract, scope, predicate.field())?;
    let reference = super::graph_compat::walk_reference_from(&contract.graph, scope, &path, true)
        .map_err(|_| error("schema.condition.undeclared_reference"))?;
    let (body, nullable) = scalar_body(contract, reference.core)?;
    if !matches!(
        body,
        Body::Null
            | Body::Boolean { .. }
            | Body::Integer(_)
            | Body::Number(_)
            | Body::String { .. }
    ) {
        return Err(error("schema.condition.nonscalar_reference"));
    }
    let compatible = |value: &Value| match value {
        Value::Null => nullable || matches!(body, Body::Null),
        Value::Bool(_) => matches!(body, Body::Boolean { .. }),
        Value::Number(number) => {
            matches!(body, Body::Number(_))
                || matches!(body, Body::Integer(_)) && (number.is_i64() || number.is_u64())
        },
        Value::String(_) => matches!(body, Body::String { .. }),
        Value::Array(_) | Value::Object(_) => false,
    };
    Ok(match predicate {
        Predicate::Eq(_, value) if compatible(value) => Predicate::Eq(path, value.clone()),
        Predicate::Ne(_, value) if compatible(value) => Predicate::Ne(path, value.clone()),
        Predicate::In(_, values) if !values.is_empty() && values.iter().all(compatible) => {
            Predicate::In(path, values.clone())
        },
        Predicate::Gt(_, number) if matches!(body, Body::Integer(_) | Body::Number(_)) => {
            Predicate::Gt(path, number.clone())
        },
        Predicate::Gte(_, number) if matches!(body, Body::Integer(_) | Body::Number(_)) => {
            Predicate::Gte(path, number.clone())
        },
        Predicate::Lt(_, number) if matches!(body, Body::Integer(_) | Body::Number(_)) => {
            Predicate::Lt(path, number.clone())
        },
        Predicate::Lte(_, number) if matches!(body, Body::Integer(_) | Body::Number(_)) => {
            Predicate::Lte(path, number.clone())
        },
        Predicate::IsTrue(_) if matches!(body, Body::Boolean { .. }) => Predicate::IsTrue(path),
        Predicate::IsFalse(_) if matches!(body, Body::Boolean { .. }) => Predicate::IsFalse(path),
        _ => return Err(error("schema.condition.incompatible_predicate")),
    })
}

fn scalar_body<'a>(
    contract: &'a InputContract,
    core: &'a UseSiteCore,
) -> Result<(&'a Body, bool), ValidationReport> {
    let mut current = core;
    // The outermost occurrence decides null, as runtime validation does.
    let nullable = matches!(core.null, NullPolicy::Allow);
    for _ in 0..MAX_GRAPH_COMPARISON_STEPS {
        if current.protection != ValueProtection::Public {
            return Err(error("schema.condition.protected_reference"));
        }
        let index = contract
            .graph
            .0
            .lookup
            .get(&current.target)
            .ok_or_else(|| error("schema.condition.undeclared_reference"))?;
        let body = &contract.graph.0.graph.definitions[index.0].body;
        match body {
            Body::Alias(alias) => current = &alias.0,
            _ => return Ok((body, nullable)),
        }
    }
    Err(error("schema.condition.reference_budget"))
}

fn canonical_path(
    contract: &InputContract,
    scope: &UseSiteCore,
    path: &ValuePath,
) -> Result<ValuePath, ValidationReport> {
    let mut authored = ValuePath::root();
    let mut canonical = ValuePath::root();
    let mut steps = 0;
    for encoded in path.as_str().split('/').skip(1) {
        steps += 1;
        if steps > MAX_GRAPH_COMPARISON_STEPS {
            return Err(error("schema.condition.reference_budget"));
        }
        let segment = encoded.replace("~1", "/").replace("~0", "~");
        let parent =
            super::graph_compat::walk_reference_from(&contract.graph, scope, &authored, true)
                .map_err(|_| error("schema.condition.undeclared_reference"))?;
        let (body, _) = scalar_body(contract, parent.core)?;
        let name = match body {
            Body::Record { properties, .. } => properties
                .iter()
                .find(|property| {
                    property.key.as_str() == segment
                        || property
                            .aliases
                            .read
                            .iter()
                            .any(|alias| alias.as_str() == segment)
                })
                .map(|property| property.key.as_str())
                .ok_or_else(|| error("schema.condition.undeclared_reference"))?,
            Body::Array(_) => segment.as_str(),
            Body::Union(union) => match &union.tagging {
                SerdeTagging::External => union
                    .variants
                    .iter()
                    .find(|variant| {
                        variant.key.as_str() == segment
                            || union.selector.aliases.iter().any(|(alias, key)| {
                                alias.as_str() == segment && key == &variant.key
                            })
                    })
                    .map(|variant| variant.key.as_str())
                    .ok_or_else(|| error("schema.condition.undeclared_reference"))?,
                SerdeTagging::Adjacent { content, .. } if content == &segment => segment.as_str(),
                _ => return Err(error("schema.condition.ambiguous_reference")),
            },
            _ => return Err(error("schema.condition.undeclared_reference")),
        };
        authored = authored.push(&segment);
        canonical = canonical.push(name);
        // Reuse graph-owned checks for nullable/protected ancestors, concrete
        // array indices and unambiguous union payloads on every traversed edge.
        super::graph_compat::walk_reference_from(&contract.graph, scope, &authored, true)
            .map_err(|_| error("schema.condition.undeclared_reference"))?;
    }
    Ok(canonical)
}

fn error(code: &'static str) -> ValidationReport {
    ValidationError::builder(code)
        .message("condition does not match the admitted public input contract")
        .build()
        .into()
}

/// Admit every behavior-bearing declaration, including unused nested scopes.
/// The empty scope identifies the published root table; nonempty scopes are
/// retained graph definition anchors. Conditions read canonical data relative
/// to their declared type, without creating another graph or input proof.
pub(super) fn checked_condition_inventory(
    contract: &InputContract,
) -> Result<Vec<(String, String, Condition)>, ValidationReport> {
    let document = contract.graph.0.document.raw();
    let mut inventory = Vec::new();
    if let Some(table) = document.get("x-nebula-conditions") {
        append_table(
            contract,
            "",
            &contract.graph.0.graph.root.0,
            table,
            &mut inventory,
        )?;
    }
    if let Some(scopes) = document.get("x-nebula-local-conditions") {
        let scopes = scopes
            .as_object()
            .ok_or_else(|| error("schema.condition.invalid_table"))?;
        if scopes.len() > super::MAX_GRAPH_DEFINITIONS {
            return Err(error("schema.condition.table_budget"));
        }
        let mut local_count = 0usize;
        for (anchor, table) in scopes {
            let target = super::DefinitionKey::new(anchor)
                .map_err(|_| error("schema.condition.invalid_scope"))?;
            if !contract.graph.0.lookup.contains_key(&target) {
                return Err(error("schema.condition.invalid_scope"));
            }
            let count = table
                .as_object()
                .ok_or_else(|| error("schema.condition.invalid_table"))?
                .len();
            local_count = local_count.saturating_add(count);
            if local_count > nebula_validator::MAX_RULE_NODES {
                return Err(error("schema.condition.table_budget"));
            }
            // A declaration root carries no parent occurrence authority. Its
            // properties and alias constraints remain those in the same graph.
            let scope = if target == contract.graph.0.graph.root.0.target {
                contract.graph.0.graph.root.0.clone()
            } else {
                UseSiteCore {
                    target,
                    null: NullPolicy::Reject,
                    empty_string: super::model::EmptyPolicy::Allow,
                    empty_collection: super::model::EmptyPolicy::Allow,
                    expression: crate::ExpressionMode::Allowed,
                    protection: ValueProtection::Public,
                    accepted_domain: super::model::AcceptedDomain::Open,
                    rules: Vec::new(),
                    transformers: Vec::new(),
                }
            };
            append_table(contract, anchor, &scope, table, &mut inventory)?;
        }
    }
    inventory.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    Ok(inventory)
}

fn append_table(
    contract: &InputContract,
    anchor: &str,
    scope: &UseSiteCore,
    table: &Value,
    inventory: &mut Vec<(String, String, Condition)>,
) -> Result<(), ValidationReport> {
    let table = table
        .as_object()
        .ok_or_else(|| error("schema.condition.invalid_table"))?;
    if table.len() > nebula_validator::MAX_RULE_NODES
        || inventory.len().saturating_add(table.len()) > 2 * nebula_validator::MAX_RULE_NODES
    {
        return Err(error("schema.condition.table_budget"));
    }
    for (name, declaration) in table {
        crate::FieldKey::new(name).map_err(|_| error("schema.condition.invalid_name"))?;
        let condition: Condition = serde_json::from_value(declaration.clone())
            .map_err(|_| error("schema.condition.invalid"))?;
        let rule = rewrite(contract, scope, condition.as_rule().root())?;
        let checked = Condition::try_from(rule).map_err(|_| error("schema.condition.invalid"))?;
        inventory.push((anchor.to_owned(), name.clone(), checked));
    }
    Ok(())
}
