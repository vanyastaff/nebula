pub mod core;
pub mod types;

// Re-export core functionality
pub use core::*;

// Re-export parameter types
pub use types::*;

// Re-export key types from nebula-core
pub use nebula_core::prelude::{KeyParseError, ParameterKey};

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::core::{
        DisplayContext, Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind,
        ParameterMetadata, ParameterType, ParameterValidation, ParameterValue, UiMode, Validatable,
    };

    pub use crate::types::{
        ButtonParameter, ButtonType, CheckboxParameter, CheckboxParameterOptions, CodeLanguage,
        CodeParameter, CodeParameterOptions, CodeTheme, ColorFormat, ColorParameter,
        ColorParameterOptions, DateParameter, DateParameterOptions, DateTimeParameter,
        DateTimeParameterOptions, ExpirableParameter, ExpirableParameterOptions, ExpirableValue,
        FileParameter, FileParameterOptions, FileReference, GroupField, GroupFieldType,
        GroupLabelPosition, GroupLayout, GroupParameter, GroupParameterOptions, GroupValue,
        HiddenParameter, ListLayout, ListParameter, ListParameterOptions, ModeItem, ModeParameter,
        ModeValue, MultiSelectParameter, MultiSelectParameterOptions, NoticeParameter,
        NoticeParameterOptions, NoticeType, ObjectLabelPosition, ObjectLayout, ObjectParameter,
        ObjectParameterOptions, ObjectValue, RadioLayoutDirection, RadioParameter,
        RadioParameterOptions, RoutingParameter, RoutingParameterOptions, RoutingValue,
        SecretParameter, SecretParameterOptions, SelectParameter, SelectParameterOptions,
        TextParameter, TextParameterOptions, TextareaParameter, TextareaParameterOptions,
        TimeParameter, TimeParameterOptions,
    };

    pub use nebula_core::prelude::{KeyParseError, ParameterKey};
}
