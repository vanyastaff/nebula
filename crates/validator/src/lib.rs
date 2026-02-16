// ValidationError (152 bytes) is the fundamental error type for all validators —
// boxing it would add indirection to every validation call for no practical benefit.
#![allow(clippy::result_large_err)]
// Deep combinator nesting (And<Or<Not<...>, ...>, ...>) produces complex types
// that are inherent to the type-safe combinator architecture.
#![allow(clippy::type_complexity)]

pub mod combinators;
pub mod foundation;
pub mod prelude;
pub mod validators;
