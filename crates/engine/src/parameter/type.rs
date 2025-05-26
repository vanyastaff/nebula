use std::borrow::{Borrow, BorrowMut};

use serde::{Deserialize, Serialize};

use crate::parameter::types::*;
use crate::{
    Parameter, ParameterDisplay, ParameterError, ParameterMetadata, ParameterValidation,
    ParameterValue,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParameterType {
    Text(TextParameter),
    Textarea(TextareaParameter),
    Select(SelectParameter),
    MultiSelect(MultiSelectParameter),
    Expirable(ExpirableParameter),
    Radio(RadioParameter),
    Checkbox(CheckboxParameter),
    DateTime(DateTimeParameter),
    Date(DateParameter),
    Time(TimeParameter),
    File(FileParameter),
    Color(ColorParameter),
    Hidden(HiddenParameter),
    Notice(NoticeParameter),
    //Credential(CredentialParameter),
    //Code(CodeParameter),
    Secret(SecretParameter),
    Button(ButtonParameter),
    Mode(ModeParameter),
    Group(GroupParameter),
}

impl Parameter for ParameterType {
    fn metadata(&self) -> &ParameterMetadata {
        match self {
            Self::Text(p) => p.metadata(),
            Self::Textarea(p) => p.metadata(),
            Self::Select(p) => p.metadata(),
            Self::MultiSelect(p) => p.metadata(),
            Self::Expirable(p) => p.metadata(),
            Self::Radio(p) => p.metadata(),
            Self::Checkbox(p) => p.metadata(),
            Self::DateTime(p) => p.metadata(),
            Self::Date(p) => p.metadata(),
            Self::Time(p) => p.metadata(),
            Self::File(p) => p.metadata(),
            Self::Color(p) => p.metadata(),
            Self::Hidden(p) => p.metadata(),
            Self::Notice(p) => p.metadata(),
            //Self::Credential(p) => p.metadata(),
            //Self::Code(p) => p.metadata(),
            Self::Secret(p) => p.metadata(),
            Self::Button(p) => p.metadata(),
            Self::Mode(p) => p.metadata(),
            Self::Group(p) => p.metadata(),
        }
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        match self {
            Self::Text(p) => p.get_value(),
            Self::Textarea(p) => p.get_value(),
            Self::Select(p) => p.get_value(),
            Self::MultiSelect(p) => p.get_value(),
            Self::Expirable(p) => p.get_value(),
            Self::Radio(p) => p.get_value(),
            Self::Checkbox(p) => p.get_value(),
            Self::DateTime(p) => p.get_value(),
            Self::Date(p) => p.get_value(),
            Self::Time(p) => p.get_value(),
            Self::File(p) => p.get_value(),
            Self::Color(p) => p.get_value(),
            Self::Hidden(p) => p.get_value(),
            Self::Notice(p) => p.get_value(),
            //Self::Credential(p) => p.get_value(),
            //Self::Code(p) => p.get_value(),
            Self::Secret(p) => p.get_value(),
            Self::Button(p) => p.get_value(),
            Self::Mode(p) => p.get_value(),
            Self::Group(p) => p.get_value(),
        }
    }

    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        match self {
            Self::Text(p) => p.set_value(value),
            Self::Textarea(p) => p.set_value(value),
            Self::Select(p) => p.set_value(value),
            Self::MultiSelect(p) => p.set_value(value),
            Self::Expirable(p) => p.set_value(value),
            Self::Radio(p) => p.set_value(value),
            Self::Checkbox(p) => p.set_value(value),
            Self::DateTime(p) => p.set_value(value),
            Self::Date(p) => p.set_value(value),
            Self::Time(p) => p.set_value(value),
            Self::File(p) => p.set_value(value),
            Self::Color(p) => p.set_value(value),
            Self::Hidden(p) => p.set_value(value),
            Self::Notice(p) => p.set_value(value),
            Self::Secret(p) => p.set_value(value),
            Self::Button(p) => p.set_value(value),
            Self::Mode(p) => p.set_value(value),
            Self::Group(p) => p.set_value(value),
        }
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        match self {
            Self::Text(p) => p.validation(),
            Self::Textarea(p) => p.validation(),
            Self::Select(p) => p.validation(),
            Self::MultiSelect(p) => p.validation(),
            Self::Expirable(p) => p.validation(),
            Self::Radio(p) => p.validation(),
            Self::Checkbox(p) => p.validation(),
            Self::DateTime(p) => p.validation(),
            Self::Date(p) => p.validation(),
            Self::Time(p) => p.validation(),
            Self::File(p) => p.validation(),
            Self::Color(p) => p.validation(),
            Self::Hidden(p) => p.validation(),
            Self::Notice(p) => p.validation(),
            //Self::Credential(p) => p.validation(),
            //Self::Code(p) => p.validation(),
            Self::Secret(p) => p.validation(),
            Self::Button(p) => p.validation(),
            Self::Mode(p) => p.validation(),
            Self::Group(p) => p.validation(),
        }
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        match self {
            Self::Text(p) => p.display(),
            Self::Textarea(p) => p.display(),
            Self::Select(p) => p.display(),
            Self::MultiSelect(p) => p.display(),
            Self::Expirable(p) => p.display(),
            Self::Radio(p) => p.display(),
            Self::Checkbox(p) => p.display(),
            Self::DateTime(p) => p.display(),
            Self::Date(p) => p.display(),
            Self::Time(p) => p.display(),
            Self::File(p) => p.display(),
            Self::Color(p) => p.display(),
            Self::Hidden(p) => p.display(),
            Self::Notice(p) => p.display(),
            //Self::Credential(p) => p.display(),
            //Self::Code(p) => p.display(),
            Self::Secret(p) => p.display(),
            Self::Button(p) => p.display(),
            Self::Mode(p) => p.display(),
            Self::Group(p) => p.display(),
        }
    }
}

impl AsRef<dyn Parameter> for ParameterType {
    fn as_ref(&self) -> &dyn Parameter {
        match self {
            ParameterType::Text(p) => p,
            ParameterType::Textarea(p) => p,
            ParameterType::Select(p) => p,
            ParameterType::MultiSelect(p) => p,
            ParameterType::Expirable(p) => p,
            ParameterType::Radio(p) => p,
            ParameterType::Checkbox(p) => p,
            ParameterType::DateTime(p) => p,
            ParameterType::Date(p) => p,
            ParameterType::Time(p) => p,
            ParameterType::File(p) => p,
            ParameterType::Color(p) => p,
            ParameterType::Hidden(p) => p,
            ParameterType::Notice(p) => p,
            ParameterType::Secret(p) => p,
            ParameterType::Button(p) => p,
            ParameterType::Mode(p) => p,
            ParameterType::Group(p) => p,
        }
    }
}

impl AsMut<dyn Parameter> for ParameterType {
    fn as_mut(&mut self) -> &mut dyn Parameter {
        match self {
            ParameterType::Text(p) => p,
            ParameterType::Textarea(p) => p,
            ParameterType::Select(p) => p,
            ParameterType::MultiSelect(p) => p,
            ParameterType::Expirable(p) => p,
            ParameterType::Radio(p) => p,
            ParameterType::Checkbox(p) => p,
            ParameterType::DateTime(p) => p,
            ParameterType::Date(p) => p,
            ParameterType::Time(p) => p,
            ParameterType::File(p) => p,
            ParameterType::Color(p) => p,
            ParameterType::Hidden(p) => p,
            ParameterType::Notice(p) => p,
            ParameterType::Secret(p) => p,
            ParameterType::Button(p) => p,
            ParameterType::Mode(p) => p,
            ParameterType::Group(p) => p,
        }
    }
}

impl Borrow<dyn Parameter> for ParameterType {
    fn borrow(&self) -> &dyn Parameter {
        self.as_ref()
    }
}

impl BorrowMut<dyn Parameter> for ParameterType {
    fn borrow_mut(&mut self) -> &mut dyn Parameter {
        self.as_mut()
    }
}
