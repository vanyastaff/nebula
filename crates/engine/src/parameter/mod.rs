pub use collection::ParameterCollection;
pub use condition::ParameterCondition;
pub use display::ParameterDisplay;
pub use error::ParameterError;
pub use metadata::ParameterMetadata;
pub use option::ParameterOption;
pub use parameter::Parameter;
pub use store::ParameterStore;
pub use r#type::ParameterType;
pub use validation::ParameterValidation;
pub use value::ParameterValue;

mod collection;
mod condition;
mod display;
mod error;
mod metadata;
mod option;
mod parameter;
mod store;
mod r#type;
mod types;
mod validation;
mod value;

pub fn validate_value(
    validation: Option<&ParameterValidation>,
    value: &ParameterValue,
) -> Result<(), ParameterError> {
    if let Some(validation) = validation {
        // Convert any ParameterValue to JSON for validation
        let json_value = match value {
            ParameterValue::Value(val) => val.clone(),
            _ => serde_json::to_value(value).map_err(ParameterError::SerializationError)?,
        };

        validation.validate(&json_value)
    } else {
        // No validation needed
        Ok(())
    }
}
