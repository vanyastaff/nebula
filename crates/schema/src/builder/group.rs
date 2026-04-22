//! Typed-closure builder for grouped fields with shared visible/required rules.
//!
//! A group is not a single [`Field`](crate::field::Field) — it is a collection
//! of sibling fields that share a common label prefix and common
//! `visible_when` / `required_when` conditions. At finish time each child
//! inherits the shared conditions (AND-composed with any per-child condition).

use nebula_validator::Rule;

use crate::{
    builder::FieldCollector,
    field::Field,
    mode::{RequiredMode, VisibilityMode},
};

/// Builder that accumulates grouped child fields with shared conditions.
pub struct GroupBuilder {
    name: String,
    visible_when: Option<Rule>,
    required_when: Option<Rule>,
    fields: Vec<Field>,
}

impl GroupBuilder {
    /// Start a new group with the given label.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible_when: None,
            required_when: None,
            fields: Vec::new(),
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

    /// Consume the group and return its children with shared conditions applied.
    #[must_use]
    pub fn into_fields(self) -> Vec<Field> {
        let Self {
            name,
            visible_when,
            required_when,
            fields,
        } = self;
        fields
            .into_iter()
            .map(|field| apply_group(field, &name, visible_when.as_ref(), required_when.as_ref()))
            .collect()
    }
}

impl FieldCollector for GroupBuilder {
    fn push_field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }
}

/// Apply shared group label + shared visible/required conditions to a field.
fn apply_group(
    field: Field,
    group: &str,
    visible_when: Option<&Rule>,
    required_when: Option<&Rule>,
) -> Field {
    let mut field = field;
    set_group(&mut field, group);
    if let Some(rule) = visible_when {
        set_visible(&mut field, rule);
    }
    if let Some(rule) = required_when {
        set_required(&mut field, rule);
    }
    field
}

/// Helper enum used by the three `set_*` functions — each match arm is fully
/// mechanical (apply the same mutation to every per-type inner struct).
macro_rules! for_each_field {
    ($field:expr, $mutation:expr) => {
        match $field {
            Field::String(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Secret(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Number(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Boolean(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Select(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Object(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::List(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Mode(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Code(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::File(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Computed(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Dynamic(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
            Field::Notice(inner) => {
                $mutation(&mut inner.group, &mut inner.visible, &mut inner.required)
            },
        }
    };
}

fn set_group(field: &mut Field, group: &str) {
    for_each_field!(field, |g: &mut Option<String>,
                            _v: &mut VisibilityMode,
                            _r: &mut RequiredMode| {
        if g.is_none() {
            *g = Some(group.to_owned());
        }
    });
}

fn set_visible(field: &mut Field, rule: &Rule) {
    for_each_field!(field, |_g: &mut Option<String>,
                            v: &mut VisibilityMode,
                            _r: &mut RequiredMode| {
        *v = compose_visible(v.clone(), rule.clone());
    });
}

fn set_required(field: &mut Field, rule: &Rule) {
    for_each_field!(field, |_g: &mut Option<String>,
                            _v: &mut VisibilityMode,
                            r: &mut RequiredMode| {
        *r = compose_required(r.clone(), rule.clone());
    });
}

fn compose_visible(existing: VisibilityMode, shared: Rule) -> VisibilityMode {
    match existing {
        VisibilityMode::Always => VisibilityMode::When(shared),
        VisibilityMode::Never => VisibilityMode::Never,
        VisibilityMode::When(child) => VisibilityMode::When(Rule::all([child, shared])),
    }
}

fn compose_required(existing: RequiredMode, shared: Rule) -> RequiredMode {
    match existing {
        RequiredMode::Never => RequiredMode::When(shared),
        RequiredMode::Always => RequiredMode::Always,
        RequiredMode::When(child) => RequiredMode::When(Rule::all([child, shared])),
    }
}
