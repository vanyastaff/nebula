use crate::request::RequestOptionsBuilder;
use crate::types::Key;
use crate::{ParameterType, ParameterValue};

pub trait CredentialContext: Send + Sync {
    fn get_parameter(&self, key: Key) -> &ParameterType;
    fn get_parameter_value(&self, key: Key) -> &ParameterValue;
    fn make_request(&self) -> RequestOptionsBuilder {
        RequestOptionsBuilder::default()
    }
}
