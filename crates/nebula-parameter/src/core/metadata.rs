use bon::bon;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use nebula_core::ParameterKey;
use crate::core::ParameterError;


#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct ParameterMetadata {
    /// Unique identifier for the parameter
    pub key: ParameterKey,

    /// Human-readable name
    pub name: String,

    /// Detailed description of the parameter's purpose
    pub description: String,

    /// Whether this parameter must be provided
    #[serde(default)]
    pub required: bool,

    /// Placeholder text for UI inputs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Additional help text or usage hint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[bon]
impl ParameterMetadata {
    /// Основной метод создания с валидацией
    #[builder]
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        #[builder(default = false)]
        required: bool,
        placeholder: Option<String>,
        hint: Option<String>,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            key: ParameterKey::new(key.into())?,
            name: name.into(),
            description: description.into(),
            required,
            placeholder,
            hint,
        })
    }

    /// Быстрое создание обязательного параметра
    #[builder]
    pub fn required(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        placeholder: Option<String>,
        hint: Option<String>,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            key: ParameterKey::new(key.into())?,
            name: name.into(),
            description: description.into(),
            required: true,
            placeholder,
            hint,
        })
    }

    /// Быстрое создание опционального параметра
    #[builder]
    pub fn optional(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, ParameterError> {
        Ok(Self {
            key: ParameterKey::new(key.into())?,
            name: name.into(),
            description: description.into(),
            required: false,
            placeholder: None,
            hint: None,
        })
    }
}