pub mod core;
pub mod error;
pub mod types;

// Re-export core functionality
pub use core::*;

// Re-export parameter types
pub use types::*;

// Re-export key types from nebula-core
pub use nebula_core::prelude::{KeyParseError, ParameterKey};

// Re-export conversion traits from nebula-value
pub use nebula_value::{JsonValueExt, ValueRefExt};

// Re-export MaybeExpression from nebula-expression
pub use nebula_expression::MaybeExpression;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::core::{
        DisplayCondition, DisplayContext, DisplayRule, DisplayRuleSet, Displayable, Parameter,
        ParameterCollection, ParameterDisplay, ParameterDisplayError, ParameterError,
        ParameterKind, ParameterMetadata, ParameterSnapshot, ParameterValidation, ParameterValues,
        Validatable,
    };
    pub use nebula_value::ValueKind;

    pub use crate::types::{
        CheckboxParameter, CheckboxParameterOptions, CodeLanguage, CodeParameter,
        CodeParameterOptions, ColorFormat, ColorParameter, ColorParameterOptions, DateParameter,
        DateParameterOptions, DateTimeParameter, DateTimeParameterOptions, ExpirableParameter,
        ExpirableParameterOptions, ExpirableValue, FileParameter, FileParameterOptions,
        FileReference, GroupField, GroupFieldType, GroupParameter, GroupParameterOptions,
        GroupValue, HiddenParameter, ListParameter, ListParameterOptions, ListValue, ModeItem,
        ModeParameter, ModeValue, MultiSelectParameter, MultiSelectParameterOptions,
        NoticeParameter, NoticeParameterOptions, NoticeType, NumberParameter,
        NumberParameterOptions, ObjectParameter, ObjectParameterOptions, ObjectValue, Panel,
        PanelParameter, PanelParameterOptions, RadioParameter, RadioParameterOptions,
        ResourceContext, ResourceLoader, ResourceParameter, ResourceValue, RoutingParameter,
        RoutingParameterOptions, RoutingValue, SecretParameter, SecretParameterOptions,
        SelectParameter, SelectParameterOptions, TextParameter, TextParameterOptions,
        TextareaParameter, TextareaParameterOptions, TimeParameter, TimeParameterOptions,
    };

    pub use nebula_core::prelude::{KeyParseError, ParameterKey};
}
