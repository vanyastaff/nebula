//! Macros for creating validators with minimal boilerplate.
//!
//! # Available Macros
//!
//! - [`validator!`] — Create a complete validator (struct + Validate impl + factory fn)
//! - [`compose!`] — AND-chain multiple validators
//! - [`any_of!`] — OR-chain multiple validators
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::validator;
//! use nebula_validator::foundation::{Validate, ValidationError};
//!
//! // Unit validator (no fields)
//! validator! {
//!     pub NotEmpty for str;
//!     rule(input) { !input.is_empty() }
//!     error(input) { ValidationError::new("not_empty", "must not be empty") }
//!     fn not_empty();
//! }
//!
//! // Struct with fields
//! validator! {
//!     #[derive(Copy, PartialEq, Eq, Hash)]
//!     pub MinLength { min: usize } for str;
//!     rule(self, input) { input.len() >= self.min }
//!     error(self, input) { ValidationError::min_length("", self.min, input.len()) }
//!     fn min_length(min: usize);
//! }
//! ```

// ============================================================================
// VALIDATOR MACRO — Unified Architecture
// ============================================================================
//
// The macro is organized in three layers:
//
// 1. ENTRY POINTS (5 arms) — parse user syntax, normalize into canonical form
//    - Unit:           `Name for Type;`
//    - Struct:         `Name { fields } for Type;`
//    - Bounded generic:`Name<T: Bounds> { fields } for Type;`
//    - Phantom unit:   `Name<T> for Type;`
//    - Phantom struct: `Name<T> { fields } for Type;`
//
// 2. TAIL PARSER (5 arms) — parse optional `new(…)` / `fn factory(…)` after rule+error
//    - (empty)                     → auto new, no factory
//    - `fn f(…);`                  → auto new + factory
//    - `new(…) {…}`               → custom new, no factory
//    - `new(…) {…} fn f(…);`      → custom new + factory
//    - `new(…)->E {…} fn f(…)->E;` → fallible new + factory
//
// 3. CODE GENERATORS (@helpers) — each responsible for one piece, zero duplication
//    - @struct_def        — struct definition with derives
//    - @auto_new_impl     — auto-generated constructor from fields
//    - @custom_new_impl   — user-provided constructor body
//    - @fallible_new_impl — user-provided fallible constructor
//    - @validate_impl     — Validate trait implementation
//    - @factory_fn        — convenience factory function
//    - @fallible_factory_fn — fallible factory function
//
// Key design decisions:
// - Meta attributes flow as opaque `tt` tokens (not `:meta`) through internal rules
// - Input types are wrapped in `[$input]` to satisfy `:ty` follow-set rules
// - The user's `self` identifier is threaded as `self_ref` for Rust 2024 macro hygiene
// - A `kind` marker (`[unit]`/`[fields]`) enables const factories for zero-sized types
//
// Adding a new feature (e.g. async validate, new generic pattern) means
// touching ONE helper, not N×M cross-product arms.
// ============================================================================

/// Creates a complete validator: struct definition, `Validate` implementation,
/// constructor, and factory function.
///
/// `#[derive(Debug, Clone)]` is always applied. Add extra derives via `#[derive(...)]`.
///
/// # Variants
///
/// **Unit validator** (zero-sized, no fields):
/// ```rust,ignore
/// validator! {
///     pub NotEmpty for str;
///     rule(input) { !input.is_empty() }
///     error(input) { ValidationError::new("not_empty", "empty") }
///     fn not_empty();
/// }
/// ```
///
/// **Struct with fields** (auto `new` from all fields):
/// ```rust,ignore
/// validator! {
///     #[derive(Copy, PartialEq, Eq, Hash)]
///     pub MinLength { min: usize } for str;
///     rule(self, input) { input.len() >= self.min }
///     error(self, input) { ValidationError::min_length("", self.min, input.len()) }
///     fn min_length(min: usize);
/// }
/// ```
///
/// **Custom constructor** (overrides auto `new`):
/// ```rust,ignore
/// validator! {
///     pub LengthRange { min: usize, max: usize } for str;
///     rule(self, input) { let l = input.len(); l >= self.min && l <= self.max }
///     error(self, input) { ValidationError::new("range", "out of range") }
///     new(min: usize, max: usize) { Self { min, max } }
///     fn length_range(min: usize, max: usize);
/// }
/// ```
///
/// **Fallible constructor** (returns Result):
/// ```rust,ignore
/// validator! {
///     pub Range { lo: usize, hi: usize } for usize;
///     rule(self, input) { *input >= self.lo && *input <= self.hi }
///     error(self, input) { ValidationError::new("range", "out of range") }
///     new(lo: usize, hi: usize) -> ValidationError {
///         if lo > hi { return Err(ValidationError::new("invalid", "lo > hi")); }
///         Ok(Self { lo, hi })
///     }
///     fn range(lo: usize, hi: usize) -> ValidationError;
/// }
/// ```
///
/// **Generic validator**:
/// ```rust,ignore
/// validator! {
///     #[derive(Copy, PartialEq, Eq, Hash)]
///     pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
///     rule(self, input) { *input >= self.min }
///     error(self, input) { ValidationError::new("min", format!("must be >= {}", self.min)) }
///     fn min(value: T);
/// }
/// ```
///
/// **Phantom generic** (generic with no trait bounds):
/// ```rust,ignore
/// validator! {
///     pub Required<T> for Option<T>;
///     rule(input) { input.is_some() }
///     error(input) { ValidationError::new("required", "required") }
///     fn required();
/// }
/// ```
#[macro_export]
macro_rules! validator {
    // ====================================================================
    // LAYER 1: ENTRY POINTS — parse header, delegate to @parse_tail
    // ====================================================================
    //
    // Meta attributes are passed as `[$(#[$meta])*]` — the #[…] wrappers
    // are included so they flow through internal rules as opaque `tt` tokens.
    //
    // `self_ref` carries the user's self identifier for Rust 2024 hygiene.
    // `kind` is `[unit]` for zero-sized types, `[fields]` for structs.

    // ── 1. Unit (no fields, no generics) ─────────────────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident for $input:ty;
        rule($inp:ident) $rule:block
        error($einp:ident) $err:block
        $($tail:tt)*
    ) => {
        $crate::validator! { @parse_tail
            meta:           [$(#[$meta])*]
            vis:            [$vis]
            name:           $name
            generics_decl:  []
            generics_use:   []
            fields:         []
            extra_fields:   []
            extra_init:     []
            extra_derives:  [Copy, PartialEq, Eq, Hash]
            kind:           [unit]
            self_ref:       [self]
            input:          [$input]
            inp:            $inp
            rule:           $rule
            einp:           $einp
            err:            $err
            tail:           [$($tail)*]
        }
    };

    // ── 2. Struct (fields, no generics) ──────────────────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        $($tail:tt)*
    ) => {
        $crate::validator! { @parse_tail
            meta:           [$(#[$meta])*]
            vis:            [$vis]
            name:           $name
            generics_decl:  []
            generics_use:   []
            fields:         [$(pub $field: $fty,)+]
            extra_fields:   []
            extra_init:     []
            extra_derives:  []
            kind:           [fields]
            self_ref:       [$self_]
            input:          [$input]
            inp:            $inp
            rule:           $rule
            einp:           $einp
            err:            $err
            tail:           [$($tail)*]
        }
    };

    // ── 3. Bounded generic struct (T: Bounds) ────────────────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident<$gen:ident: $first:ident $(+ $rest:ident)*>
            { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        $($tail:tt)*
    ) => {
        $crate::validator! { @parse_tail
            meta:           [$(#[$meta])*]
            vis:            [$vis]
            name:           $name
            generics_decl:  [<$gen: $first $(+ $rest)*>]
            generics_use:   [<$gen>]
            fields:         [$(pub $field: $fty,)+]
            extra_fields:   []
            extra_init:     []
            extra_derives:  []
            kind:           [fields]
            self_ref:       [$self_]
            input:          [$input]
            inp:            $inp
            rule:           $rule
            einp:           $einp
            err:            $err
            tail:           [$($tail)*]
        }
    };

    // ── 4. Phantom unit (generic T, no bounds, no fields) ────────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident<$gen:ident> for $input:ty;
        rule($inp:ident) $rule:block
        error($einp:ident) $err:block
        $($tail:tt)*
    ) => {
        $crate::validator! { @parse_tail
            meta:           [$(#[$meta])*]
            vis:            [$vis]
            name:           $name
            generics_decl:  [<$gen>]
            generics_use:   [<$gen>]
            fields:         []
            extra_fields:   [_phantom: ::std::marker::PhantomData<$gen>,]
            extra_init:     [_phantom: ::std::marker::PhantomData,]
            extra_derives:  [Copy, PartialEq, Eq, Hash]
            kind:           [unit]
            self_ref:       [self]
            input:          [$input]
            inp:            $inp
            rule:           $rule
            einp:           $einp
            err:            $err
            tail:           [$($tail)*]
        }
    };

    // ── 5. Phantom struct (generic T, no bounds, with fields) ────────────
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident<$gen:ident>
            { $($field:ident: $fty:ty),+ $(,)? } for $input:ty;
        rule($self_:ident, $inp:ident) $rule:block
        error($self2:ident, $einp:ident) $err:block
        $($tail:tt)*
    ) => {
        $crate::validator! { @parse_tail
            meta:           [$(#[$meta])*]
            vis:            [$vis]
            name:           $name
            generics_decl:  [<$gen>]
            generics_use:   [<$gen>]
            fields:         [$(pub $field: $fty,)+]
            extra_fields:   [_phantom: ::std::marker::PhantomData<$gen>,]
            extra_init:     [_phantom: ::std::marker::PhantomData,]
            extra_derives:  []
            kind:           [fields]
            self_ref:       [$self_]
            input:          [$input]
            inp:            $inp
            rule:           $rule
            einp:           $einp
            err:            $err
            tail:           [$($tail)*]
        }
    };

    // ====================================================================
    // LAYER 2: TAIL PARSER — detect new/factory, emit code via helpers
    // ====================================================================
    //
    // `meta` is matched as `[$($meta:tt)*]` so #[…] attributes flow
    // through as opaque token trees without re-parsing as `:meta`.

    // ── Tail 1: (empty) → auto new, no factory ──────────────────────────
    (@parse_tail
        meta: [$($meta:tt)*] vis: [$vis:vis] name: $name:ident
        generics_decl: [$($gd:tt)*] generics_use: [$($gu:tt)*]
        fields: [$($fields:tt)*] extra_fields: [$($ef:tt)*]
        extra_init: [$($ei:tt)*] extra_derives: [$($ed:ident),*]
        kind: [$kind:tt] self_ref: [$self_ref:ident]
        input: [$input:ty] inp: $inp:ident rule: $rule:block
        einp: $einp:ident err: $err:block
        tail: []
    ) => {
        $crate::validator!(@struct_def
            [$($meta)*] $vis $name [$($gd)*] [$($gu)*]
            [$($fields)* $($ef)*] [$($ed),*]
        );
        $crate::validator!(@auto_new_impl
            $vis $name [$($gd)*] [$($gu)*]
            [$($fields)*] [$($ei)*]
        );
        $crate::validator!(@validate_impl
            $name [$($gd)*] [$($gu)*] [$input] $self_ref $inp $rule $einp $err
        );
    };

    // ── Tail 2: factory only → auto new + factory ────────────────────────
    (@parse_tail
        meta: [$($meta:tt)*] vis: [$vis:vis] name: $name:ident
        generics_decl: [$($gd:tt)*] generics_use: [$($gu:tt)*]
        fields: [$($fields:tt)*] extra_fields: [$($ef:tt)*]
        extra_init: [$($ei:tt)*] extra_derives: [$($ed:ident),*]
        kind: [$kind:tt] self_ref: [$self_ref:ident]
        input: [$input:ty] inp: $inp:ident rule: $rule:block
        einp: $einp:ident err: $err:block
        tail: [fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);]
    ) => {
        $crate::validator!(@struct_def
            [$($meta)*] $vis $name [$($gd)*] [$($gu)*]
            [$($fields)* $($ef)*] [$($ed),*]
        );
        $crate::validator!(@auto_new_impl
            $vis $name [$($gd)*] [$($gu)*]
            [$($fields)*] [$($ei)*]
        );
        $crate::validator!(@validate_impl
            $name [$($gd)*] [$($gu)*] [$input] $self_ref $inp $rule $einp $err
        );
        $crate::validator!(@factory_fn
            [$kind] $vis $name [$($gd)*] [$($gu)*]
            $factory [$($farg: $faty),*] [$($farg),*]
        );
    };

    // ── Tail 3: custom new, no factory ───────────────────────────────────
    (@parse_tail
        meta: [$($meta:tt)*] vis: [$vis:vis] name: $name:ident
        generics_decl: [$($gd:tt)*] generics_use: [$($gu:tt)*]
        fields: [$($fields:tt)*] extra_fields: [$($ef:tt)*]
        extra_init: [$($ei:tt)*] extra_derives: [$($ed:ident),*]
        kind: [$kind:tt] self_ref: [$self_ref:ident]
        input: [$input:ty] inp: $inp:ident rule: $rule:block
        einp: $einp:ident err: $err:block
        tail: [new($($narg:ident: $naty:ty),* $(,)?) $new_body:block]
    ) => {
        $crate::validator!(@struct_def
            [$($meta)*] $vis $name [$($gd)*] [$($gu)*]
            [$($fields)* $($ef)*] [$($ed),*]
        );
        $crate::validator!(@custom_new_impl
            $vis $name [$($gd)*] [$($gu)*]
            [$($narg: $naty),*] $new_body
        );
        $crate::validator!(@validate_impl
            $name [$($gd)*] [$($gu)*] [$input] $self_ref $inp $rule $einp $err
        );
    };

    // ── Tail 4: custom new + factory ─────────────────────────────────────
    (@parse_tail
        meta: [$($meta:tt)*] vis: [$vis:vis] name: $name:ident
        generics_decl: [$($gd:tt)*] generics_use: [$($gu:tt)*]
        fields: [$($fields:tt)*] extra_fields: [$($ef:tt)*]
        extra_init: [$($ei:tt)*] extra_derives: [$($ed:ident),*]
        kind: [$kind:tt] self_ref: [$self_ref:ident]
        input: [$input:ty] inp: $inp:ident rule: $rule:block
        einp: $einp:ident err: $err:block
        tail: [
            new($($narg:ident: $naty:ty),* $(,)?) $new_body:block
            fn $factory:ident($($farg:ident: $faty:ty),* $(,)?);
        ]
    ) => {
        $crate::validator!(@struct_def
            [$($meta)*] $vis $name [$($gd)*] [$($gu)*]
            [$($fields)* $($ef)*] [$($ed),*]
        );
        $crate::validator!(@custom_new_impl
            $vis $name [$($gd)*] [$($gu)*]
            [$($narg: $naty),*] $new_body
        );
        $crate::validator!(@validate_impl
            $name [$($gd)*] [$($gu)*] [$input] $self_ref $inp $rule $einp $err
        );
        $crate::validator!(@factory_fn
            [$kind] $vis $name [$($gd)*] [$($gu)*]
            $factory [$($farg: $faty),*] [$($farg),*]
        );
    };

    // ── Tail 5: fallible new + fallible factory ──────────────────────────
    (@parse_tail
        meta: [$($meta:tt)*] vis: [$vis:vis] name: $name:ident
        generics_decl: [$($gd:tt)*] generics_use: [$($gu:tt)*]
        fields: [$($fields:tt)*] extra_fields: [$($ef:tt)*]
        extra_init: [$($ei:tt)*] extra_derives: [$($ed:ident),*]
        kind: [$kind:tt] self_ref: [$self_ref:ident]
        input: [$input:ty] inp: $inp:ident rule: $rule:block
        einp: $einp:ident err: $err:block
        tail: [
            new($($narg:ident: $naty:ty),* $(,)?) -> $ety:ty $new_body:block
            fn $factory:ident($($farg:ident: $faty:ty),* $(,)?) -> $efty:ty;
        ]
    ) => {
        $crate::validator!(@struct_def
            [$($meta)*] $vis $name [$($gd)*] [$($gu)*]
            [$($fields)* $($ef)*] [$($ed),*]
        );
        $crate::validator!(@fallible_new_impl
            $vis $name [$($gd)*] [$($gu)*]
            [$($narg: $naty),*] $ety $new_body
        );
        $crate::validator!(@validate_impl
            $name [$($gd)*] [$($gu)*] [$input] $self_ref $inp $rule $einp $err
        );
        $crate::validator!(@fallible_factory_fn
            $vis $name [$($gd)*] [$($gu)*]
            $factory [$($farg: $faty),*] [$($farg),*] $efty
        );
    };

    // ====================================================================
    // LAYER 3: CODE GENERATORS — each handles exactly one concern
    // ====================================================================

    // ── @struct_def: unit struct (no fields at all) ──────────────────────
    (@struct_def
        [$($meta:tt)*] $vis:vis $name:ident
        [$($gd:tt)*] [$($gu:tt)*]
        [] [$($ed:ident),*]
    ) => {
        $($meta)*
        #[derive(Debug, Clone, $($ed,)*)]
        $vis struct $name $($gd)*;
    };

    // ── @struct_def: struct with fields ──────────────────────────────────
    (@struct_def
        [$($meta:tt)*] $vis:vis $name:ident
        [$($gd:tt)*] [$($gu:tt)*]
        [$($all_fields:tt)+] [$($ed:ident),*]
    ) => {
        $($meta)*
        #[derive(Debug, Clone, $($ed,)*)]
        $vis struct $name $($gu)* {
            $($all_fields)+
        }
    };

    // ── @auto_new_impl: true unit (no fields, no phantom) → skip ────────
    (@auto_new_impl
        $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        [] []
    ) => {
        // True unit structs don't need a constructor.
    };

    // ── @auto_new_impl: phantom unit (no user fields, has phantom) ──────
    (@auto_new_impl
        $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        [] [$($ei:tt)+]
    ) => {
        impl $($gd)* $name $($gu)* {
            #[must_use] #[inline]
            pub fn new() -> Self {
                Self { $($ei)+ }
            }
        }
    };

    // ── @auto_new_impl: has user fields → generate new(fields) ──────────
    (@auto_new_impl
        $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        [$(pub $field:ident: $fty:ty,)+] [$($ei:tt)*]
    ) => {
        impl $($gd)* $name $($gu)* {
            #[must_use] #[inline]
            pub fn new($($field: $fty),+) -> Self {
                Self { $($field,)+ $($ei)* }
            }
        }
    };

    // ── @custom_new_impl ─────────────────────────────────────────────────
    (@custom_new_impl
        $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        [$($args:tt)*] $body:block
    ) => {
        #[allow(clippy::new_without_default)]
        impl $($gd)* $name $($gu)* {
            #[must_use] #[inline]
            pub fn new($($args)*) -> Self $body
        }
    };

    // ── @fallible_new_impl ───────────────────────────────────────────────
    (@fallible_new_impl
        $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        [$($args:tt)*] $ety:ty $body:block
    ) => {
        impl $($gd)* $name $($gu)* {
            #[inline]
            pub fn new($($args)*) -> ::std::result::Result<Self, $ety> $body
        }
    };

    // ── @validate_impl ──────────────────────────────────────────────────
    //
    // Uses the user's `$self_ref` identifier as the method self parameter
    // so that Rust 2024 macro hygiene allows the rule/error bodies to
    // reference `self` fields.
    (@validate_impl
        $name:ident [$($gd:tt)*] [$($gu:tt)*]
        [$input:ty] $self_ref:ident $inp:ident $rule:block $einp:ident $err:block
    ) => {
        impl $($gd)* $crate::foundation::Validate for $name $($gu)* {
            type Input = $input;

            #[inline]
            #[allow(unused_variables)]
            fn validate(&$self_ref, $inp: &Self::Input)
                -> ::std::result::Result<(), $crate::foundation::ValidationError>
            {
                if $rule {
                    Ok(())
                } else {
                    let $einp = $inp;
                    Err($err)
                }
            }
        }
    };

    // ── @factory_fn: unit, no generics, no args → const, direct value ───
    (@factory_fn
        [unit] $vis:vis $name:ident [] []
        $factory:ident [] []
    ) => {
        #[must_use] #[inline]
        $vis const fn $factory() -> $name { $name }
    };

    // ── @factory_fn: generic no-args → delegates to new() ────────────────
    (@factory_fn
        [$_kind:tt] $vis:vis $name:ident [$($gd:tt)+] [$($gu:tt)+]
        $factory:ident [] []
    ) => {
        #[must_use] #[inline]
        $vis fn $factory $($gd)+() -> $name $($gu)+ {
            $name::new()
        }
    };

    // ── @factory_fn: non-generic no-args (fields) → delegates to new() ──
    (@factory_fn
        [fields] $vis:vis $name:ident [] []
        $factory:ident [] []
    ) => {
        #[must_use] #[inline]
        $vis fn $factory() -> $name {
            $name::new()
        }
    };

    // ── @factory_fn: with args → delegates to new() ──────────────────────
    (@factory_fn
        [$_kind:tt] $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        $factory:ident [$($args:tt)+] [$($passthrough:tt)+]
    ) => {
        #[must_use] #[inline]
        $vis fn $factory $($gd)*($($args)+) -> $name $($gu)* {
            $name::new($($passthrough)+)
        }
    };

    // ── @fallible_factory_fn ─────────────────────────────────────────────
    (@fallible_factory_fn
        $vis:vis $name:ident [$($gd:tt)*] [$($gu:tt)*]
        $factory:ident [$($args:tt)*] [$($passthrough:tt)*] $efty:ty
    ) => {
        #[inline]
        $vis fn $factory $($gd)*($($args)*) -> ::std::result::Result<$name $($gu)*, $efty> {
            $name::new($($passthrough)*)
        }
    };
}

// ============================================================================
// COMPOSE MACRO
// ============================================================================

/// Composes multiple validators using AND logic.
///
/// ```rust,ignore
/// let validator = compose![min_length(5), max_length(20), alphanumeric()];
/// ```
#[macro_export]
macro_rules! compose {
    ($first:expr) => { $first };
    ($first:expr, $($rest:expr),+ $(,)?) => {
        $first$(.and($rest))+
    };
}

// ============================================================================
// ANY_OF MACRO
// ============================================================================

/// Composes multiple validators using OR logic.
///
/// ```rust,ignore
/// let validator = any_of![exact_length(5), exact_length(10)];
/// ```
#[macro_export]
macro_rules! any_of {
    ($first:expr) => { $first };
    ($first:expr, $($rest:expr),+ $(,)?) => {
        $first$(.or($rest))+
    };
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use crate::foundation::{Validate, ValidationError};

    // Test 1: Unit validator (no fields)
    validator! {
        /// A test unit validator.
        TestNotEmpty for str;
        rule(input) { !input.is_empty() }
        error(input) { ValidationError::new("not_empty", "must not be empty") }
        fn test_not_empty();
    }

    #[test]
    fn test_unit_validator() {
        let v = TestNotEmpty;
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("").is_err());
    }

    #[test]
    fn test_unit_factory() {
        let v = test_not_empty();
        assert!(v.validate("x").is_ok());
    }

    // Test 2: Struct with fields + auto new
    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestMinLen { min: usize } for str;
        rule(self, input) { input.len() >= self.min }
        error(self, input) {
            ValidationError::new("min_len", format!("need {} chars", self.min))
        }
        fn test_min_len(min: usize);
    }

    #[test]
    fn test_struct_validator() {
        let v = TestMinLen { min: 3 };
        assert!(v.validate("abc").is_ok());
        assert!(v.validate("ab").is_err());
    }

    #[test]
    fn test_struct_new() {
        let v = TestMinLen::new(5);
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("hi").is_err());
    }

    #[test]
    fn test_struct_factory() {
        let v = test_min_len(5);
        assert!(v.validate("hello").is_ok());
        assert!(v.validate("hi").is_err());
    }

    // Test 3: Generic validator
    use std::fmt::Display;

    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestMin<T: PartialOrd + Display + Copy> { min: T } for T;
        rule(self, input) { *input >= self.min }
        error(self, input) {
            ValidationError::new("min", format!("must be >= {}", self.min))
        }
        fn test_min_val(value: T);
    }

    #[test]
    fn test_generic_validator() {
        let v = test_min_val(5_i32);
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&4).is_err());
    }

    #[test]
    fn test_generic_validator_f64() {
        let v = TestMin::new(1.5_f64);
        assert!(v.validate(&2.0).is_ok());
        assert!(v.validate(&1.0).is_err());
    }

    // Test 4: Custom constructor
    validator! {
        #[derive(Copy, PartialEq, Eq, Hash)]
        TestRange { lo: usize, hi: usize } for usize;
        rule(self, input) { *input >= self.lo && *input <= self.hi }
        error(self, input) {
            ValidationError::new("range", format!("{} not in {}..{}", input, self.lo, self.hi))
        }
        new(lo: usize, hi: usize) { Self { lo, hi } }
        fn test_range(lo: usize, hi: usize);
    }

    #[test]
    fn test_custom_new() {
        let v = test_range(1, 10);
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&0).is_err());
        assert!(v.validate(&11).is_err());
    }

    // Test 5: Unit validator without factory fn
    validator! {
        TestAlwaysOk for str;
        rule(input) { true }
        error(input) { ValidationError::new("unreachable", "unreachable") }
    }

    #[test]
    fn test_unit_without_factory() {
        let v = TestAlwaysOk;
        assert!(v.validate("anything").is_ok());
    }

    // Test 6: Struct without factory fn
    validator! {
        TestMax { max: usize } for usize;
        rule(self, input) { *input <= self.max }
        error(self, input) {
            ValidationError::new("max", format!("must be <= {}", self.max))
        }
    }

    #[test]
    fn test_struct_without_factory() {
        let v = TestMax::new(10);
        assert!(v.validate(&10).is_ok());
        assert!(v.validate(&11).is_err());
    }

    // Test 7: compose! and any_of! still work
    #[test]
    fn test_compose_still_works() {
        use crate::foundation::ValidateExt;
        let v = compose![TestMinLen { min: 3 }, TestMinLen { min: 1 }];
        assert!(v.validate("abc").is_ok());
        assert!(v.validate("ab").is_err());
    }

    #[test]
    fn test_any_of_still_works() {
        use crate::foundation::ValidateExt;
        let v = any_of![TestMinLen { min: 100 }, TestMinLen { min: 1 }];
        assert!(v.validate("x").is_ok());
    }

    // Test 8: Error messages are correct
    #[test]
    fn test_error_message_content() {
        let v = TestMinLen { min: 5 };
        let err = v.validate("hi").unwrap_err();
        assert_eq!(err.code, "min_len");
        assert_eq!(err.message, "need 5 chars");
    }

    #[test]
    fn test_unit_error_message_content() {
        let v = TestNotEmpty;
        let err = v.validate("").unwrap_err();
        assert_eq!(err.code, "not_empty");
        assert_eq!(err.message, "must not be empty");
    }

    // Test 9: Custom new body is respected
    #[test]
    fn test_custom_new_body() {
        let v = TestRange::new(3, 7);
        assert_eq!(v.lo, 3);
        assert_eq!(v.hi, 7);
    }

    // Test 10: Phantom unit validator (generic, no fields, no bounds)
    validator! {
        TestPhantomUnit<T> for Option<T>;
        rule(input) { input.is_some() }
        error(input) { ValidationError::new("required", "required") }
        fn test_phantom_unit();
    }

    #[test]
    fn test_phantom_unit_validator() {
        let v = test_phantom_unit::<i32>();
        assert!(v.validate(&Some(42)).is_ok());
        assert!(v.validate(&None::<i32>).is_err());
    }

    #[test]
    fn test_phantom_unit_copy() {
        let v = test_phantom_unit::<i32>();
        let v2 = v; // Copy works when T: Copy
        assert!(v.validate(&Some(1)).is_ok());
        assert!(v2.validate(&None::<i32>).is_err());
    }

    // Test 11: Phantom struct validator (generic, fields, no bounds)
    validator! {
        TestPhantomStruct<T> { min: usize } for [T];
        rule(self, input) { input.len() >= self.min }
        error(self, input) {
            ValidationError::new("min", format!("need {} elements", self.min))
        }
        fn test_phantom_struct(min: usize);
    }

    #[test]
    fn test_phantom_struct_validator() {
        let v = test_phantom_struct::<i32>(2);
        assert!(v.validate(&[1, 2, 3]).is_ok());
        assert!(v.validate(&[1]).is_err());
    }

    #[test]
    fn test_phantom_struct_new() {
        let v = TestPhantomStruct::<String>::new(1);
        assert!(v.validate(&["a".to_string()]).is_ok());
        assert!(v.validate(&[]).is_err());
    }

    #[test]
    fn test_phantom_struct_error_message() {
        let v = test_phantom_struct::<i32>(3);
        let err = v.validate(&[1]).unwrap_err();
        assert_eq!(err.code, "min");
        assert_eq!(err.message, "need 3 elements");
    }

    // Test 12: Fallible constructor (returns Result)
    validator! {
        TestFallible { lo: usize, hi: usize } for usize;
        rule(self, input) { *input >= self.lo && *input <= self.hi }
        error(self, input) {
            ValidationError::new("range", format!("{} not in {}..{}", input, self.lo, self.hi))
        }
        new(lo: usize, hi: usize) -> ValidationError {
            if lo > hi {
                return Err(ValidationError::new("invalid", "lo must be <= hi"));
            }
            Ok(Self { lo, hi })
        }
        fn test_fallible(lo: usize, hi: usize) -> ValidationError;
    }

    #[test]
    fn test_fallible_valid_construction() {
        let v = test_fallible(1, 10).unwrap();
        assert!(v.validate(&5).is_ok());
        assert!(v.validate(&0).is_err());
        assert!(v.validate(&11).is_err());
    }

    #[test]
    fn test_fallible_invalid_construction() {
        assert!(test_fallible(10, 5).is_err());
        assert!(TestFallible::new(10, 5).is_err());
    }

    #[test]
    fn test_fallible_error_content() {
        let err = TestFallible::new(10, 5).unwrap_err();
        assert_eq!(err.code, "invalid");
    }
}
