//! Assertion macros for testing action results.
//!
//! These macros simplify asserting on [`ActionResult`] and
//! [`ActionError`] variants in tests. They are exported via
//! `#[macro_export]` so they are available at the crate root.
//!
//! **Note:** These macros require `Debug` on the result type for error
//! formatting in panic messages. This is standard for test assertions.

/// Assert that the result is `Ok(ActionResult::Success { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Ok(ActionResult::Success { .. })`.
///
/// # Examples
///
/// ```rust
/// use nebula_action::{ActionError, ActionResult, assert_success};
///
/// // Stand in for `let result = action.execute(input, &ctx).await;`
/// let result: Result<ActionResult<i32>, ActionError> = Ok(ActionResult::success(42));
/// assert_success!(result);
///
/// // The two-argument form additionally asserts on the unwrapped output value.
/// assert_success!(result, 42);
/// ```
#[macro_export]
macro_rules! assert_success {
    ($result:expr) => {
        match &$result {
            Ok($crate::ActionResult::Success { .. }) => {},
            other => panic!("expected ActionResult::Success, got {:?}", other),
        }
    };
    ($result:expr, $expected:expr) => {
        match &$result {
            Ok($crate::ActionResult::Success { output, .. }) => {
                let val = output
                    .as_value()
                    .expect("expected ActionOutput::Value in assert_success!");
                assert_eq!(val, &$expected, "ActionResult::Success output mismatch");
            },
            other => panic!("expected ActionResult::Success, got {:?}", other),
        }
    };
}

/// Assert that the result is `Ok(ActionResult::Branch { selected, .. })` with
/// the given branch key.
///
/// # Panics
///
/// Panics if the result is not a `Branch` or the selected key does not match.
#[macro_export]
macro_rules! assert_branch {
    ($result:expr, $key:expr) => {
        match &$result {
            Ok($crate::ActionResult::Branch { selected, .. }) => {
                assert_eq!(
                    selected.as_str(),
                    $key,
                    "expected branch key '{}', got '{}'",
                    $key,
                    selected
                );
            },
            other => panic!("expected ActionResult::Branch, got {:?}", other),
        }
    };
}

/// Assert that the result is `Ok(ActionResult::Continue { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Ok(ActionResult::Continue { .. })`.
#[macro_export]
macro_rules! assert_continue {
    ($result:expr) => {
        match &$result {
            Ok($crate::ActionResult::Continue { .. }) => {},
            other => panic!("expected ActionResult::Continue, got {:?}", other),
        }
    };
}

/// Assert that the result is `Ok(ActionResult::Break { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Ok(ActionResult::Break { .. })`.
#[macro_export]
macro_rules! assert_break {
    ($result:expr) => {
        match &$result {
            Ok($crate::ActionResult::Break { .. }) => {},
            other => panic!("expected ActionResult::Break, got {:?}", other),
        }
    };
}

/// Assert that the result is `Ok(ActionResult::Skip { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Ok(ActionResult::Skip { .. })`.
#[macro_export]
macro_rules! assert_skip {
    ($result:expr) => {
        match &$result {
            Ok($crate::ActionResult::Skip { .. }) => {},
            other => panic!("expected ActionResult::Skip, got {:?}", other),
        }
    };
}

/// Assert that the result is `Ok(ActionResult::Wait { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Ok(ActionResult::Wait { .. })`.
#[macro_export]
macro_rules! assert_wait {
    ($result:expr) => {
        match &$result {
            Ok($crate::ActionResult::Wait { .. }) => {},
            other => panic!("expected ActionResult::Wait, got {:?}", other),
        }
    };
}

/// Assert that the result is `Err(ActionError::Retryable { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Err(ActionError::Retryable { .. })`.
#[macro_export]
macro_rules! assert_retryable {
    ($result:expr) => {
        match &$result {
            Err($crate::ActionError::Retryable { .. }) => {},
            other => panic!("expected ActionError::Retryable, got {:?}", other),
        }
    };
}

/// Assert that the result is `Err(ActionError::Fatal { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Err(ActionError::Fatal { .. })`.
#[macro_export]
macro_rules! assert_fatal {
    ($result:expr) => {
        match &$result {
            Err($crate::ActionError::Fatal { .. }) => {},
            other => panic!("expected ActionError::Fatal, got {:?}", other),
        }
    };
}

/// Assert that the result is `Err(ActionError::Validation { .. })`.
///
/// # Panics
///
/// Panics if the result is not `Err(ActionError::Validation { .. })`.
#[macro_export]
macro_rules! assert_validation_error {
    ($result:expr) => {
        match &$result {
            Err($crate::ActionError::Validation { .. }) => {},
            other => panic!("expected ActionError::Validation, got {:?}", other),
        }
    };
}

/// Assert that the result is `Err(ActionError::Cancelled)`.
///
/// # Panics
///
/// Panics if the result is not `Err(ActionError::Cancelled)`.
#[macro_export]
macro_rules! assert_cancelled {
    ($result:expr) => {
        match &$result {
            Err($crate::ActionError::Cancelled) => {},
            other => panic!("expected ActionError::Cancelled, got {:?}", other),
        }
    };
}

/// Declare another catalog entry that runs an existing stateless action.
///
/// An action's key and metadata belong to its type, so one type is one catalog
/// entry. To register the same behavior under a second key, or with another
/// name or version, declare a new type for that entry. This macro writes that
/// type: a newtype over `$inner` with its own `KEY` and metadata, whose
/// `execute` runs `$inner`'s. `Config`, `Output` and dependencies are always
/// `$inner`'s. The entry's interface (ports, isolation, effect contract and
/// concurrency hint) is `$inner`'s too, unless the entry declares its own with
/// these optional trailing arguments, in this order:
///
/// - `inputs: Vec<InputPort>`
/// - `outputs: Vec<OutputPort>`
/// - `isolation_level: IsolationLevel`
/// - `effect_contract: ActionEffectContract`
/// - `max_concurrent: Option<NonZeroU32>`
///
/// The metadata and the port lists are expressions evaluated in the generated
/// functions, so they cannot refer to local variables. The `isolation_level`,
/// `effect_contract` and `max_concurrent` values must be constant expressions.
///
/// # Examples
///
/// ```rust
/// use std::sync::OnceLock;
/// use nebula_action::{
///     Action, ActionContext, ActionError, ActionResult, Component, Dependencies, Metadata,
///     StatelessAction, kind, metadata_name, stateless_entry,
/// };
///
/// struct Echo;
///
/// impl Component for Echo {
///     type Kind = kind::Action;
///     type Config = serde_json::Value;
///
///     const KEY: &'static str = "demo.echo";
///
///     fn metadata() -> Metadata {
///         Metadata::new(metadata_name!("Echo"), "Echoes its input")
///     }
/// }
///
/// impl Action for Echo {
///     type Output = serde_json::Value;
///
///     fn dependencies() -> &'static Dependencies {
///         static D: OnceLock<Dependencies> = OnceLock::new();
///         D.get_or_init(Dependencies::new)
///     }
/// }
///
/// impl StatelessAction for Echo {
///     async fn execute(
///         &self,
///         input: serde_json::Value,
///         _ctx: &(impl ActionContext + ?Sized),
///     ) -> Result<ActionResult<serde_json::Value>, ActionError> {
///         Ok(ActionResult::success(input))
///     }
/// }
///
/// stateless_entry!(Mirror(Echo) = "demo.mirror", Metadata::new(metadata_name!("Mirror"), ""));
///
/// // The same behavior, declared read-only under its own key.
/// stateless_entry!(
///     ReadOnlyMirror(Echo) = "demo.mirror_read_only",
///     Metadata::new(metadata_name!("Read-only mirror"), ""),
///     effect_contract: nebula_action::ActionEffectContract::ReadOnly,
/// );
///
/// assert_eq!(nebula_action::key_of::<Mirror>().as_str(), "demo.mirror");
/// assert_eq!(
///     <ReadOnlyMirror as Action>::EFFECT_CONTRACT,
///     nebula_action::ActionEffectContract::ReadOnly
/// );
/// let _entry = Mirror(Echo);
/// ```
#[macro_export]
macro_rules! stateless_entry {
    (
        $(#[$attr:meta])*
        $vis:vis $name:ident($inner:ty) = $key:literal, $metadata:expr
        $(, inputs: $inputs:expr)?
        $(, outputs: $outputs:expr)?
        $(, isolation_level: $isolation_level:expr)?
        $(, effect_contract: $effect_contract:expr)?
        $(, max_concurrent: $max_concurrent:expr)?
        $(,)?
    ) => {
        $(#[$attr])*
        $vis struct $name(pub $inner);

        impl $crate::Component for $name {
            type Kind = $crate::kind::Action;
            type Config = <$inner as $crate::Component>::Config;

            const KEY: &'static str = $key;

            fn metadata() -> $crate::Metadata {
                $metadata
            }
        }

        impl $crate::Action for $name {
            type Output = <$inner as $crate::Action>::Output;

            const ISOLATION_LEVEL: $crate::IsolationLevel = $crate::__entry_or!(
                $($isolation_level)?; <$inner as $crate::Action>::ISOLATION_LEVEL
            );
            const EFFECT_CONTRACT: $crate::ActionEffectContract = $crate::__entry_or!(
                $($effect_contract)?; <$inner as $crate::Action>::EFFECT_CONTRACT
            );
            const MAX_CONCURRENT: ::core::option::Option<::core::num::NonZeroU32> =
                $crate::__entry_or!(
                    $($max_concurrent)?; <$inner as $crate::Action>::MAX_CONCURRENT
                );

            fn inputs() -> ::std::vec::Vec<$crate::InputPort> {
                $crate::__entry_or!($($inputs)?; <$inner as $crate::Action>::inputs())
            }

            fn outputs() -> ::std::vec::Vec<$crate::OutputPort> {
                $crate::__entry_or!($($outputs)?; <$inner as $crate::Action>::outputs())
            }

            fn dependencies() -> &'static $crate::Dependencies {
                <$inner as $crate::Action>::dependencies()
            }
        }

        impl $crate::StatelessAction for $name {
            fn execute(
                &self,
                input: <Self as $crate::Component>::Config,
                ctx: &(impl $crate::ActionContext + ?Sized),
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<
                    $crate::ActionResult<<Self as $crate::Action>::Output>,
                    $crate::ActionError,
                >,
            > + Send {
                <$inner as $crate::StatelessAction>::execute(&self.0, input, ctx)
            }
        }
    };
}

/// The entry's own declaration when [`stateless_entry!`] was given one, else
/// the inner action's.
#[doc(hidden)]
#[macro_export]
macro_rules! __entry_or {
    (; $inherited:expr) => {
        $inherited
    };
    ($declared:expr; $inherited:expr) => {
        $declared
    };
}
