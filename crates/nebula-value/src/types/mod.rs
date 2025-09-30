// Primitive types
//!
//!
//!
//!
//!
//! ## Collection types
//! ## Numeric types
//! ## Primitive types
//! ## Temporal types
//! - [`Array`] - ordered sequences of values
//! - [`Boolean`] - boolean values
//! - [`Bytes`] - binary data
//! - [`DateTime`] - combined date and time
//! - [`Date`] - calendar dates
//! - [`Decimal`] - high-precision decimal numbers (optional)
//! - [`Duration`] - time spans
//! - [`Float`] - 64-bit floating point numbers
//! - [`Integer`] - 64-bit signed integers
//! - [`Number`] - unified numeric type
//! - [`Object`] - key-value mappings
//! - [`Text`] - string/text values
//! - [`Time`] - time of day
//! This module contains concrete implementations for all supported value types,
//! Value type implementations
//! organized into logical groups:
pub mod object_builder;
pub mod array_builder;
mod boolean;
mod text;
mod bytes;

// Numeric types
mod number;
mod decimal;

// Collection types
mod array;
mod object;

// Temporal types
mod date;
mod time;
mod datetime;
mod duration;
mod file;

// Re-export all types
pub use boolean::{Boolean, BooleanError};
pub use text::{Text, TextError};
pub use bytes::{Bytes, BytesError};
pub use number::{Float, Integer, Number, NumberError, NumberResult, JsonNumberStrategy};
pub use array::{Array, ArrayError};
pub use array_builder::ArrayBuilder;
pub use object::{Object, ObjectError};
pub use date::{Date, DateError};
pub use time::{Time, TimeError};
pub use datetime::{DateTime, DateTimeError};
pub use duration::{Duration, DurationError};
pub use file::{File, FileError, FileMetadata, StorageType};

pub use decimal::{Decimal, DecimalError};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}


