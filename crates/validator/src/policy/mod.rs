//! Field visibility / required policy evaluation (ADR-0052).
//!
//! Owns the *engine* for `When(Rule)` conditions. Callers get typed
//! `Presence`/`Requiredness` verdicts — never a raw `bool` they could
//! forget to branch on.
//!
//! Imports are added by later tasks as each type is first consumed
//! (Task 3 adds `crate::rule::{PredicateContext, Rule}`; Task 4 adds
//! `crate::foundation::{FieldPath, ValidationError, ValidationErrors}`).

/// Whether a field participates in this validation round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Presence {
    /// Field is visible; its value rules must run.
    Active,
    /// Field is hidden; its value rules MUST be skipped.
    Skipped,
}

/// Resolved required-ness for a field in this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Requiredness {
    /// Absence is an error.
    Required,
    /// Absence is allowed.
    Optional,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_variants_are_copy_and_eq() {
        let p = Presence::Active;
        let q = p; // Copy
        assert_eq!(p, q);
        assert_ne!(Presence::Active, Presence::Skipped);
    }

    #[test]
    fn requiredness_variants_are_copy_and_eq() {
        let r = Requiredness::Required;
        let s = r;
        assert_eq!(r, s);
        assert_ne!(Requiredness::Required, Requiredness::Optional);
    }
}
