//! Generic error wrapper.
//!
//! [`NebulaError`] enriches any [`Classify`] error with optional message
//! overrides, typed detail metadata, a context chain, and a source error.

use std::borrow::Cow;
use std::error::Error;
use std::fmt;

use crate::details::{ErrorDetail, ErrorDetails};
use crate::traits::Classify;
use crate::{ErrorCategory, ErrorCode, ErrorSeverity, RetryHint};

/// The main error wrapper that enriches any [`Classify`] error with
/// details, context chain, and metadata.
///
/// Create one via [`NebulaError::new`] or the [`From`] impl, then
/// chain builder methods to attach a message, details, context, or
/// a source error.
///
/// # Examples
///
/// ```
/// use nebula_error::{
///     Classify, ErrorCategory, ErrorCode, ErrorSeverity,
///     NebulaError, ResourceInfo, codes,
/// };
///
/// #[derive(Debug)]
/// struct NotFound(String);
///
/// impl std::fmt::Display for NotFound {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "not found: {}", self.0)
///     }
/// }
///
/// impl Classify for NotFound {
///     fn category(&self) -> ErrorCategory { ErrorCategory::NotFound }
///     fn code(&self) -> ErrorCode { codes::NOT_FOUND.clone() }
/// }
///
/// let err = NebulaError::new(NotFound("workflow-42".into()))
///     .with_message("workflow does not exist")
///     .with_detail(ResourceInfo {
///         resource_type: "workflow".into(),
///         resource_name: "workflow-42".into(),
///         owner: None,
///     })
///     .context("while loading execution plan");
///
/// assert_eq!(err.category(), ErrorCategory::NotFound);
/// assert_eq!(err.to_string(), "workflow does not exist");
/// ```
pub struct NebulaError<E: Classify> {
    inner: E,
    message: Option<Cow<'static, str>>,
    details: ErrorDetails,
    context_chain: Vec<Cow<'static, str>>,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl<E: Classify> NebulaError<E> {
    /// Wraps a domain error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{Classify, ErrorCategory, ErrorCode, NebulaError, codes};
    ///
    /// #[derive(Debug)]
    /// struct MyErr;
    /// impl std::fmt::Display for MyErr {
    ///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    ///         f.write_str("my error")
    ///     }
    /// }
    /// impl Classify for MyErr {
    ///     fn category(&self) -> ErrorCategory { ErrorCategory::Internal }
    ///     fn code(&self) -> ErrorCode { codes::INTERNAL.clone() }
    /// }
    ///
    /// let err = NebulaError::new(MyErr);
    /// assert_eq!(err.category(), ErrorCategory::Internal);
    /// ```
    pub fn new(inner: E) -> Self {
        Self {
            inner,
            message: None,
            details: ErrorDetails::new(),
            context_chain: Vec::new(),
            source: None,
        }
    }

    /// Overrides the display message.
    ///
    /// When set, [`Display`] uses this message instead of the inner
    /// error's display.
    #[must_use]
    pub fn with_message(mut self, msg: impl Into<Cow<'static, str>>) -> Self {
        self.message = Some(msg.into());
        self
    }

    /// Attaches a source error for the [`Error::source`] chain.
    #[must_use]
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Inserts a typed detail into the detail map.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{
    ///     Classify, ErrorCategory, ErrorCode, NebulaError, RetryInfo, codes,
    /// };
    /// use std::time::Duration;
    ///
    /// # #[derive(Debug)]
    /// # struct E;
    /// # impl std::fmt::Display for E {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    /// #         f.write_str("e")
    /// #     }
    /// # }
    /// # impl Classify for E {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::Timeout }
    /// #     fn code(&self) -> ErrorCode { codes::TIMEOUT.clone() }
    /// # }
    /// let err = NebulaError::new(E)
    ///     .with_detail(RetryInfo {
    ///         retry_delay: Some(Duration::from_secs(1)),
    ///         max_attempts: Some(3),
    ///     });
    ///
    /// assert!(err.detail::<RetryInfo>().is_some());
    /// ```
    #[must_use]
    pub fn with_detail<D: ErrorDetail>(mut self, detail: D) -> Self {
        self.details.insert(detail);
        self
    }

    /// Pushes a context string onto the context chain.
    ///
    /// Context entries are ordered from innermost (first pushed) to
    /// outermost (last pushed).
    #[must_use]
    pub fn context(mut self, ctx: impl Into<Cow<'static, str>>) -> Self {
        self.context_chain.push(ctx.into());
        self
    }

    // --- Delegating accessors ---

    /// The broad category of this error, delegated to the inner type.
    pub fn category(&self) -> ErrorCategory {
        self.inner.category()
    }

    /// The severity of this error, delegated to the inner type.
    pub fn severity(&self) -> ErrorSeverity {
        self.inner.severity()
    }

    /// The machine-readable error code, delegated to the inner type.
    pub fn error_code(&self) -> ErrorCode {
        self.inner.code()
    }

    /// Whether this error is retryable, delegated to the inner type.
    pub fn is_retryable(&self) -> bool {
        self.inner.is_retryable()
    }

    /// Advisory retry hint, delegated to the inner type.
    pub fn retry_hint(&self) -> Option<RetryHint> {
        self.inner.retry_hint()
    }

    // --- Domain access ---

    /// Returns a reference to the wrapped domain error.
    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Unwraps and returns the domain error, discarding all metadata.
    pub fn into_inner(self) -> E {
        self.inner
    }

    /// Transforms the inner error type while preserving all metadata
    /// (message, details, context chain, source).
    ///
    /// Useful for converting between crate-specific error types when
    /// propagating errors across crate boundaries.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::{Classify, ErrorCategory, ErrorCode, NebulaError, codes};
    ///
    /// # #[derive(Debug)]
    /// # struct A;
    /// # impl std::fmt::Display for A {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a") }
    /// # }
    /// # impl Classify for A {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::Internal }
    /// #     fn code(&self) -> ErrorCode { codes::INTERNAL.clone() }
    /// # }
    /// # #[derive(Debug)]
    /// # struct B;
    /// # impl std::fmt::Display for B {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("b") }
    /// # }
    /// # impl Classify for B {
    /// #     fn category(&self) -> ErrorCategory { ErrorCategory::External }
    /// #     fn code(&self) -> ErrorCode { codes::EXTERNAL.clone() }
    /// # }
    /// let err = NebulaError::new(A).with_message("msg").context("ctx");
    /// let mapped: NebulaError<B> = err.map_inner(|_| B);
    /// assert_eq!(mapped.to_string(), "msg");
    /// ```
    #[must_use]
    pub fn map_inner<F: Classify>(self, f: impl FnOnce(E) -> F) -> NebulaError<F> {
        NebulaError {
            inner: f(self.inner),
            message: self.message,
            details: self.details,
            context_chain: self.context_chain,
            source: self.source,
        }
    }

    // --- Detail access ---

    /// Returns a reference to a specific detail type, if present.
    pub fn detail<D: ErrorDetail>(&self) -> Option<&D> {
        self.details.get::<D>()
    }

    /// Returns a reference to the full detail map.
    pub fn details(&self) -> &ErrorDetails {
        &self.details
    }

    /// Returns a mutable reference to the detail map.
    pub fn details_mut(&mut self) -> &mut ErrorDetails {
        &mut self.details
    }

    // --- Context access ---

    /// Returns the context chain as a slice, innermost first.
    pub fn context_chain(&self) -> &[Cow<'static, str>] {
        &self.context_chain
    }

    /// Returns the source error, if one was attached.
    pub fn source(&self) -> Option<&(dyn Error + Send + Sync)> {
        self.source.as_deref()
    }
}

impl<E: Classify + fmt::Display> fmt::Display for NebulaError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(msg) = &self.message {
            f.write_str(msg)
        } else {
            fmt::Display::fmt(&self.inner, f)
        }
    }
}

impl<E: Classify + fmt::Debug> fmt::Debug for NebulaError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NebulaError")
            .field("inner", &self.inner)
            .field("category", &self.inner.category())
            .field("severity", &self.inner.severity())
            .field("code", &self.inner.code())
            .field("retryable", &self.inner.is_retryable())
            .field("details", &self.details)
            .field("context", &self.context_chain)
            .finish()
    }
}

impl<E: Classify + fmt::Debug + fmt::Display> Error for NebulaError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn Error + 'static))
    }
}

impl<E: Classify> Classify for NebulaError<E> {
    fn category(&self) -> ErrorCategory {
        self.inner.category()
    }

    fn code(&self) -> ErrorCode {
        self.inner.code()
    }

    fn severity(&self) -> ErrorSeverity {
        self.inner.severity()
    }

    fn is_retryable(&self) -> bool {
        self.inner.is_retryable()
    }

    fn retry_hint(&self) -> Option<RetryHint> {
        self.inner.retry_hint()
    }
}

impl<E: Classify> From<E> for NebulaError<E> {
    fn from(inner: E) -> Self {
        Self::new(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResourceInfo, RetryInfo, codes};
    use std::time::Duration;

    #[derive(Debug, Clone)]
    struct TestError {
        cat: ErrorCategory,
        sev: ErrorSeverity,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "test error ({})", self.cat)
        }
    }

    impl Classify for TestError {
        fn category(&self) -> ErrorCategory {
            self.cat
        }
        fn code(&self) -> ErrorCode {
            codes::INTERNAL.clone()
        }
        fn severity(&self) -> ErrorSeverity {
            self.sev
        }
    }

    fn make_error() -> TestError {
        TestError {
            cat: ErrorCategory::Internal,
            sev: ErrorSeverity::Error,
        }
    }

    #[test]
    fn new_wraps_inner() {
        let inner = make_error();
        let err = NebulaError::new(inner.clone());
        assert_eq!(err.inner().cat, inner.cat);
        assert_eq!(err.category(), ErrorCategory::Internal);
    }

    #[test]
    fn from_conversion() {
        fn fallible() -> Result<(), NebulaError<TestError>> {
            Err(make_error())?
        }
        let result = fallible();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().category(), ErrorCategory::Internal);
    }

    #[test]
    fn with_message_overrides_display() {
        let err = NebulaError::new(make_error()).with_message("custom message");
        assert_eq!(err.to_string(), "custom message");
    }

    #[test]
    fn default_display_uses_inner() {
        let err = NebulaError::new(make_error());
        assert_eq!(err.to_string(), "test error (internal)");
    }

    #[test]
    fn context_chain() {
        let err = NebulaError::new(make_error())
            .context("loading workflow")
            .context("executing step 3");
        let chain = err.context_chain();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], "loading workflow");
        assert_eq!(chain[1], "executing step 3");
    }

    #[test]
    fn with_detail() {
        let err = NebulaError::new(make_error())
            .with_detail(RetryInfo {
                retry_delay: Some(Duration::from_secs(5)),
                max_attempts: Some(3),
            })
            .with_detail(ResourceInfo {
                resource_type: "workflow".into(),
                resource_name: "wf-1".into(),
                owner: None,
            });

        let retry = err.detail::<RetryInfo>().expect("retry info");
        assert_eq!(retry.max_attempts, Some(3));

        let resource = err.detail::<ResourceInfo>().expect("resource info");
        assert_eq!(resource.resource_name, "wf-1");
    }

    #[test]
    fn into_inner_recovers_domain_error() {
        let inner = make_error();
        let err = NebulaError::new(inner).with_message("gone").context("ctx");
        let recovered = err.into_inner();
        assert_eq!(recovered.cat, ErrorCategory::Internal);
    }

    #[test]
    fn with_source_chains_error() {
        let source = std::io::Error::new(std::io::ErrorKind::NotFound, "file gone");
        let err = NebulaError::new(make_error()).with_source(source);

        let src = err.source().expect("should have source");
        assert!(src.to_string().contains("file gone"));

        // Also check std::error::Error::source
        let std_src = Error::source(&err).expect("std source");
        assert!(std_src.to_string().contains("file gone"));
    }

    #[test]
    fn map_inner_transforms_error_type() {
        #[derive(Debug, Clone)]
        struct OtherError(ErrorCategory);

        impl fmt::Display for OtherError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "other({})", self.0)
            }
        }

        impl Classify for OtherError {
            fn category(&self) -> ErrorCategory {
                self.0
            }
            fn code(&self) -> ErrorCode {
                codes::EXTERNAL.clone()
            }
        }

        let original = NebulaError::new(make_error())
            .with_message("test msg")
            .context("ctx1");

        let mapped = original.map_inner(|_inner| OtherError(ErrorCategory::External));

        assert_eq!(mapped.category(), ErrorCategory::External);
        assert_eq!(mapped.to_string(), "test msg");
        assert_eq!(mapped.context_chain().len(), 1);
    }

    #[test]
    fn severity_delegates_to_inner() {
        let warning_err = TestError {
            cat: ErrorCategory::RateLimit,
            sev: ErrorSeverity::Warning,
        };
        let err = NebulaError::new(warning_err);
        assert_eq!(err.severity(), ErrorSeverity::Warning);
    }
}
