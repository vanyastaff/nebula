//! Value type implementations
//!
//! This module contains concrete implementations for all supported value types,
//! organized into logical groups:
//!
//! ## Primitive types
//! - [`Boolean`] - boolean values
//! - [`Text`] - string/text values
//! - [`Bytes`] - binary data
//!
//! ## Numeric types
//! - [`Integer`] - 64-bit signed integers
//! - [`Float`] - 64-bit floating point numbers
//! - [`Number`] - unified numeric type
//! - [`Decimal`] - high-precision decimal numbers (optional)
//!
//! ## Collection types
//! - [`Array`] - ordered sequences of values
//! - [`Object`] - key-value mappings
//!
//! ## Temporal types
//! - [`Date`] - calendar dates
//! - [`Time`] - time of day
//! - [`DateTime`] - combined date and time
//! - [`Duration`] - time spans

// Primitive types
mod boolean;
mod text;
mod bytes;

// Numeric types
mod number;

// Collection types
mod array;
mod object;

// Temporal types
mod date;
mod time;
mod datetime;
mod duration;

// Re-export all types
pub use boolean::{Boolean, BooleanError};
pub use text::{Text, TextError};
pub use bytes::{Bytes, BytesError};
pub use number::{Float, Integer, Number, NumberError};
pub use array::{Array, ArrayError};
pub use object::{Object, ObjectError};
pub use date::{Date, DateError};
pub use time::{Time, TimeError};
pub use datetime::{DateTime, DateTimeError};
pub use duration::{Duration, DurationError};

#[cfg(feature = "decimal")]
pub use decimal::Decimal;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
