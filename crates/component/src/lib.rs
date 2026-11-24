//! The contract every integration component implements. See the crate README
//! below for the model.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod component;
/// The three kinds of component and the [`ComponentKind`] trait they share.
pub mod kind;
mod slot;

pub use component::{Component, KeyOf, admit, dependencies_of, key_of};
pub use kind::ComponentKind;
pub use slot::{
    Dependencies, DependencyKind, Slot, SlotError, SlotTarget, SlotValue, Target, is_valid_slot_key,
};
