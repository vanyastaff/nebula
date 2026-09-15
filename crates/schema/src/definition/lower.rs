use std::collections::BTreeSet;

use serde_json::Number;

use crate::{
    ExpressionMode, Property, ScalarSchema, Schema, ValidSchema, ValidationError, ValidationReport,
};

use super::{
    admission::AdmittedSchemaGraph,
    model::{
        AcceptedDomain, AdditionalProperties, Body, DefinitionIndex, DirectionalAliases,
        EmptyPolicy, NullPolicy, NumericBody, PresencePolicy, PropertyUse, UseSiteCore,
        ValueProtection,
    },
};

impl AdmittedSchemaGraph {
    /// Lower this admitted definition graph to the legacy schema model when the
    /// graph is exactly representable.
    ///
    /// Valid but unrepresentable documents fail closed with
    /// `schema.graph.lower.*` diagnostics.
    pub fn lower_to_valid_schema(&self) -> Result<ValidSchema, ValidationReport> {
        Lowerer { graph: self }.lower()
    }
}

struct Lowerer<'a> {
    graph: &'a AdmittedSchemaGraph,
}

impl Lowerer<'_> {
    fn lower(&self) -> Result<ValidSchema, ValidationReport> {
        if self.has_recursive_reachable_definition() {
            return Err(LowerIssue::CycleOrRecursive.into_report());
        }
        let root_index = self.definition_index(&self.graph.0.graph.root.0)?;
        let root = &self.graph.0.graph.definitions[root_index.0];
        match &root.body {
            Body::Any => {
                self.check_root_use_site(RootPolicy::Any)?;
                Ok(ValidSchema::any())
            },
            Body::Null => {
                self.check_root_use_site(RootPolicy::Null)?;
                ValidSchema::scalar(ScalarSchema::null())
            },
            Body::Boolean { intrinsic_rules } => {
                self.check_root_use_site(RootPolicy::NonNullScalar)?;
                self.check_intrinsic_rules(intrinsic_rules)?;
                ValidSchema::scalar(ScalarSchema::boolean())
            },
            Body::String { intrinsic_rules } => {
                self.check_root_use_site(RootPolicy::NonNullScalar)?;
                self.check_intrinsic_rules(intrinsic_rules)?;
                ValidSchema::scalar(ScalarSchema::string())
            },
            Body::Integer(numeric) => {
                self.check_root_use_site(RootPolicy::NonNullScalar)?;
                let scalar = self.integer_scalar(numeric)?;
                ValidSchema::scalar(scalar)
            },
            Body::Number(numeric) => {
                self.check_root_use_site(RootPolicy::NonNullScalar)?;
                let scalar = self.number_scalar(numeric)?;
                ValidSchema::scalar(scalar)
            },
            Body::Record {
                properties,
                additional_properties,
                intrinsic_rules,
            } => {
                self.check_root_use_site(RootPolicy::Record)?;
                self.lower_record(properties, additional_properties, intrinsic_rules)
            },
            Body::Bytes => Err(LowerIssue::Bytes.into_report()),
            Body::Array(_) => Err(LowerIssue::RootArray.into_report()),
            Body::Union(_) => Err(LowerIssue::Union.into_report()),
            Body::Alias(_) => Err(LowerIssue::AliasBody.into_report()),
        }
    }

    fn check_root_use_site(&self, policy: RootPolicy) -> Result<(), ValidationReport> {
        let root = &self.graph.0.graph.root.0;
        check_common_use_site(root)?;
        match policy {
            RootPolicy::Any => {
                if !matches!(root.null, NullPolicy::Allow) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
            },
            RootPolicy::Null => {
                if !matches!(root.null, NullPolicy::Allow) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
            },
            RootPolicy::NonNullScalar | RootPolicy::Record => {
                if !matches!(root.null, NullPolicy::Reject) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
            },
        }
        if !matches!(root.empty_string, EmptyPolicy::Allow)
            || !matches!(root.empty_collection, EmptyPolicy::Allow)
        {
            return Err(LowerIssue::Occurrence.into_report());
        }
        Ok(())
    }

    fn lower_record(
        &self,
        properties: &[PropertyUse],
        additional_properties: &AdditionalProperties,
        intrinsic_rules: &[nebula_validator::Rule],
    ) -> Result<ValidSchema, ValidationReport> {
        match additional_properties {
            AdditionalProperties::Open => {},
            AdditionalProperties::Closed => {
                return Err(LowerIssue::ClosedAdditionalProperties.into_report());
            },
            AdditionalProperties::Typed(_) => {
                return Err(LowerIssue::TypedAdditionalProperties.into_report());
            },
        }
        self.check_intrinsic_rules(intrinsic_rules)?;
        let mut builder = Schema::builder();
        for property in properties {
            builder = builder.property(self.lower_property(property)?);
        }
        builder.build()
    }

    fn lower_property(&self, property: &PropertyUse) -> Result<Property, ValidationReport> {
        if !matches!(property.presence, PresencePolicy::Required) {
            return Err(LowerIssue::Occurrence.into_report());
        }
        if property.input_default.is_some() {
            return Err(LowerIssue::Default.into_report());
        }
        if !aliases_are_empty(&property.aliases) {
            return Err(LowerIssue::Occurrence.into_report());
        }
        check_common_use_site(&property.core)?;
        if !matches!(property.core.null, NullPolicy::Reject)
            || !matches!(property.core.empty_collection, EmptyPolicy::Allow)
        {
            return Err(LowerIssue::Occurrence.into_report());
        }
        let definition_index = self.definition_index(&property.core)?;
        let definition = &self.graph.0.graph.definitions[definition_index.0];
        match &definition.body {
            Body::Boolean { intrinsic_rules } => {
                self.check_intrinsic_rules(intrinsic_rules)?;
                if !matches!(property.core.empty_string, EmptyPolicy::Allow) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
                Ok(Property::boolean(property.key.clone())
                    .required()
                    .into_property())
            },
            Body::String { intrinsic_rules } => {
                self.check_intrinsic_rules(intrinsic_rules)?;
                if !matches!(property.core.empty_string, EmptyPolicy::Reject) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
                Ok(Property::string(property.key.clone())
                    .required()
                    .no_expression()
                    .into_property())
            },
            Body::Integer(numeric) => {
                self.check_intrinsic_rules(&numeric.intrinsic_rules)?;
                if !matches!(property.core.empty_string, EmptyPolicy::Allow) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
                let (minimum, maximum) = numeric_bounds(numeric)?;
                Ok(Property::integer(property.key.clone())
                    .required()
                    .no_expression()
                    .min(minimum)
                    .max(maximum)
                    .into_property())
            },
            Body::Number(numeric) => {
                self.check_intrinsic_rules(&numeric.intrinsic_rules)?;
                if !matches!(property.core.empty_string, EmptyPolicy::Allow) {
                    return Err(LowerIssue::Occurrence.into_report());
                }
                let (minimum, maximum) = numeric_bounds(numeric)?;
                Ok(Property::number(property.key.clone())
                    .required()
                    .no_expression()
                    .min(minimum)
                    .max(maximum)
                    .into_property())
            },
            Body::Any | Body::Null => Err(LowerIssue::Occurrence.into_report()),
            Body::Bytes => Err(LowerIssue::Bytes.into_report()),
            Body::Record { .. } | Body::Array(_) => Err(LowerIssue::NestedContainer.into_report()),
            Body::Union(_) => Err(LowerIssue::Union.into_report()),
            Body::Alias(_) => Err(LowerIssue::AliasBody.into_report()),
        }
    }

    fn integer_scalar(&self, numeric: &NumericBody) -> Result<ScalarSchema, ValidationReport> {
        self.check_intrinsic_rules(&numeric.intrinsic_rules)?;
        let (minimum, maximum) = numeric_bounds(numeric)?;
        ScalarSchema::integer(minimum, maximum).map_err(ValidationReport::from)
    }

    fn number_scalar(&self, numeric: &NumericBody) -> Result<ScalarSchema, ValidationReport> {
        self.check_intrinsic_rules(&numeric.intrinsic_rules)?;
        let (minimum, maximum) = numeric_bounds(numeric)?;
        ScalarSchema::number(minimum, maximum).map_err(ValidationReport::from)
    }

    fn check_intrinsic_rules(
        &self,
        intrinsic_rules: &[nebula_validator::Rule],
    ) -> Result<(), ValidationReport> {
        if intrinsic_rules.is_empty() {
            Ok(())
        } else {
            Err(LowerIssue::Rules.into_report())
        }
    }

    fn definition_index(
        &self,
        use_site: &UseSiteCore,
    ) -> Result<DefinitionIndex, ValidationReport> {
        self.graph
            .0
            .lookup
            .get(&use_site.target)
            .copied()
            .ok_or_else(|| LowerIssue::NestedContainer.into_report())
    }

    fn has_recursive_reachable_definition(&self) -> bool {
        let Some(root_index) = self.graph.0.lookup.get(&self.graph.0.graph.root.0.target) else {
            return false;
        };
        let mut active_path = BTreeSet::new();
        let mut finished = BTreeSet::new();
        self.visits_active_definition(*root_index, &mut active_path, &mut finished)
    }

    fn visits_active_definition(
        &self,
        definition_index: DefinitionIndex,
        active_path: &mut BTreeSet<DefinitionIndex>,
        finished: &mut BTreeSet<DefinitionIndex>,
    ) -> bool {
        if finished.contains(&definition_index) {
            return false;
        }
        if !active_path.insert(definition_index) {
            return true;
        }
        let definition = &self.graph.0.graph.definitions[definition_index.0];
        for target in referenced_definitions(&definition.body) {
            let Some(target_index) = self.graph.0.lookup.get(target) else {
                continue;
            };
            if self.visits_active_definition(*target_index, active_path, finished) {
                return true;
            }
        }
        active_path.remove(&definition_index);
        finished.insert(definition_index);
        false
    }
}

#[derive(Debug, Clone, Copy)]
enum RootPolicy {
    Any,
    Null,
    NonNullScalar,
    Record,
}

#[derive(Debug, Clone, Copy)]
enum LowerIssue {
    RootArray,
    Bytes,
    Union,
    AliasBody,
    NestedContainer,
    ClosedAdditionalProperties,
    TypedAdditionalProperties,
    CycleOrRecursive,
    Default,
    AcceptedDomain,
    Protection,
    Rules,
    Transformers,
    Expression,
    Occurrence,
}

impl LowerIssue {
    const fn code(self) -> &'static str {
        match self {
            Self::RootArray => "schema.graph.lower.root_array",
            Self::Bytes => "schema.graph.lower.bytes",
            Self::Union => "schema.graph.lower.union",
            Self::AliasBody => "schema.graph.lower.alias_body",
            Self::NestedContainer => "schema.graph.lower.nested_container",
            Self::ClosedAdditionalProperties => "schema.graph.lower.closed_additional_properties",
            Self::TypedAdditionalProperties => "schema.graph.lower.typed_additional_properties",
            Self::CycleOrRecursive => "schema.graph.lower.cycle_or_recursive",
            Self::Default => "schema.graph.lower.default",
            Self::AcceptedDomain => "schema.graph.lower.accepted_domain",
            Self::Protection => "schema.graph.lower.protection",
            Self::Rules => "schema.graph.lower.rules",
            Self::Transformers => "schema.graph.lower.transformers",
            Self::Expression => "schema.graph.lower.expression",
            Self::Occurrence => "schema.graph.lower.occurrence",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::RootArray => "root arrays are not exactly representable by legacy ValidSchema",
            Self::Bytes => "bytes definitions are not exactly representable by legacy ValidSchema",
            Self::Union => "union definitions are not exactly representable by legacy ValidSchema",
            Self::AliasBody => {
                "alias definitions are not lowered until alias semantics are proven exact"
            },
            Self::NestedContainer => {
                "nested container properties are not exactly representable by this lowering packet"
            },
            Self::ClosedAdditionalProperties => {
                "closed additional-properties records are not exactly representable"
            },
            Self::TypedAdditionalProperties => {
                "typed additional-properties records are not exactly representable"
            },
            Self::CycleOrRecursive => {
                "recursive schema graphs are not exactly representable by legacy ValidSchema"
            },
            Self::Default => "input defaults are not lowered by this packet",
            Self::AcceptedDomain => "closed accepted domains are not lowered by this packet",
            Self::Protection => "non-public protection is not lowered by this packet",
            Self::Rules => "graph rules are not lowered by this packet",
            Self::Transformers => "graph transformers are not lowered by this packet",
            Self::Expression => "expression-admitting use sites are not lowered by this packet",
            Self::Occurrence => "occurrence policies are not exactly representable",
        }
    }

    fn into_report(self) -> ValidationReport {
        ValidationError::builder(self.code())
            .message(self.message())
            .build()
            .into()
    }
}

fn check_common_use_site(use_site: &UseSiteCore) -> Result<(), ValidationReport> {
    if !matches!(use_site.expression, ExpressionMode::Forbidden) {
        return Err(LowerIssue::Expression.into_report());
    }
    if !matches!(use_site.protection, ValueProtection::Public) {
        return Err(LowerIssue::Protection.into_report());
    }
    if !matches!(use_site.accepted_domain, AcceptedDomain::Open) {
        return Err(LowerIssue::AcceptedDomain.into_report());
    }
    if !use_site.rules.is_empty() {
        return Err(LowerIssue::Rules.into_report());
    }
    if !use_site.transformers.is_empty() {
        return Err(LowerIssue::Transformers.into_report());
    }
    Ok(())
}

fn aliases_are_empty(aliases: &DirectionalAliases) -> bool {
    aliases.read.is_empty() && aliases.write.is_none()
}

fn numeric_bounds(numeric: &NumericBody) -> Result<(Number, Number), ValidationReport> {
    let Some(minimum) = numeric.minimum.clone() else {
        return Err(LowerIssue::Occurrence.into_report());
    };
    let Some(maximum) = numeric.maximum.clone() else {
        return Err(LowerIssue::Occurrence.into_report());
    };
    Ok((minimum, maximum))
}

fn referenced_definitions(body: &Body) -> Vec<&super::model::DefinitionKey> {
    match body {
        Body::Record {
            properties,
            additional_properties,
            ..
        } => {
            let mut targets = properties
                .iter()
                .map(|property| &property.core.target)
                .collect::<Vec<_>>();
            if let AdditionalProperties::Typed(core) = additional_properties {
                targets.push(&core.target);
            }
            targets
        },
        Body::Array(array) => vec![&array.element.0.target],
        Body::Union(union) => union
            .variants
            .iter()
            .filter_map(|variant| variant.payload.as_ref().map(|payload| &payload.0.target))
            .collect(),
        Body::Alias(alias) => vec![&alias.0.target],
        Body::Any
        | Body::Null
        | Body::Boolean { .. }
        | Body::Integer(_)
        | Body::Number(_)
        | Body::String { .. }
        | Body::Bytes => Vec::new(),
    }
}
