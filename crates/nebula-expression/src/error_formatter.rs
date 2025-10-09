//! Error formatting with source code context and visual highlighting
//!
//! This module provides beautiful error messages that show:
//! - The exact line and column where the error occurred
//! - Surrounding source code for context
//! - Visual highlighting with ^^^ under the error location

use crate::template::Position;

/// Format an error message with source context
pub struct ErrorFormatter<'a> {
    source: &'a str,
    position: Position,
    error_message: String,
    /// Number of lines to show before the error line
    context_before: usize,
    /// Number of lines to show after the error line
    context_after: usize,
}

impl<'a> ErrorFormatter<'a> {
    /// Create a new error formatter
    pub fn new(source: &'a str, position: Position, error_message: impl Into<String>) -> Self {
        Self {
            source,
            position,
            error_message: error_message.into(),
            context_before: 2,
            context_after: 1,
        }
    }

    /// Set the number of context lines to show
    pub fn with_context(mut self, before: usize, after: usize) -> Self {
        self.context_before = before;
        self.context_after = after;
        self
    }

    /// Format the error message with source context
    pub fn format(&self) -> String {
        let lines: Vec<&str> = self.source.lines().collect();
        let error_line_idx = self.position.line.saturating_sub(1);

        // Estimate output size: ~80 chars per line + header
        let num_lines = self.context_before + self.context_after + 2;
        let estimated_size = 200 + (num_lines * 80);
        let mut output = String::with_capacity(estimated_size);

        // Header with error message
        output.push_str(&format!("Error at {}:\n", self.position));
        output.push_str(&format!("  {}\n\n", self.error_message));

        // Calculate line number width for alignment
        let max_line_num = (error_line_idx + self.context_after + 1).min(lines.len());
        let line_num_width = max_line_num.to_string().len();

        // Show context before error
        let start_line = error_line_idx.saturating_sub(self.context_before);
        for i in start_line..error_line_idx {
            if i < lines.len() {
                output.push_str(&format!(
                    " {:width$} | {}\n",
                    i + 1,
                    lines[i],
                    width = line_num_width
                ));
            }
        }

        // Show error line
        if error_line_idx < lines.len() {
            output.push_str(&format!(
                " {:width$} | {}\n",
                error_line_idx + 1,
                lines[error_line_idx],
                width = line_num_width
            ));

            // Add highlighting under the error position
            let padding = " ".repeat(line_num_width + 3); // " N | "
            let column_padding = " ".repeat(self.position.column.saturating_sub(1));
            output.push_str(&format!("{}{}^\n", padding, column_padding));
        }

        // Show context after error
        let end_line = (error_line_idx + self.context_after + 1).min(lines.len());
        for i in (error_line_idx + 1)..end_line {
            output.push_str(&format!(
                " {:width$} | {}\n",
                i + 1,
                lines[i],
                width = line_num_width
            ));
        }

        output
    }

    /// Format with multi-character highlighting (for ranges)
    pub fn format_with_length(&self, length: usize) -> String {
        let lines: Vec<&str> = self.source.lines().collect();
        let error_line_idx = self.position.line.saturating_sub(1);

        // Estimate output size: ~80 chars per line + header + highlighting
        let num_lines = self.context_before + self.context_after + 2;
        let estimated_size = 200 + (num_lines * 80) + length;
        let mut output = String::with_capacity(estimated_size);

        // Header
        output.push_str(&format!("Error at {}:\n", self.position));
        output.push_str(&format!("  {}\n\n", self.error_message));

        let max_line_num = (error_line_idx + self.context_after + 1).min(lines.len());
        let line_num_width = max_line_num.to_string().len();

        // Context before
        let start_line = error_line_idx.saturating_sub(self.context_before);
        for i in start_line..error_line_idx {
            if i < lines.len() {
                output.push_str(&format!(
                    " {:width$} | {}\n",
                    i + 1,
                    lines[i],
                    width = line_num_width
                ));
            }
        }

        // Error line
        if error_line_idx < lines.len() {
            output.push_str(&format!(
                " {:width$} | {}\n",
                error_line_idx + 1,
                lines[error_line_idx],
                width = line_num_width
            ));

            // Multi-character highlighting
            let padding = " ".repeat(line_num_width + 3);
            let column_padding = " ".repeat(self.position.column.saturating_sub(1));
            let highlight = "^".repeat(length.max(1));
            output.push_str(&format!("{}{}{}\n", padding, column_padding, highlight));
        }

        // Context after
        let end_line = (error_line_idx + self.context_after + 1).min(lines.len());
        for i in (error_line_idx + 1)..end_line {
            output.push_str(&format!(
                " {:width$} | {}\n",
                i + 1,
                lines[i],
                width = line_num_width
            ));
        }

        output
    }
}

/// Helper to format template errors with source context
pub fn format_template_error(
    source: &str,
    position: Position,
    error_msg: &str,
    expression: Option<&str>,
) -> String {
    let formatter = ErrorFormatter::new(source, position, error_msg);

    let formatted = if let Some(expr) = expression {
        // Try to highlight the expression length
        let expr_len = expr.len();
        formatter.format_with_length(expr_len)
    } else {
        formatter.format()
    };

    if let Some(expr) = expression {
        format!("{}\nExpression: {}", formatted, expr)
    } else {
        formatted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_error_formatting() {
        let source = "Line 1\nLine 2 with error\nLine 3";
        let position = Position::new(2, 8, 15);
        let formatter = ErrorFormatter::new(source, position, "Variable not found");

        let output = formatter.format();
        assert!(output.contains("Error at line 2, column 8"));
        assert!(output.contains("Variable not found"));
        assert!(output.contains("Line 2 with error"));
        assert!(output.contains("^"));
    }

    #[test]
    fn test_error_with_context() {
        let source = "Line 1\nLine 2\nLine 3 ERROR\nLine 4\nLine 5";
        let position = Position::new(3, 8, 0);
        let formatter = ErrorFormatter::new(source, position, "Syntax error").with_context(2, 2);

        let output = formatter.format();
        assert!(output.contains("Line 1"));
        assert!(output.contains("Line 2"));
        assert!(output.contains("Line 3 ERROR"));
        assert!(output.contains("Line 4"));
        assert!(output.contains("Line 5"));
    }

    #[test]
    fn test_multichar_highlighting() {
        let source = "Hello {{ invalid_function() }} World";
        let position = Position::new(1, 10, 9);
        let formatter = ErrorFormatter::new(source, position, "Unknown function");

        let output = formatter.format_with_length(16); // "invalid_function"
        assert!(output.contains("^^^^^^^^^^^^^^^^"));
    }

    #[test]
    fn test_template_error_format() {
        let source = "<html>\n  <title>{{ $unknown }}</title>\n</html>";
        let position = Position::new(2, 14, 0);

        let output =
            format_template_error(source, position, "Undefined variable", Some("$unknown"));

        assert!(output.contains("Error at line 2, column 14"));
        assert!(output.contains("Undefined variable"));
        assert!(output.contains("<title>{{ $unknown }}</title>"));
        assert!(output.contains("Expression: $unknown"));
    }
}
