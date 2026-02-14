//! Template engine with parsing, caching, and detailed error reporting
//!
//! This module provides a Template type that can parse templates with {{ }} expressions,
//! cache the parsed structure for fast rendering, and provide detailed error information
//! including line and column numbers.

use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::engine::ExpressionEngine;
use crate::error_formatter::format_template_error;
use nebula_log::trace;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Maximum number of expressions allowed in a single template (DoS protection)
const MAX_TEMPLATE_EXPRESSIONS: usize = 1000;

/// A template part - either static text or an expression to evaluate
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    /// Static text that doesn't need evaluation
    Static {
        /// The static text content
        content: Arc<str>,
        /// Starting position in the original template
        position: Position,
    },
    /// An expression to be evaluated
    Expression {
        /// The expression content (without {{ }})
        content: Arc<str>,
        /// Starting position of {{ in the original template
        position: Position,
        /// Length of the full {{ expression }} in characters
        length: usize,
        /// Strip whitespace to the left ({{-)
        strip_left: bool,
        /// Strip whitespace to the right (-}})
        strip_right: bool,
    },
}

/// Position in the template (line and column)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Line number (1-based)
    pub line: usize,
    /// Column number (1-based)
    pub column: usize,
    /// Absolute character offset (0-based)
    pub offset: usize,
}

impl Position {
    /// Create a new position
    pub fn new(line: usize, column: usize, offset: usize) -> Self {
        Self {
            line,
            column,
            offset,
        }
    }

    /// Position at the start of input
    pub fn start() -> Self {
        Self {
            line: 1,
            column: 1,
            offset: 0,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// A parsed template with cached structure
#[derive(Debug, Clone)]
pub struct Template {
    /// Original template source
    source: Arc<str>,
    /// Parsed template parts (cached after first parse)
    parts: Vec<TemplatePart>,
}

impl Template {
    /// Create a new template from a string
    ///
    /// This will parse the template immediately and cache the structure.
    pub fn new(source: impl Into<String>) -> ExpressionResult<Self> {
        let source_str = source.into();
        let parts = Self::parse(&source_str)?;
        let source = Arc::from(source_str.as_str());
        Ok(Self { source, parts })
    }

    /// Get the original source string
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Get the parsed parts
    pub fn parts(&self) -> &[TemplatePart] {
        &self.parts
    }

    /// Render the template with the given context
    pub fn render(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> ExpressionResult<String> {
        let mut result = String::with_capacity(self.source.len());
        let mut strip_next_leading = false;

        for part in &self.parts {
            match part {
                TemplatePart::Static { content, .. } => {
                    if strip_next_leading {
                        result.push_str(content.trim_start());
                        strip_next_leading = false;
                    } else {
                        result.push_str(content);
                    }
                }
                TemplatePart::Expression {
                    content,
                    position,
                    strip_left,
                    strip_right,
                    ..
                } => {
                    trace!(
                        expression = &**content,
                        position = %position,
                        strip_left = strip_left,
                        strip_right = strip_right,
                        "Rendering template expression"
                    );

                    // Strip whitespace on the left if requested
                    if *strip_left {
                        // Truncate in-place instead of allocating new String
                        let trimmed_len = result.trim_end().len();
                        result.truncate(trimmed_len);
                    }

                    match engine.evaluate(content.trim(), context) {
                        Ok(value) => {
                            match value.as_str() {
                                Some(s) => result.push_str(s),
                                None => result.push_str(&value.to_string()),
                            }

                            // Mark that we should strip leading whitespace from next static part
                            if *strip_right {
                                strip_next_leading = true;
                            }
                        }
                        Err(e) => {
                            // Create beautiful error message with source context
                            let formatted_error = format_template_error(
                                &self.source,
                                *position,
                                &e.to_string(),
                                Some(content.trim()),
                            );
                            return Err(ExpressionError::expression_eval_error(formatted_error));
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    /// Parse a template string into parts
    fn parse(source: &str) -> ExpressionResult<Vec<TemplatePart>> {
        let mut parts = Vec::new();
        let mut current_static = String::new();
        let mut static_start = Position::start();

        let chars: Vec<char> = source.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut line = 1;
        let mut column = 1;

        while i < len {
            // Look for opening {{
            if i + 1 < len && chars[i] == '{' && chars[i + 1] == '{' {
                // Save any accumulated static content
                if !current_static.is_empty() {
                    parts.push(TemplatePart::Static {
                        content: Arc::from(current_static.as_str()),
                        position: static_start,
                    });
                    current_static.clear();
                }

                let expr_start = Position::new(line, column, i);

                // Find closing }}
                let mut j = i + 2;
                let mut depth = 1;
                let mut expr_line = line;
                let mut expr_column = column + 2;

                while j + 1 < len {
                    if chars[j] == '\n' {
                        expr_line += 1;
                        expr_column = 1;
                    }

                    if chars[j] == '{' && chars[j + 1] == '{' {
                        depth += 1;
                        j += 2;
                        expr_column += 2;
                    } else if chars[j] == '}' && chars[j + 1] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        j += 2;
                        expr_column += 2;
                    } else {
                        j += 1;
                        expr_column += 1;
                    }
                }

                if depth == 0 && j + 1 < len {
                    // Check for whitespace control markers
                    let mut expr_start_idx = i + 2;
                    let mut expr_end_idx = j;
                    let mut strip_left = false;
                    let mut strip_right = false;

                    // Check for {{- (strip left)
                    if expr_start_idx < len && chars[expr_start_idx] == '-' {
                        strip_left = true;
                        expr_start_idx += 1;
                    }

                    // Check for -}} (strip right)
                    if expr_end_idx > 0 && chars[expr_end_idx - 1] == '-' {
                        strip_right = true;
                        expr_end_idx -= 1;
                    }

                    // Extract the expression content (without whitespace markers)
                    let expr_content: String = chars[expr_start_idx..expr_end_idx].iter().collect();
                    let full_length = j + 2 - i;

                    parts.push(TemplatePart::Expression {
                        content: Arc::from(expr_content.as_str()),
                        position: expr_start,
                        length: full_length,
                        strip_left,
                        strip_right,
                    });

                    // Check expression count limit (DoS protection)
                    let expr_count = parts
                        .iter()
                        .filter(|p| matches!(p, TemplatePart::Expression { .. }))
                        .count();
                    if expr_count > MAX_TEMPLATE_EXPRESSIONS {
                        return Err(ExpressionError::expression_parse_error(format!(
                            "Template contains too many expressions: {} (max {})",
                            expr_count, MAX_TEMPLATE_EXPRESSIONS
                        )));
                    }

                    // Update position tracking
                    i = j + 2;
                    line = expr_line;
                    column = expr_column + 2;
                    static_start = Position::new(line, column, i);
                } else {
                    // Unclosed {{ - this is an error
                    let formatted_error = format_template_error(
                        source,
                        expr_start,
                        "Unclosed '{{' - expected closing '}}'",
                        None,
                    );
                    return Err(ExpressionError::expression_parse_error(formatted_error));
                }
            } else {
                // Regular character
                current_static.push(chars[i]);
                i += 1;

                // Track newlines
                if chars[i - 1] == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
            }
        }

        // Add any remaining static content
        if !current_static.is_empty() {
            parts.push(TemplatePart::Static {
                content: Arc::from(current_static.as_str()),
                position: static_start,
            });
        }

        Ok(parts)
    }

    /// Check if the template contains any expressions
    pub fn has_expressions(&self) -> bool {
        self.parts
            .iter()
            .any(|part| matches!(part, TemplatePart::Expression { .. }))
    }

    /// Get the number of expressions in the template
    pub fn expression_count(&self) -> usize {
        self.parts
            .iter()
            .filter(|part| matches!(part, TemplatePart::Expression { .. }))
            .count()
    }

    /// Get all expression contents
    pub fn expressions(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                TemplatePart::Expression { content, .. } => Some(&**content),
                _ => None,
            })
            .collect()
    }
}

impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

/// A template that can be either unresolved (template string) or resolved (final value)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybeTemplate {
    /// A template that needs to be rendered
    Template(String),
    /// An already resolved value
    Resolved(String),
}

impl MaybeTemplate {
    /// Create from a string, automatically detecting if it's a template
    pub fn from_string(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.contains("{{") && s.contains("}}") {
            Self::Template(s)
        } else {
            Self::Resolved(s)
        }
    }

    /// Check if this is a template
    pub fn is_template(&self) -> bool {
        matches!(self, Self::Template(_))
    }

    /// Resolve the template or return the resolved value
    pub fn resolve(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> ExpressionResult<String> {
        match self {
            Self::Template(template_str) => {
                let template = Template::new(template_str)?;
                template.render(engine, context)
            }
            Self::Resolved(value) => Ok(value.clone()),
        }
    }

    /// Get the underlying string (template or resolved)
    pub fn as_str(&self) -> &str {
        match self {
            Self::Template(s) | Self::Resolved(s) => s,
        }
    }
}

impl From<String> for MaybeTemplate {
    fn from(s: String) -> Self {
        Self::from_string(s)
    }
}

impl From<&str> for MaybeTemplate {
    fn from(s: &str) -> Self {
        Self::from_string(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExpressionEngine;
    use serde_json::Value;

    #[test]
    fn test_template_parse_static_only() {
        let template = Template::new("Hello, World!").unwrap();
        assert_eq!(template.parts().len(), 1);
        assert!(!template.has_expressions());
        assert_eq!(template.expression_count(), 0);
    }

    #[test]
    fn test_template_parse_single_expression() {
        let template = Template::new("Hello {{ $input }}!").unwrap();
        assert_eq!(template.parts().len(), 3);
        assert!(template.has_expressions());
        assert_eq!(template.expression_count(), 1);
    }

    #[test]
    fn test_template_parse_multiple_expressions() {
        let template = Template::new("{{ $a }} + {{ $b }} = {{ $a + $b }}").unwrap();
        assert_eq!(template.expression_count(), 3);
        let exprs = template.expressions();
        assert_eq!(exprs, vec![" $a ", " $b ", " $a + $b "]);
    }

    #[test]
    fn test_template_render_simple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("World".to_string()));

        let template = Template::new("Hello {{ $input }}!").unwrap();
        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_template_render_multiple() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::Number(5.into()));

        let template = Template::new("{{ $input }} * 2 = {{ $input * 2 }}").unwrap();
        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "5 * 2 = 10");
    }

    #[test]
    fn test_template_render_with_functions() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("hello".to_string()));

        let template = Template::new("{{ $input | uppercase() }}").unwrap();
        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "HELLO");
    }

    #[test]
    fn test_template_unclosed_expression() {
        let result = Template::new("Hello {{ $input");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unclosed"));
    }

    #[test]
    fn test_template_multiline() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("Alice".to_string()));

        let template = Template::new(
            r#"Line 1: {{ $input }}
Line 2: {{ $input | uppercase() }}
Line 3: Done"#,
        )
        .unwrap();

        let result = template.render(&engine, &context).unwrap();
        assert!(result.contains("Line 1: Alice"));
        assert!(result.contains("Line 2: ALICE"));
        assert!(result.contains("Line 3: Done"));
    }

    #[test]
    fn test_template_error_with_position() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let template = Template::new("Hello {{ invalid_func() }}!").unwrap();
        let result = template.render(&engine, &context);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("line 1"));
    }

    #[test]
    fn test_maybe_template_auto_detection() {
        let template = MaybeTemplate::from_string("Hello {{ $input }}");
        assert!(template.is_template());

        let resolved = MaybeTemplate::from_string("Hello World");
        assert!(!resolved.is_template());
    }

    #[test]
    fn test_maybe_template_resolve() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("World".to_string()));

        let template = MaybeTemplate::from_string("Hello {{ $input }}!");
        let result = template.resolve(&engine, &context).unwrap();
        assert_eq!(result, "Hello World!");

        let resolved = MaybeTemplate::from_string("Hello World!");
        let result = resolved.resolve(&engine, &context).unwrap();
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_position_tracking() {
        let template = Template::new("Line 1\n{{ $a }}\nLine 3").unwrap();

        // Find the expression part
        let expr_part = template
            .parts()
            .iter()
            .find(|p| matches!(p, TemplatePart::Expression { .. }));

        assert!(expr_part.is_some());
        if let Some(TemplatePart::Expression { position, .. }) = expr_part {
            assert_eq!(position.line, 2);
            assert_eq!(position.column, 1);
        }
    }

    #[test]
    fn test_whitespace_control_left() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("World".to_string()));

        // {{- strips whitespace to the left
        let template = Template::new("Hello   {{- $input }}!").unwrap();
        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "HelloWorld!");
    }

    #[test]
    fn test_whitespace_control_right() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("Hello".to_string()));

        // -}} strips whitespace to the right
        let template = Template::new("{{ $input -}}   World!").unwrap();
        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "HelloWorld!");
    }

    #[test]
    fn test_whitespace_control_both() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("X".to_string()));

        // {{- and -}} strip both sides
        let template = Template::new("A   {{- $input -}}   B").unwrap();
        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "AXB");
    }

    #[test]
    fn test_whitespace_control_multiline() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("Content".to_string()));

        let template = Template::new("<div>\n    {{- $input -}}\n</div>").unwrap();

        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "<div>Content</div>");
    }

    #[test]
    fn test_whitespace_control_html() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::String("Title".to_string()));

        // {{- strips whitespace before, -}} strips whitespace after
        let template = Template::new("<html><title>{{- $input -}}</title></html>").unwrap();

        let result = template.render(&engine, &context).unwrap();
        assert_eq!(result, "<html><title>Title</title></html>");
    }

    #[test]
    fn test_whitespace_parse_markers() {
        let template = Template::new("{{- $input -}}").unwrap();

        if let Some(TemplatePart::Expression {
            strip_left,
            strip_right,
            content,
            ..
        }) = template
            .parts()
            .iter()
            .find(|p| matches!(p, TemplatePart::Expression { .. }))
        {
            assert!(*strip_left);
            assert!(*strip_right);
            assert_eq!(content.trim(), "$input");
        } else {
            panic!("Expected expression part");
        }
    }
}
