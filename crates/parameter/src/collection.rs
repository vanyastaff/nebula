//! Parameter collection — top-level container for parameter definitions.
//!
//! [`ParameterCollection`] is the v3 replacement for [`crate::schema::Schema`].
//! It holds an ordered list of [`Parameter`] definitions and provides
//! validation/normalization entry points (delegated to the validation and
//! normalization engines implemented in later tasks).
//!
//! UI elements and groups from v2 are replaced by first-class parameter
//! variants: [`ParameterType::Notice`] for inline messages and
//! [`DisplayMode::Sections`] for visual grouping.

use crate::error::ParameterError;
use crate::parameter::Parameter;
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;
use crate::values::ParameterValues;

/// Complete parameter collection for v3 authoring.
///
/// Replaces [`Schema`](crate::schema::Schema) from v2. Contains an ordered
/// list of [`Parameter`] definitions. UI elements and groups are now expressed
/// through `ParameterType::Notice` and `DisplayMode::Sections`.
///
/// # Examples
///
/// ```ignore
/// use nebula_parameter::collection::ParameterCollection;
/// use nebula_parameter::parameter::Parameter;
/// use serde_json::json;
///
/// let params = ParameterCollection::new()
///     .add(Parameter::string("name").label("Name").required())
///     .add(Parameter::integer("age").label("Age").default(json!(18)));
///
/// assert_eq!(params.len(), 2);
/// assert!(params.contains("name"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParameterCollection {
    /// Ordered parameter definitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
}

impl ParameterCollection {
    /// Creates an empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a parameter to the collection.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, param: Parameter) -> Self {
        self.parameters.push(param);
        self
    }

    /// Returns the number of parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Returns `true` if the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    /// Returns the parameter with the given id, if any.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.id == id)
    }

    /// Returns `true` if the collection contains a parameter with the given id.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.parameters.iter().any(|p| p.id == id)
    }

    /// Validates `values` against this collection using strict defaults.
    ///
    /// On success, returns [`ValidatedValues`](crate::runtime::ValidatedValues)
    /// — proof that values passed validation.
    ///
    /// # Errors
    ///
    /// Returns a non-empty list of [`ParameterError`] on failure.
    pub fn validate(
        &self,
        values: &ParameterValues,
    ) -> Result<crate::runtime::ValidatedValues, Vec<ParameterError>> {
        crate::validate::validate_parameters(&self.parameters, values)?;
        Ok(crate::runtime::ValidatedValues::new(values.clone()))
    }

    /// Validates `values` under the given [`ValidationProfile`].
    ///
    /// Returns a [`ValidationReport`] separating hard errors from warnings.
    #[must_use]
    pub fn validate_with_profile(
        &self,
        values: &ParameterValues,
        profile: ValidationProfile,
    ) -> ValidationReport {
        crate::validate::validate_with_profile(&self.parameters, values, profile)
    }

    /// Normalizes runtime values using schema defaults.
    ///
    /// Existing user-provided values are preserved. Missing fields are
    /// materialized from `default` metadata and mode default variants.
    #[must_use]
    pub fn normalize(&self, values: &ParameterValues) -> ParameterValues {
        crate::normalize::normalize_parameters(&self.parameters, values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter::Parameter;
    use serde_json::json;

    #[test]
    fn new_creates_empty_collection() {
        let coll = ParameterCollection::new();
        assert!(coll.is_empty());
        assert_eq!(coll.len(), 0);
    }

    #[test]
    fn add_appends_parameters_in_order() {
        let coll = ParameterCollection::new()
            .add(Parameter::string("first"))
            .add(Parameter::string("second"))
            .add(Parameter::integer("third"));

        assert_eq!(coll.len(), 3);
        assert!(!coll.is_empty());
        assert_eq!(coll.parameters[0].id, "first");
        assert_eq!(coll.parameters[1].id, "second");
        assert_eq!(coll.parameters[2].id, "third");
    }

    #[test]
    fn get_returns_matching_parameter() {
        let coll = ParameterCollection::new()
            .add(Parameter::string("name").label("Name"))
            .add(Parameter::integer("age").label("Age"));

        let param = coll.get("age").expect("should find 'age'");
        assert_eq!(param.id, "age");
        assert_eq!(param.label.as_deref(), Some("Age"));

        assert!(coll.get("missing").is_none());
    }

    #[test]
    fn contains_checks_presence() {
        let coll = ParameterCollection::new().add(Parameter::string("host"));

        assert!(coll.contains("host"));
        assert!(!coll.contains("port"));
    }

    #[test]
    fn default_is_empty() {
        let coll = ParameterCollection::default();
        assert!(coll.is_empty());
        assert_eq!(coll.len(), 0);
    }

    #[test]
    fn serde_round_trip_empty() {
        let coll = ParameterCollection::new();
        let json_str = serde_json::to_string(&coll).expect("serialize");
        let restored: ParameterCollection = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(coll, restored);
    }

    #[test]
    fn serde_round_trip_with_parameters() {
        let coll = ParameterCollection::new()
            .add(Parameter::string("name").label("Name").required())
            .add(Parameter::integer("age").label("Age").default(json!(18)));

        let json_str = serde_json::to_string_pretty(&coll).expect("serialize");
        let restored: ParameterCollection = serde_json::from_str(&json_str).expect("deserialize");

        assert_eq!(coll.len(), restored.len());
        assert_eq!(
            coll.get("name").map(|p| &p.id),
            restored.get("name").map(|p| &p.id)
        );
        assert_eq!(
            coll.get("age").map(|p| &p.id),
            restored.get("age").map(|p| &p.id)
        );
    }

    #[test]
    fn serde_empty_collection_omits_parameters_key() {
        let coll = ParameterCollection::new();
        let json_str = serde_json::to_string(&coll).expect("serialize");
        assert_eq!(json_str, "{}");
    }

    #[test]
    fn validate_no_required_returns_ok() {
        let coll = ParameterCollection::new().add(Parameter::string("name"));
        let values = ParameterValues::new();

        let result = coll.validate(&values);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_with_profile_no_issues_returns_ok() {
        let coll = ParameterCollection::new().add(Parameter::string("name"));
        let mut values = ParameterValues::new();
        values.set("name", json!("Alice"));

        let report = coll.validate_with_profile(&values, ValidationProfile::Strict);
        assert!(report.is_ok());
        assert!(!report.has_errors());
        assert!(!report.has_warnings());
    }

    #[test]
    fn normalize_preserves_existing_values() {
        let coll = ParameterCollection::new().add(Parameter::string("name"));

        let mut values = ParameterValues::new();
        values.set("name", json!("Alice"));

        let normalized = coll.normalize(&values);
        assert_eq!(normalized.get("name"), Some(&json!("Alice")));
    }
}
