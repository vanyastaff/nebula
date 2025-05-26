use std::any::Any;
use std::fmt::Debug;

use downcast_rs::{impl_downcast, Downcast};
use dyn_clone::{clone_trait_object, DynClone};

use crate::types::Key;
use crate::{
    ParameterDisplay, ParameterError, ParameterMetadata, ParameterValidation, ParameterValue,
};

pub trait Parameter: DynClone + Downcast + Any + Debug {
    fn metadata(&self) -> &ParameterMetadata;

    fn name(&self) -> &str {
        &self.metadata().name
    }

    fn key(&self) -> &Key {
        &self.metadata().key
    }

    fn get_value(&self) -> Option<&ParameterValue>;

    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError>;

    fn validation(&self) -> Option<&ParameterValidation> {
        None
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        None
    }
}

impl_downcast!(Parameter);
clone_trait_object!(Parameter);
