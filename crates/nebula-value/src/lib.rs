#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(missing_docs)]
#![warn(clippy::all)]

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

mod core;
mod types;
mod validation;
pub use core::*;
pub use types::*;
