//! Typed-closure builder for grouped properties with shared visible/required rules.
//!
//! A group is not a single [`Property`] — it is a collection
//! of sibling properties that share a common label prefix and common
//! `visible_when` / `required_when` conditions. At finish time each child
//! inherits the shared conditions (AND-composed with any per-child condition).

use nebula_validator::{Rule, RuleBuildError};

use crate::{
    builder::PropertyCollector,
    field::Property,
    mode::{RequiredMode, VisibilityMode},
};

/// Builder that accumulates grouped child properties with shared conditions.
pub struct GroupBuilder {
    name: String,
    visible_when: Option<Rule>,
    required_when: Option<Rule>,
    properties: Vec<Property>,
}

impl GroupBuilder {
    /// Start a new group with the given label.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible_when: None,
            required_when: None,
            properties: Vec::new(),
        }
    }

    /// Borrow the group label.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Require every child of this group when the given predicate holds.
    #[must_use]
    pub fn required_when(mut self, rule: Rule) -> Self {
        self.required_when = Some(rule);
        self
    }

    /// Show every child of this group only when the given predicate holds.
    #[must_use]
    pub fn visible_when(mut self, rule: Rule) -> Self {
        self.visible_when = Some(rule);
        self
    }

    /// Consume the group and return its child properties with shared conditions applied.
    ///
    /// # Errors
    /// Returns the exhausted rule budget when composing shared and child conditions.
    pub fn into_properties(self) -> Result<Vec<Property>, RuleBuildError> {
        let Self {
            name,
            visible_when,
            required_when,
            properties,
        } = self;
        properties
            .into_iter()
            .map(|field| apply_group(field, &name, visible_when.as_ref(), required_when.as_ref()))
            .collect()
    }
}

impl PropertyCollector for GroupBuilder {
    fn push_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }
}

/// Apply shared group label + shared visible/required conditions to a property.
fn apply_group(
    field: Property,
    group: &str,
    visible_when: Option<&Rule>,
    required_when: Option<&Rule>,
) -> Result<Property, RuleBuildError> {
    let mut field = field;
    set_group(&mut field, group);
    if let Some(rule) = visible_when {
        set_visible(&mut field, rule)?;
    }
    if let Some(rule) = required_when {
        set_required(&mut field, rule)?;
    }
    Ok(field)
}

/// Helper enum used by the three `set_*` functions — each match arm is fully
/// mechanical (apply the same mutation to every per-type inner struct).
macro_rules! for_each_field {
    ($field:expr, $mutation:expr) => {
        match $field {
            Property::String(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Secret(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Number(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Boolean(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Select(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Object(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::List(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Mode(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Code(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::File(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Computed(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Dynamic(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Property::Notice(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            // A forward-compat `Unknown` field has no typed `group` slot and is
            // never produced by the authoring builder (only by deserialization),
            // so group/visibility/required composition is a no-op for it.
            Property::Unknown(_) => {},
        }
    };
}

fn set_group(field: &mut Property, group: &str) {
    for_each_field!(field, |g: &mut Option<String>,
                            _v: &mut VisibilityMode,
                            _r: &mut RequiredMode| {
        if g.is_none() {
            *g = Some(group.to_owned());
        }
    });
}

fn set_visible(field: &mut Property, rule: &Rule) -> Result<(), RuleBuildError> {
    let mut result = Ok(());
    for_each_field!(field, |_g: &mut Option<String>,
                            v: &mut VisibilityMode,
                            _r: &mut RequiredMode| {
        if result.is_ok() {
            result = compose_visible(v.clone(), rule.clone()).map(|mode| *v = mode);
        }
    });
    result
}

fn set_required(field: &mut Property, rule: &Rule) -> Result<(), RuleBuildError> {
    let mut result = Ok(());
    for_each_field!(field, |_g: &mut Option<String>,
                            _v: &mut VisibilityMode,
                            r: &mut RequiredMode| {
        if result.is_ok() {
            result = compose_required(r.clone(), rule.clone()).map(|mode| *r = mode);
        }
    });
    result
}

fn compose_visible(
    existing: VisibilityMode,
    shared: Rule,
) -> Result<VisibilityMode, RuleBuildError> {
    Ok(match existing {
        VisibilityMode::Always => VisibilityMode::When(shared),
        VisibilityMode::Never => VisibilityMode::Never,
        VisibilityMode::When(child) => VisibilityMode::When(Rule::all([child, shared])?),
    })
}

fn compose_required(existing: RequiredMode, shared: Rule) -> Result<RequiredMode, RuleBuildError> {
    Ok(match existing {
        RequiredMode::Never => RequiredMode::When(shared),
        RequiredMode::Always => RequiredMode::Always,
        RequiredMode::When(child) => RequiredMode::When(Rule::all([child, shared])?),
    })
}
