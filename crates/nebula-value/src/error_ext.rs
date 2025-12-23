//! Enhanced error handling with context, suggestions, and traces.
//!
//! This module extends the base [`ValueError`] with additional context,
//! suggestions for fixing errors, and detailed error traces.

use crate::core::ValueError;
use std::fmt;

/// Error with enhanced context and suggestions.
///
/// Wraps a [`ValueError`] with additional diagnostic information
/// to help users understand and fix errors.
///
/// # Examples
///
/// ```
/// use nebula_value::error_ext::EnhancedError;
/// use nebula_value::ValueError;
///
/// let error = ValueError::type_mismatch("String", "Integer");
/// let enhanced = EnhancedError::new(error)
///     .with_suggestion("Try using Value::text() instead of Value::integer()")
///     .with_hint("String values must be created with Value::text()");
///
/// println!("{}", enhanced);
/// ```
#[derive(Debug, Clone)]
pub struct EnhancedError {
    /// The underlying error
    error: ValueError,

    /// Additional context about where/why the error occurred
    context: Vec<String>,

    /// Suggestions for fixing the error
    suggestions: Vec<String>,

    /// Hints about what might have caused the error
    hints: Vec<String>,

    /// Related documentation URLs
    docs: Vec<String>,

    /// Error location (file, line, column)
    location: Option<ErrorLocation>,
}

/// Location information for an error.
#[derive(Debug, Clone)]
pub struct ErrorLocation {
    /// File where the error occurred
    pub file: String,

    /// Line number
    pub line: u32,

    /// Column number
    pub column: Option<u32>,
}

impl EnhancedError {
    /// Create a new enhanced error from a ValueError.
    pub fn new(error: ValueError) -> Self {
        Self {
            error,
            context: Vec::new(),
            suggestions: Vec::new(),
            hints: Vec::new(),
            docs: Vec::new(),
            location: None,
        }
    }

    /// Add contextual information.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::error_ext::EnhancedError;
    /// use nebula_value::ValueError;
    ///
    /// let err = EnhancedError::new(ValueError::key_not_found("name"))
    ///     .with_context("While processing user data")
    ///     .with_context("In request from client 192.168.1.1");
    /// ```
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context.push(context.into());
        self
    }

    /// Add a suggestion for fixing the error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::error_ext::EnhancedError;
    /// use nebula_value::ValueError;
    ///
    /// let err = EnhancedError::new(ValueError::type_mismatch("String", "Integer"))
    ///     .with_suggestion("Use Value::text() to create a string value")
    ///     .with_suggestion("Or use to_integer() to convert the string to an integer");
    /// ```
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestions.push(suggestion.into());
        self
    }

    /// Add a hint about what might have caused the error.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    /// Add a documentation URL.
    pub fn with_doc(mut self, url: impl Into<String>) -> Self {
        self.docs.push(url.into());
        self
    }

    /// Add location information.
    pub fn with_location(mut self, file: String, line: u32, column: Option<u32>) -> Self {
        self.location = Some(ErrorLocation { file, line, column });
        self
    }

    /// Get the underlying error.
    pub fn inner(&self) -> &ValueError {
        &self.error
    }

    /// Consume self and return the underlying error.
    pub fn into_inner(self) -> ValueError {
        self.error
    }

    /// Get the error code.
    pub fn code(&self) -> &'static str {
        self.error.code()
    }

    /// Get all context messages.
    pub fn context(&self) -> &[String] {
        &self.context
    }

    /// Get all suggestions.
    pub fn suggestions(&self) -> &[String] {
        &self.suggestions
    }

    /// Get all hints.
    pub fn hints(&self) -> &[String] {
        &self.hints
    }
}

impl fmt::Display for EnhancedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "colored-errors")]
        {
            use colored::Colorize;

            // Main error message (red, bold)
            writeln!(f, "{} {}", "Error:".red().bold(), self.error.to_string().red())?;

            // Error code (cyan)
            writeln!(f, "{} {}", "Code:".cyan(), self.error.code().cyan())?;

            // Location (yellow)
            if let Some(ref loc) = self.location {
                write!(f, "{} {}:{}", "Location:".yellow(), loc.file, loc.line)?;
                if let Some(col) = loc.column {
                    write!(f, ":{}", col)?;
                }
                writeln!(f)?;
            }

            // Context (reverse order - most recent first)
            if !self.context.is_empty() {
                writeln!(f, "\n{}", "Context:".blue().bold())?;
                for (i, ctx) in self.context.iter().rev().enumerate() {
                    writeln!(f, "  {}. {}", (i + 1).to_string().blue(), ctx)?;
                }
            }

            // Hints (magenta)
            if !self.hints.is_empty() {
                writeln!(f, "\n{}", "Possible causes:".magenta().bold())?;
                for hint in &self.hints {
                    writeln!(f, "  {} {}", "•".magenta(), hint)?;
                }
            }

            // Suggestions (green)
            if !self.suggestions.is_empty() {
                writeln!(f, "\n{}", "Suggestions:".green().bold())?;
                for suggestion in &self.suggestions {
                    writeln!(f, "  {} {}", "➜".green(), suggestion.green())?;
                }
            }

            // Documentation (bright blue)
            if !self.docs.is_empty() {
                writeln!(f, "\n{}", "Documentation:".bright_blue().bold())?;
                for doc in &self.docs {
                    writeln!(f, "  {} {}", "📚".bright_blue(), doc.bright_blue().underline())?;
                }
            }
        }

        #[cfg(not(feature = "colored-errors"))]
        {
            // Main error message
            writeln!(f, "Error: {}", self.error)?;

            // Error code
            writeln!(f, "Code: {}", self.error.code())?;

            // Location
            if let Some(ref loc) = self.location {
                write!(f, "Location: {}:{}", loc.file, loc.line)?;
                if let Some(col) = loc.column {
                    write!(f, ":{}", col)?;
                }
                writeln!(f)?;
            }

            // Context (reverse order - most recent first)
            if !self.context.is_empty() {
                writeln!(f, "\nContext:")?;
                for (i, ctx) in self.context.iter().rev().enumerate() {
                    writeln!(f, "  {}. {}", i + 1, ctx)?;
                }
            }

            // Hints
            if !self.hints.is_empty() {
                writeln!(f, "\nPossible causes:")?;
                for hint in &self.hints {
                    writeln!(f, "  • {}", hint)?;
                }
            }

            // Suggestions
            if !self.suggestions.is_empty() {
                writeln!(f, "\nSuggestions:")?;
                for suggestion in &self.suggestions {
                    writeln!(f, "  ➜ {}", suggestion)?;
                }
            }

            // Documentation
            if !self.docs.is_empty() {
                writeln!(f, "\nDocumentation:")?;
                for doc in &self.docs {
                    writeln!(f, "  📚 {}", doc)?;
                }
            }
        }

        Ok(())
    }
}

impl std::error::Error for EnhancedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl From<ValueError> for EnhancedError {
    fn from(error: ValueError) -> Self {
        Self::new(error)
    }
}

/// Extension trait for ValueError to create enhanced errors.
pub trait ValueErrorExt {
    /// Convert to an enhanced error.
    fn enhanced(self) -> EnhancedError;

    /// Add a suggestion to the error.
    fn suggest(self, suggestion: impl Into<String>) -> EnhancedError;

    /// Add context to the error.
    fn context(self, context: impl Into<String>) -> EnhancedError;
}

impl ValueErrorExt for ValueError {
    fn enhanced(self) -> EnhancedError {
        EnhancedError::new(self)
    }

    fn suggest(self, suggestion: impl Into<String>) -> EnhancedError {
        EnhancedError::new(self).with_suggestion(suggestion)
    }

    fn context(self, context: impl Into<String>) -> EnhancedError {
        EnhancedError::new(self).with_context(context)
    }
}

/// Result type using EnhancedError.
pub type EnhancedResult<T> = Result<T, EnhancedError>;

/// Builder for creating common enhanced errors with appropriate suggestions.
pub struct ErrorBuilder;

impl ErrorBuilder {
    /// Create a type mismatch error with suggestions.
    pub fn type_mismatch(expected: &str, actual: &str) -> EnhancedError {
        let error = ValueError::type_mismatch(expected, actual);
        let mut enhanced = EnhancedError::new(error);

        // Add type-specific suggestions
        match (expected, actual) {
            ("String", "Integer") => {
                enhanced = enhanced
                    .with_suggestion("Use Value::text() to create a string value")
                    .with_suggestion("Or use to_integer() to convert the string to an integer")
                    .with_hint("String and Integer are not compatible types");
            }
            ("Integer", "String") => {
                enhanced = enhanced
                    .with_suggestion("Use Value::integer() to create an integer value")
                    .with_suggestion("Or use to_string() to convert the integer to a string")
                    .with_suggestion("Or parse the string with .parse::<i64>()")
                    .with_hint("The value contains text, not a number");
            }
            ("Array", _) => {
                enhanced = enhanced
                    .with_suggestion("Use Value::Array(...) or array![] macro")
                    .with_hint("Expected a collection of values");
            }
            ("Object", _) => {
                enhanced = enhanced
                    .with_suggestion("Use Value::Object(...) or object!{} macro")
                    .with_hint("Expected a key-value map");
            }
            _ => {}
        }

        enhanced
    }

    /// Create a key not found error with suggestions.
    pub fn key_not_found(key: &str, available_keys: &[String]) -> EnhancedError {
        let error = ValueError::key_not_found(key);
        let mut enhanced = EnhancedError::new(error)
            .with_hint(format!("The key '{}' does not exist in the object", key));

        // Suggest similar keys if any
        if !available_keys.is_empty() {
            enhanced = enhanced.with_context(format!("Available keys: {}", available_keys.join(", ")));

            // Find similar keys (simple Levenshtein-like similarity)
            let similar: Vec<_> = available_keys
                .iter()
                .filter(|k| k.contains(key) || key.contains(k.as_str()))
                .collect();

            if !similar.is_empty() {
                for sim in similar {
                    enhanced = enhanced.with_suggestion(format!("Did you mean '{}'?", sim));
                }
            } else {
                enhanced = enhanced.with_suggestion("Check the object structure");
                enhanced = enhanced.with_suggestion("Use .keys() to see all available keys");
            }
        }

        enhanced
    }

    /// Create an index out of bounds error with suggestions.
    pub fn index_out_of_bounds(index: usize, length: usize) -> EnhancedError {
        let error = ValueError::index_out_of_bounds(index, length);
        EnhancedError::new(error)
            .with_hint(format!("Array has {} elements (indices 0..{})", length, length.saturating_sub(1)))
            .with_suggestion(format!("Use an index between 0 and {}", length.saturating_sub(1)))
            .with_suggestion("Check the array length with .len() before accessing")
            .with_suggestion("Use .get() which returns Option instead of panicking")
    }

    /// Create a conversion error with suggestions.
    pub fn conversion_error(from: &str, to: &str, value: &str) -> EnhancedError {
        let error = ValueError::conversion_error(from, to);
        let mut enhanced = EnhancedError::new(error)
            .with_context(format!("Value: {}", value))
            .with_hint(format!("Cannot convert {} to {}", from, to));

        // Add specific suggestions based on conversion type
        if to == "Integer" && from == "Text" {
            enhanced = enhanced
                .with_suggestion("Ensure the string contains only digits")
                .with_suggestion("Example: \"42\" can be converted, but \"abc\" cannot")
                .with_suggestion("Use to_integer() for conversion with error handling");
        }

        enhanced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_error_display() {
        let error = ValueError::key_not_found("username");
        let enhanced = EnhancedError::new(error)
            .with_context("Processing user profile")
            .with_context("In API request /api/users/123")
            .with_suggestion("Check that the user object has a 'username' field")
            .with_hint("The field might be named differently (e.g., 'user_name')")
            .with_doc("https://docs.rs/nebula-value");

        let output = format!("{}", enhanced);
        assert!(output.contains("Error:"));
        assert!(output.contains("Code:"));
        assert!(output.contains("Context:"));
        assert!(output.contains("Suggestions:"));
        assert!(output.contains("Possible causes:"));
    }

    #[test]
    fn test_error_builder_type_mismatch() {
        let error = ErrorBuilder::type_mismatch("Integer", "String");
        assert_eq!(error.code(), "VALUE_TYPE_MISMATCH");
        assert!(!error.suggestions().is_empty());
    }

    #[test]
    fn test_error_builder_key_not_found() {
        let available = vec!["name".to_string(), "age".to_string(), "email".to_string()];
        let error = ErrorBuilder::key_not_found("username", &available);

        let output = format!("{}", error);
        assert!(output.contains("Available keys"));
    }

    #[test]
    fn test_value_error_ext() {
        let error = ValueError::type_mismatch("String", "Integer")
            .suggest("Use Value::text() instead");

        assert_eq!(error.suggestions().len(), 1);
    }
}
