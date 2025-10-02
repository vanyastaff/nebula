pub mod core;
pub mod types;

// Re-export core functionality
pub use core::*;

// Re-export parameter types
pub use types::*;

// Re-export key types from nebula-core
pub use nebula_core::prelude::{ParameterKey, KeyParseError};

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::core::{
        ParameterType, HasValue, Validatable, Displayable,
        ParameterKind, ParameterError, ParameterValue, ParameterMetadata,
        ParameterDisplay, ParameterValidation, DisplayContext, UiMode,
    };

    pub use crate::types::{
        ButtonParameter, ButtonType,
        CheckboxParameter, CheckboxParameterOptions,
        CodeParameter, CodeParameterOptions, CodeLanguage, CodeTheme,
        ColorParameter, ColorParameterOptions, ColorFormat,
        DateParameter, DateParameterOptions,
        DateTimeParameter, DateTimeParameterOptions,
        ExpirableParameter, ExpirableParameterOptions, ExpirableValue,
        FileParameter, FileParameterOptions, FileReference,
        GroupParameter, GroupParameterOptions, GroupField, GroupFieldType, GroupValue, GroupLayout, GroupLabelPosition,
        HiddenParameter,
        ListParameter, ListParameterOptions, ListLayout,
        ModeParameter, ModeItem, ModeValue,
        MultiSelectParameter, MultiSelectParameterOptions,
        ObjectParameter, ObjectParameterOptions, ObjectValue, ObjectLayout, ObjectLabelPosition,
        NoticeParameter, NoticeParameterOptions, NoticeType,
        RadioParameter, RadioParameterOptions, RadioLayoutDirection,
        SecretParameter, SecretParameterOptions,
        SelectParameter, SelectParameterOptions,
        TextParameter, TextParameterOptions,
        TextareaParameter, TextareaParameterOptions,
        TimeParameter, TimeParameterOptions,
        RoutingParameter, RoutingParameterOptions, RoutingValue,
    };

    pub use nebula_core::prelude::{ParameterKey, KeyParseError};
}