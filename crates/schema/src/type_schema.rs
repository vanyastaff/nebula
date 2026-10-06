//! Type-driven construction of the single admitted schema graph.

use std::any::TypeId;
use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::{
    AdmittedSchemaGraph, HasSchema, Property, RequiredMode, SchemaGraphDocument, ValidationError,
    ValidationReport,
};

/// The wire direction of a schema-owned codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaDirection {
    /// Canonical names consumed by Deserialize.
    Input,
    /// Canonical names emitted by Serialize.
    Output,
}

/// A structural type description, independent of codec provenance.
pub trait PropertyType: 'static {
    /// Describe this type in the builder's wire direction.
    ///
    /// # Errors
    /// Returns malformed or unsupported declaration diagnostics.
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport>;

    /// Construct and admit the complete reachable graph for this type.
    ///
    /// # Errors
    /// Returns schema construction or admission diagnostics.
    fn definition(direction: SchemaDirection) -> Result<AdmittedSchemaGraph, ValidationReport> {
        let mut builder = SchemaTypeBuilder::new(direction);
        let root = Self::define_schema_type(&mut builder)?;
        builder.finish(root)
    }
}

/// Reviewed input codec fidelity. Schema alone does not imply this contract.
pub trait InputCodec: PropertyType + serde::de::DeserializeOwned {
    /// Exact canonical inbound graph.
    ///
    /// # Errors
    /// Returns construction or admission diagnostics.
    fn input_definition() -> Result<AdmittedSchemaGraph, ValidationReport> {
        Self::definition(SchemaDirection::Input)
    }
}

/// Reviewed output codec fidelity. It grants no admission authority.
pub trait OutputCodec: PropertyType + serde::Serialize {
    /// Exact canonical outbound graph.
    ///
    /// # Errors
    /// Returns construction or admission diagnostics.
    fn output_definition() -> Result<AdmittedSchemaGraph, ValidationReport> {
        Self::definition(SchemaDirection::Output)
    }
}

/// An authored use site. Only finishing the builder admits its graph.
#[derive(Debug, Clone)]
pub struct SchemaTypeUse(Value);

impl SchemaTypeUse {
    /// Append a rule on this authored occurrence, retaining its other facets.
    ///
    /// Contextual predicates belong to use sites rather than a definition's
    /// context-free intrinsic rules. Finishing the graph checks applicability
    /// and rule budgets before this occurrence gains executable authority.
    ///
    /// # Errors
    /// Returns a payload-free rule-encoding or malformed occurrence diagnostic.
    pub fn with_rule(mut self, rule: crate::Rule) -> Result<Self, ValidationReport> {
        let rule = encode(&rule)?;
        let object = self
            .0
            .as_object_mut()
            .ok_or_else(|| ValidationError::builder("schema.type.encoding").build())?;
        let rules = object.entry("rules").or_insert_with(|| json!([]));
        let rules = rules
            .as_array_mut()
            .ok_or_else(|| ValidationError::builder("schema.type.encoding").build())?;
        rules.push(rule);
        Ok(self)
    }

    /// Declare a protected UTF-8 occurrence, checked against its target at admission.
    #[must_use]
    pub fn secret_utf8(mut self) -> Self {
        self.0["protection"] = json!("secret_utf8");
        self
    }

    /// Declare a protected byte occurrence using canonical Base64 wire data.
    #[must_use]
    pub fn secret_bytes(mut self) -> Self {
        self.0["protection"] = json!("secret_bytes");
        self
    }
    /// Wire evidence for this authored occurrence.
    #[must_use]
    pub fn to_json(&self) -> Value {
        self.0.clone()
    }

    /// Describe Option's null representation without changing its child domain.
    #[must_use]
    pub fn allow_null(mut self) -> Self {
        self.0["null"] = json!("allow");
        self
    }

    /// Compose checked property facets with this typed occurrence.
    ///
    /// # Errors
    /// Returns payload-free encoding diagnostics; final graph admission checks
    /// applicability of every composed facet.
    pub fn with_property(self, property: Property) -> Result<Value, ValidationReport> {
        let mut value = self.0;
        value["key"] = json!(property.key().as_str());
        value["presence"] = match property.required() {
            RequiredMode::Always => json!("required"),
            RequiredMode::Never => json!("optional"),
            RequiredMode::When(rule) => json!({"required_when": rule}),
        };
        value["expression"] = encode(property.expression())?;
        let mut rules = property.rules().to_vec();
        if let Property::List(list) = &property {
            if let Some(minimum) = list.min_items {
                rules.push(crate::Rule::min_items(minimum as usize));
            }
            if let Some(maximum) = list.max_items {
                rules.push(crate::Rule::max_items(maximum as usize));
            }
            if list.unique {
                return Err(ValidationError::builder("schema.type.unsupported_facet")
                    .message("unique list facets require an exact typed graph declaration")
                    .build()
                    .into());
            }
        }
        if let Property::Select(select) = &property {
            if select.dynamic || select.loader.is_some() || select.multiple {
                return Err(ValidationError::builder("schema.type.unsupported_facet")
                    .message(
                        "dynamic and multi-select facets require an exact typed graph declaration",
                    )
                    .build()
                    .into());
            }
            if !select.allow_custom {
                let mut domain: Vec<_> = select
                    .options
                    .iter()
                    .map(|option| option.value.clone())
                    .collect();
                if value.get("null").and_then(Value::as_str) == Some("allow") {
                    domain.push(Value::Null);
                }
                value["accepted_domain"] = json!({"closed": domain});
            }
        }
        if value.get("rules").is_none() {
            value["rules"] = json!([]);
        }
        let occurrence_rules = value
            .get_mut("rules")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| ValidationError::builder("schema.type.encoding").build())?;
        for rule in rules {
            occurrence_rules.push(encode(&rule)?);
        }
        value["transformers"] = encode(property.transformers())?;
        value["aliases"] = json!({
            "read": property.read_aliases().iter().map(crate::FieldKey::as_str).collect::<Vec<_>>(),
            "write": property.emit_as().map(crate::FieldKey::as_str)
        });
        if let Some(default) = property.default() {
            value["input_default"] = default.clone();
        }
        if matches!(property, Property::Secret(_)) {
            value["protection"] = json!("secret_utf8");
        }
        value["x-nebula-property"] = encode(&property)?;
        Ok(value)
    }
}

fn encode<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value, ValidationReport> {
    serde_json::to_value(value).map_err(|_| {
        ValidationError::builder("schema.type.encoding")
            .message("schema type facets cannot be encoded")
            .build()
            .into()
    })
}

/// Authoring collector for recursively linked, direction-specific definitions.
pub struct SchemaTypeBuilder {
    direction: SchemaDirection,
    anchors: BTreeMap<(TypeId, Option<String>), SchemaTypeUse>,
    definitions: Vec<Value>,
    conditions: BTreeMap<String, nebula_validator::Condition>,
    local_conditions: BTreeMap<String, BTreeMap<String, nebula_validator::Condition>>,
    active_definitions: Vec<String>,
    condition_count: usize,
}

impl SchemaTypeBuilder {
    /// Start one directional declaration graph.
    #[must_use]
    pub fn new(direction: SchemaDirection) -> Self {
        Self {
            direction,
            anchors: BTreeMap::new(),
            definitions: Vec::new(),
            conditions: BTreeMap::new(),
            local_conditions: BTreeMap::new(),
            active_definitions: Vec::new(),
            condition_count: 0,
        }
    }

    /// Direction selected by the codec owner.
    #[must_use]
    pub const fn direction(&self) -> SchemaDirection {
        self.direction
    }

    /// Declare a named checked condition for inbound factory admission.
    ///
    /// # Errors
    /// Rejects invalid names, duplicate names and declaration budget overflow.
    pub fn named_condition(
        &mut self,
        name: &str,
        condition: nebula_validator::Condition,
    ) -> Result<(), ValidationReport> {
        if self.direction == SchemaDirection::Output {
            return Err(
                ValidationError::builder("schema.direction.unsupported_facet")
                    .message("outbound codec cannot execute named condition declarations")
                    .build()
                    .into(),
            );
        }
        crate::FieldKey::new(name).map_err(|_| {
            ValidationError::builder("schema.condition.invalid_name")
                .message("condition name is invalid")
                .build()
        })?;
        let table = match self.active_definitions.last() {
            Some(anchor) => self.local_conditions.entry(anchor.clone()).or_default(),
            None => &mut self.conditions,
        };
        if table.contains_key(name) || self.condition_count >= nebula_validator::MAX_RULE_NODES {
            return Err(ValidationError::builder("schema.condition.invalid_table")
                .message("condition declaration is duplicate or exceeds budget")
                .build()
                .into());
        }
        table.insert(name.to_owned(), condition);
        self.condition_count += 1;
        Ok(())
    }

    /// Compose authored facets in the selected codec direction.
    ///
    /// # Errors
    /// Rejects unsupported facets instead of dropping their semantics.
    pub fn property(
        &self,
        use_site: SchemaTypeUse,
        property: Property,
    ) -> Result<Value, ValidationReport> {
        let output_key = property
            .emit_as()
            .unwrap_or_else(|| property.key())
            .as_str()
            .to_owned();
        let mut value = use_site.with_property(property)?;
        if self.direction == SchemaDirection::Output {
            value["key"] = json!(output_key);
            value["presence"] = json!("required");
            value["expression"] = encode(&crate::ExpressionMode::Forbidden)?;
            value["aliases"] = json!({"read": [], "write": null});
            value["transformers"] = json!([]);
            if let Some(object) = value.as_object_mut() {
                object.remove("input_default");
            }
        }
        Ok(value)
    }

    /// Define a type once, registering its anchor before visiting its children.
    ///
    /// # Errors
    /// Returns graph-budget or child-description diagnostics.
    pub fn define<T: ?Sized + 'static>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<Value, ValidationReport>,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        self.define_named((TypeId::of::<T>(), None), body)
    }

    /// Define a named enum variant payload within its owner's type namespace.
    ///
    /// # Errors
    /// Returns graph-budget or child-description diagnostics.
    pub fn define_variant<T: ?Sized + 'static>(
        &mut self,
        variant: &str,
        body: impl FnOnce(&mut Self) -> Result<Value, ValidationReport>,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        self.define_named((TypeId::of::<T>(), Some(variant.to_owned())), body)
    }

    fn define_named(
        &mut self,
        name: (TypeId, Option<String>),
        body: impl FnOnce(&mut Self) -> Result<Value, ValidationReport>,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        if let Some(use_site) = self.anchors.get(&name) {
            return Ok(use_site.clone());
        }
        if self.anchors.len() >= crate::definition::MAX_GRAPH_DEFINITIONS {
            return Err(ValidationError::builder("schema.graph.definition_limit")
                .message("schema type graph exceeds its definition limit")
                .build()
                .into());
        }
        let key = format!("type_{}", self.anchors.len());
        self.anchors.insert(
            name.clone(),
            SchemaTypeUse(json!({"target": key, "null": "reject"})),
        );
        self.active_definitions.push(key.clone());
        let definition = body(self);
        self.active_definitions.pop();
        let definition = definition?;
        let allow_null = matches!(
            definition.get("kind").and_then(Value::as_str),
            Some("null" | "any")
        ) || definition.get("kind").and_then(Value::as_str) == Some("alias")
            && definition
                .get("alias")
                .and_then(|value| value.get("null"))
                .and_then(Value::as_str)
                == Some("allow");
        self.definitions
            .push(json!({"key": key, "body": definition}));
        let use_site = SchemaTypeUse(
            json!({"target": key, "null": if allow_null { "allow" } else { "reject" }}),
        );
        self.anchors.insert(name, use_site.clone());
        Ok(use_site)
    }

    /// Admit all collected definitions under the supplied root occurrence.
    ///
    /// # Errors
    /// Returns payload-free wire decoding or graph admission diagnostics.
    pub fn finish(mut self, root: SchemaTypeUse) -> Result<AdmittedSchemaGraph, ValidationReport> {
        if let Some(target) = root.0.get("target").and_then(Value::as_str)
            && let Some(table) = self.local_conditions.get(target)
        {
            for (name, condition) in table {
                if self
                    .conditions
                    .insert(name.clone(), condition.clone())
                    .is_some()
                {
                    return Err(ValidationError::builder("schema.condition.invalid_table")
                        .message("root condition declaration is duplicate")
                        .build()
                        .into());
                }
            }
        }
        let mut document = json!({
            "version": crate::definition::SCHEMA_GRAPH_WIRE_VERSION,
            "root": root.to_json(), "definitions": self.definitions
        });
        if self.direction == SchemaDirection::Input {
            document["x-nebula-conditions"] = encode(&self.conditions)?;
            document["x-nebula-local-conditions"] = encode(&self.local_conditions)?;
        }
        let document: SchemaGraphDocument = serde_json::from_value(document).map_err(|_| {
            ValidationError::builder("schema.type.encoding")
                .message("schema type graph cannot be decoded")
                .build()
        })?;
        document.admit().map_err(|error| error.report().clone())
    }
}

macro_rules! primitive_codec {
    ($($ty:ty => $body:expr),* $(,)?) => {$(
        impl PropertyType for $ty {
            fn define_schema_type(builder: &mut SchemaTypeBuilder) -> Result<SchemaTypeUse, ValidationReport> {
                builder.define::<Self>(|_| Ok($body))
            }
        }
        impl InputCodec for $ty {}
        impl OutputCodec for $ty {}
    )*};
}

primitive_codec!(
    bool => json!({"kind": "boolean"}),
    String => json!({"kind": "string"}),
    Value => json!({"kind": "any"}),
    i8 => json!({"kind": "integer", "minimum": i8::MIN, "maximum": i8::MAX}),
    i16 => json!({"kind": "integer", "minimum": i16::MIN, "maximum": i16::MAX}),
    i32 => json!({"kind": "integer", "minimum": i32::MIN, "maximum": i32::MAX}),
    i64 => json!({"kind": "integer", "minimum": i64::MIN, "maximum": i64::MAX}),
    u8 => json!({"kind": "integer", "minimum": 0, "maximum": u8::MAX}),
    u16 => json!({"kind": "integer", "minimum": 0, "maximum": u16::MAX}),
    u32 => json!({"kind": "integer", "minimum": 0, "maximum": u32::MAX}),
    u64 => json!({"kind": "integer", "minimum": 0, "maximum": u64::MAX}),
    f32 => json!({"kind": "number", "minimum": f32::MIN, "maximum": f32::MAX}),
    f64 => json!({"kind": "number", "minimum": f64::MIN, "maximum": f64::MAX}),
);

impl PropertyType for () {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder
            .define::<Self>(|_| Ok(json!({"kind": "null"})))
            .map(SchemaTypeUse::allow_null)
    }
}
impl InputCodec for () {}
impl OutputCodec for () {}

impl<T: PropertyType> PropertyType for Vec<T> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| {
            Ok(json!({"kind": "array", "element": T::define_schema_type(builder)?.to_json()}))
        })
    }
}

impl<T: PropertyType> PropertyType for Option<T> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        T::define_schema_type(builder).map(SchemaTypeUse::allow_null)
    }
}

impl<T: PropertyType> HasSchema for Vec<T> {
    fn schema() -> Result<crate::ValidSchema, ValidationReport> {
        crate::ValidSchema::from_graph(&Self::definition(SchemaDirection::Input)?)
    }
}
impl<T: PropertyType> HasSchema for Option<T> {
    fn schema() -> Result<crate::ValidSchema, ValidationReport> {
        crate::ValidSchema::from_graph(&Self::definition(SchemaDirection::Input)?)
    }
}
impl<T: InputCodec> InputCodec for Vec<T> {}
impl<T: OutputCodec> OutputCodec for Vec<T> {}
impl<T: InputCodec> InputCodec for Option<T> {}
impl<T: OutputCodec> OutputCodec for Option<T> {}

impl<T: PropertyType> PropertyType for Box<T> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        T::define_schema_type(builder)
    }
}
impl<T: InputCodec> InputCodec for Box<T> {}
impl<T: OutputCodec> OutputCodec for Box<T> {}

impl<T: PropertyType, const N: usize> PropertyType for [T; N] {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| Ok(json!({"kind": "array", "element": T::define_schema_type(builder)?.to_json(), "min_items": N, "max_items": N})))
    }
}
impl<T: InputCodec, const N: usize> InputCodec for [T; N] where Self: serde::de::DeserializeOwned {}
impl<T: OutputCodec, const N: usize> OutputCodec for [T; N] where Self: serde::Serialize {}
