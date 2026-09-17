//! Conservative structural inhabitation shapes for admission checks.

use nebula_validator::{RuleRef, RuleView};

use super::super::model::{
    AdditionalProperties, AdmissionIssue, Body, DefinitionLookup, EmptyPolicy, NullPolicy,
    PresencePolicy, UseSiteCore,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct StructuralShape {
    can_be_null: bool,
    can_be_empty_collection: bool,
    can_be_other: bool,
}

impl StructuralShape {
    pub(super) const fn union(self, other: Self) -> Self {
        Self {
            can_be_null: self.can_be_null || other.can_be_null,
            can_be_empty_collection: self.can_be_empty_collection || other.can_be_empty_collection,
            can_be_other: self.can_be_other || other.can_be_other,
        }
    }

    pub(super) const fn is_productive(self) -> bool {
        self.can_be_null || self.can_be_empty_collection || self.can_be_other
    }
}

pub(super) fn body_shape(
    body: &Body,
    lookup: &DefinitionLookup,
    shapes: &[StructuralShape],
) -> Result<StructuralShape, AdmissionIssue> {
    match body {
        Body::Null => Ok(StructuralShape {
            can_be_null: true,
            ..StructuralShape::default()
        }),
        Body::Any => Ok(StructuralShape {
            can_be_null: true,
            can_be_empty_collection: true,
            can_be_other: true,
        }),
        Body::Boolean { .. }
        | Body::Integer(_)
        | Body::Number(_)
        | Body::String { .. }
        | Body::Bytes => Ok(StructuralShape {
            can_be_other: true,
            ..StructuralShape::default()
        }),
        Body::Record {
            properties,
            additional_properties,
            ..
        } => {
            let mut required_paths_productive = true;
            let mut has_required_property = false;
            let mut has_productive_property = false;
            for property in properties {
                let property_shape = occurrence_shape(&property.core, lookup, shapes)?;
                has_productive_property |= property_shape.is_productive();
                if presence_is_always_required(&property.presence)?
                    || property.input_default.is_some()
                {
                    has_required_property = true;
                    required_paths_productive &= property_shape.is_productive();
                }
            }
            let dynamic_property_productive = match additional_properties {
                AdditionalProperties::Open => true,
                AdditionalProperties::Closed => false,
                AdditionalProperties::Typed(core) => {
                    occurrence_shape(core, lookup, shapes)?.is_productive()
                },
            };
            Ok(StructuralShape {
                can_be_null: false,
                can_be_empty_collection: required_paths_productive && !has_required_property,
                can_be_other: required_paths_productive
                    && (has_required_property
                        || has_productive_property
                        || dynamic_property_productive),
            })
        },
        Body::Array(array) => Ok(StructuralShape {
            can_be_null: false,
            can_be_empty_collection: array.min_items == 0,
            can_be_other: array.max_items != Some(0)
                && occurrence_shape(&array.element.0, lookup, shapes)?.is_productive(),
        }),
        Body::Union(union) => {
            for variant in &union.variants {
                match &variant.payload {
                    None => {
                        return Ok(StructuralShape {
                            can_be_other: true,
                            ..StructuralShape::default()
                        });
                    },
                    Some(payload)
                        if occurrence_shape(&payload.0, lookup, shapes)?.is_productive() =>
                    {
                        return Ok(StructuralShape {
                            can_be_other: true,
                            ..StructuralShape::default()
                        });
                    },
                    Some(_) => {},
                }
            }
            Ok(StructuralShape::default())
        },
        Body::Alias(alias) => occurrence_shape(&alias.0, lookup, shapes),
    }
}

pub(super) fn occurrence_shape(
    use_site: &UseSiteCore,
    lookup: &DefinitionLookup,
    shapes: &[StructuralShape],
) -> Result<StructuralShape, AdmissionIssue> {
    let target = lookup
        .get(&use_site.target)
        .map(|index| shapes[index.0])
        .ok_or(AdmissionIssue::DanglingReference)?;
    Ok(StructuralShape {
        can_be_null: null_is_possible(&use_site.null)?,
        can_be_empty_collection: target.can_be_empty_collection
            && empty_collection_is_possible(&use_site.empty_collection)?,
        can_be_other: target.can_be_other,
    })
}

pub(super) fn presence_is_always_required(policy: &PresencePolicy) -> Result<bool, AdmissionIssue> {
    match policy {
        PresencePolicy::Required => Ok(true),
        PresencePolicy::Optional => Ok(false),
        PresencePolicy::RequiredWhen(rule) => {
            Ok(condition_truth(rule.root())? == ConditionTruth::AlwaysTrue)
        },
    }
}

fn null_is_possible(policy: &NullPolicy) -> Result<bool, AdmissionIssue> {
    match policy {
        NullPolicy::Allow => Ok(true),
        NullPolicy::Reject => Ok(false),
        NullPolicy::RejectWhen(rule) => {
            Ok(condition_truth(rule.root())? != ConditionTruth::AlwaysTrue)
        },
    }
}

fn empty_collection_is_possible(policy: &EmptyPolicy) -> Result<bool, AdmissionIssue> {
    match policy {
        EmptyPolicy::Allow => Ok(true),
        EmptyPolicy::Reject => Ok(false),
        EmptyPolicy::RejectWhen(rule) => {
            Ok(condition_truth(rule.root())? != ConditionTruth::AlwaysTrue)
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionTruth {
    AlwaysTrue,
    AlwaysFalse,
    Unknown,
}

fn condition_truth(rule: RuleRef<'_>) -> Result<ConditionTruth, AdmissionIssue> {
    match rule.view() {
        RuleView::Predicate(_) => Ok(ConditionTruth::Unknown),
        RuleView::All(children) => {
            let mut result = ConditionTruth::AlwaysTrue;
            for child in children {
                result = combine_all(result, condition_truth(child)?);
            }
            Ok(result)
        },
        RuleView::Any(children) => {
            let mut result = ConditionTruth::AlwaysFalse;
            for child in children {
                result = combine_any(result, condition_truth(child)?);
            }
            Ok(result)
        },
        RuleView::Not(inner) => Ok(match condition_truth(inner)? {
            ConditionTruth::AlwaysTrue => ConditionTruth::AlwaysFalse,
            ConditionTruth::AlwaysFalse => ConditionTruth::AlwaysTrue,
            ConditionTruth::Unknown => ConditionTruth::Unknown,
        }),
        RuleView::Described { inner, .. } => condition_truth(inner),
        RuleView::Value(_) | RuleView::Deferred(_) => Err(AdmissionIssue::InapplicableFacet),
        _ => Err(AdmissionIssue::InvalidRule),
    }
}

const fn combine_all(left: ConditionTruth, right: ConditionTruth) -> ConditionTruth {
    match (left, right) {
        (ConditionTruth::AlwaysFalse, _) | (_, ConditionTruth::AlwaysFalse) => {
            ConditionTruth::AlwaysFalse
        },
        (ConditionTruth::AlwaysTrue, ConditionTruth::AlwaysTrue) => ConditionTruth::AlwaysTrue,
        _ => ConditionTruth::Unknown,
    }
}

const fn combine_any(left: ConditionTruth, right: ConditionTruth) -> ConditionTruth {
    match (left, right) {
        (ConditionTruth::AlwaysTrue, _) | (_, ConditionTruth::AlwaysTrue) => {
            ConditionTruth::AlwaysTrue
        },
        (ConditionTruth::AlwaysFalse, ConditionTruth::AlwaysFalse) => ConditionTruth::AlwaysFalse,
        _ => ConditionTruth::Unknown,
    }
}
