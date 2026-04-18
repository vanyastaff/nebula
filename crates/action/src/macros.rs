//! Assertion macros for testing action results.
//!
//! These macros simplify asserting on [`ActionResult`](crate::ActionResult) and
//! [`ActionError`](crate::ActionError) variants in tests. They are exported via
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
/// ```rust,ignore
/// let result = action.execute(input, &ctx).await;
/// assert_success!(result);
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
                    selected, $key,
                    "expected branch key '{}', got '{}'",
                    $key, selected
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

/// Assert that the result is `Ok(ActionResult::Retry { .. })`.
///
/// Gated behind the `unstable-retry-scheduler` feature: the `Retry` variant is
/// not part of the public contract until the engine retry scheduler lands
/// (canon §11.2).
///
/// # Panics
///
/// Panics if the result is not `Ok(ActionResult::Retry { .. })`.
#[cfg(feature = "unstable-retry-scheduler")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable-retry-scheduler")))]
#[macro_export]
macro_rules! assert_retry {
    ($result:expr) => {
        match &$result {
            Ok($crate::ActionResult::Retry { .. }) => {},
            other => panic!("expected ActionResult::Retry, got {:?}", other),
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
