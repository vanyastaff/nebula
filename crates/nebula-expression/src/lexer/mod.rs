//! Lexer for tokenizing expression strings
//!
//! This module implements a lexer that converts expression strings into tokens.

use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::core::span::Span;
use crate::core::token::{Token, TokenKind};
use nebula_error::NebulaError;

/// Lexer for tokenizing expression strings
pub struct Lexer<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer from an input string
    pub fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    /// Tokenize the entire input string
    pub fn tokenize(&mut self) -> ExpressionResult<Vec<Token<'a>>> {
        // Estimate: typical expressions have ~1 token per 5 chars
        let estimated_tokens = (self.input.len() / 5).max(8);
        let mut tokens = Vec::with_capacity(estimated_tokens);

        loop {
            let token = self.next_token()?;
            if token.kind == TokenKind::Eof {
                tokens.push(token);
                break;
            }
            tokens.push(token);
        }

        Ok(tokens)
    }

    /// Get the next token from the input
    pub fn next_token(&mut self) -> ExpressionResult<Token<'a>> {
        self.skip_whitespace();

        let start = self.position;

        let Some(ch) = self.current_char() else {
            return Ok(Token::new(
                TokenKind::Eof,
                Span::new(self.position, self.position),
            ));
        };

        let token = match ch {
            // Template delimiters
            '{' if self.peek() == Some('{') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::TemplateStart, Span::new(start, self.position))
            }
            '}' if self.peek() == Some('}') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::TemplateEnd, Span::new(start, self.position))
            }

            // Single character delimiters
            '(' => {
                self.advance();
                Token::new(TokenKind::LeftParen, Span::new(start, self.position))
            }
            ')' => {
                self.advance();
                Token::new(TokenKind::RightParen, Span::new(start, self.position))
            }
            '[' => {
                self.advance();
                Token::new(TokenKind::LeftBracket, Span::new(start, self.position))
            }
            ']' => {
                self.advance();
                Token::new(TokenKind::RightBracket, Span::new(start, self.position))
            }
            '{' => {
                self.advance();
                Token::new(TokenKind::LeftBrace, Span::new(start, self.position))
            }
            '}' => {
                self.advance();
                Token::new(TokenKind::RightBrace, Span::new(start, self.position))
            }
            ',' => {
                self.advance();
                Token::new(TokenKind::Comma, Span::new(start, self.position))
            }
            '.' => {
                self.advance();
                Token::new(TokenKind::Dot, Span::new(start, self.position))
            }
            ':' => {
                self.advance();
                Token::new(TokenKind::Colon, Span::new(start, self.position))
            }
            '?' => {
                self.advance();
                Token::new(TokenKind::Question, Span::new(start, self.position))
            }

            // Operators
            '+' => {
                self.advance();
                Token::new(TokenKind::Plus, Span::new(start, self.position))
            }
            '-' => {
                self.advance();
                Token::new(TokenKind::Minus, Span::new(start, self.position))
            }
            '*' if self.peek() == Some('*') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Power, Span::new(start, self.position))
            }
            '*' => {
                self.advance();
                Token::new(TokenKind::Star, Span::new(start, self.position))
            }
            '/' => {
                self.advance();
                Token::new(TokenKind::Slash, Span::new(start, self.position))
            }
            '%' => {
                self.advance();
                Token::new(TokenKind::Percent, Span::new(start, self.position))
            }

            // Comparison operators
            '=' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Equal, Span::new(start, self.position))
            }
            '=' if self.peek() == Some('~') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::RegexMatch, Span::new(start, self.position))
            }
            '=' if self.peek() == Some('>') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Arrow, Span::new(start, self.position))
            }
            '!' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::NotEqual, Span::new(start, self.position))
            }
            '!' => {
                self.advance();
                Token::new(TokenKind::Not, Span::new(start, self.position))
            }
            '<' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::LessEqual, Span::new(start, self.position))
            }
            '<' => {
                self.advance();
                Token::new(TokenKind::LessThan, Span::new(start, self.position))
            }
            '>' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::GreaterEqual, Span::new(start, self.position))
            }
            '>' => {
                self.advance();
                Token::new(TokenKind::GreaterThan, Span::new(start, self.position))
            }

            // Logical operators
            '&' if self.peek() == Some('&') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::And, Span::new(start, self.position))
            }
            '|' if self.peek() == Some('|') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Or, Span::new(start, self.position))
            }
            '|' => {
                self.advance();
                Token::new(TokenKind::Pipe, Span::new(start, self.position))
            }

            // String literals
            '"' | '\'' => self.read_string(ch)?,

            // Variable references
            '$' => self.read_variable()?,

            // Numbers
            ch if ch.is_ascii_digit() => self.read_number()?,

            // Identifiers and keywords
            ch if ch.is_alphabetic() || ch == '_' => self.read_identifier_or_keyword()?,

            _ => {
                return Err(NebulaError::expression_syntax_error(format!(
                    "Unexpected character '{}' at position {}",
                    ch, self.position
                )));
            }
        };

        Ok(token)
    }

    /// Get the current character at position
    fn current_char(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }

    /// Peek at the next character without advancing
    fn peek(&self) -> Option<char> {
        let current = self.current_char()?;
        let next_pos = self.position + current.len_utf8();
        self.input[next_pos..].chars().next()
    }

    /// Advance position by the current character's UTF-8 byte length
    fn advance(&mut self) {
        if let Some(ch) = self.current_char() {
            self.position += ch.len_utf8();
        }
    }

    /// Skip whitespace characters
    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char() {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Read a string literal
    fn read_string(&mut self, quote: char) -> ExpressionResult<Token<'a>> {
        let start_pos = self.position;
        self.advance(); // Skip opening quote

        let mut has_escapes = false;

        while let Some(ch) = self.current_char() {
            if ch == quote {
                let end_pos = self.position;
                self.advance(); // Skip closing quote

                let span = Span::new(start_pos, self.position);

                // If no escapes, we can return a zero-copy slice
                if !has_escapes {
                    return Ok(Token::new(
                        TokenKind::String(&self.input[start_pos + 1..end_pos]),
                        span,
                    ));
                }

                // Otherwise, we need to process escapes
                return self.read_string_with_escapes(start_pos + 1, end_pos, span);
            } else if ch == '\\' {
                has_escapes = true;
                self.advance();
                if self.current_char().is_some() {
                    self.advance();
                }
            } else {
                self.advance();
            }
        }

        Err(NebulaError::expression_syntax_error(
            "Unterminated string literal",
        ))
    }

    /// Read a string with escape sequences (requires allocation)
    fn read_string_with_escapes(
        &self,
        start: usize,
        end: usize,
        span: Span,
    ) -> ExpressionResult<Token<'a>> {
        let raw = &self.input[start..end];
        let mut result = String::with_capacity(raw.len());
        let mut chars = raw.chars();

        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    let escaped_char = match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
                        _ => escaped,
                    };
                    result.push(escaped_char);
                }
            } else {
                result.push(ch);
            }
        }

        // We need to leak the string to get a 'a lifetime
        // This is necessary because we can't return a reference to a local String
        let leaked = Box::leak(result.into_boxed_str());
        Ok(Token::new(TokenKind::String(leaked), span))
    }

    /// Read a variable reference
    fn read_variable(&mut self) -> ExpressionResult<Token<'a>> {
        let token_start = self.position;
        self.advance(); // Skip $
        let start_pos = self.position;

        while let Some(ch) = self.current_char() {
            if ch.is_alphanumeric() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }

        let end_pos = self.position;

        if start_pos == end_pos {
            return Err(NebulaError::expression_syntax_error(
                "Expected variable name after $",
            ));
        }

        Ok(Token::new(
            TokenKind::Variable(&self.input[start_pos..end_pos]),
            Span::new(token_start, self.position),
        ))
    }

    /// Read a number (integer or float)
    fn read_number(&mut self) -> ExpressionResult<Token<'a>> {
        let start_pos = self.position;
        let mut is_float = false;

        while let Some(ch) = self.current_char() {
            if ch.is_ascii_digit() {
                self.advance();
            } else if ch == '.' && !is_float {
                // Check if next char is a digit
                if let Some(next) = self.peek() {
                    if next.is_ascii_digit() {
                        is_float = true;
                        self.advance(); // consume '.'
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let end_pos = self.position;
        let num_str = &self.input[start_pos..end_pos];
        let span = Span::new(start_pos, end_pos);

        if is_float {
            num_str
                .parse::<f64>()
                .map(|f| Token::new(TokenKind::Float(f), span))
                .map_err(|_| NebulaError::expression_syntax_error("Invalid float literal"))
        } else {
            num_str
                .parse::<i64>()
                .map(|i| Token::new(TokenKind::Integer(i), span))
                .map_err(|_| NebulaError::expression_syntax_error("Invalid integer literal"))
        }
    }

    /// Read an identifier or keyword
    fn read_identifier_or_keyword(&mut self) -> ExpressionResult<Token<'a>> {
        let start_pos = self.position;

        while let Some(ch) = self.current_char() {
            if ch.is_alphanumeric() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }

        let end_pos = self.position;
        let name = &self.input[start_pos..end_pos];
        let span = Span::new(start_pos, end_pos);

        // Check for keywords
        let token_kind = match name {
            "true" => TokenKind::Boolean(true),
            "false" => TokenKind::Boolean(false),
            "null" => TokenKind::Null,
            "if" => TokenKind::If,
            "then" => TokenKind::Then,
            "else" => TokenKind::Else,
            _ => TokenKind::Identifier(name),
        };

        Ok(Token::new(token_kind, span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_tokens() {
        let mut lexer = Lexer::new("+ - * / %");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Plus,
                &TokenKind::Minus,
                &TokenKind::Star,
                &TokenKind::Slash,
                &TokenKind::Percent,
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_numbers() {
        let mut lexer = Lexer::new("42 3.14 -10");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Integer(42),
                &TokenKind::Float(3.14),
                &TokenKind::Minus,
                &TokenKind::Integer(10),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_strings() {
        let mut lexer = Lexer::new(r#""hello" 'world'"#);
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::String("hello"),
                &TokenKind::String("world"),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_string_escapes() {
        let mut lexer = Lexer::new(r#""hello\nworld" 'test\'quote'"#);
        let tokens = lexer.tokenize().unwrap();
        match &tokens[0].kind {
            TokenKind::String(s) => assert_eq!(*s, "hello\nworld"),
            _ => panic!("Expected string token"),
        }
        match &tokens[1].kind {
            TokenKind::String(s) => assert_eq!(*s, "test'quote"),
            _ => panic!("Expected string token"),
        }
    }

    #[test]
    fn test_variables() {
        let mut lexer = Lexer::new("$node $execution");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Variable("node"),
                &TokenKind::Variable("execution"),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_operators() {
        let mut lexer = Lexer::new("== != <= >= && || =~");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Equal,
                &TokenKind::NotEqual,
                &TokenKind::LessEqual,
                &TokenKind::GreaterEqual,
                &TokenKind::And,
                &TokenKind::Or,
                &TokenKind::RegexMatch,
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_keywords() {
        let mut lexer = Lexer::new("if true then false else null");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::If,
                &TokenKind::Boolean(true),
                &TokenKind::Then,
                &TokenKind::Boolean(false),
                &TokenKind::Else,
                &TokenKind::Null,
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_identifiers() {
        let mut lexer = Lexer::new("foo bar_baz test123");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Identifier("foo"),
                &TokenKind::Identifier("bar_baz"),
                &TokenKind::Identifier("test123"),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_template_delimiters() {
        let mut lexer = Lexer::new("{{ $var }}");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::TemplateStart,
                &TokenKind::Variable("var"),
                &TokenKind::TemplateEnd,
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_complex_expression() {
        let mut lexer = Lexer::new("$node.value >= 10 && $status == 'active'");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Variable("node"),
                &TokenKind::Dot,
                &TokenKind::Identifier("value"),
                &TokenKind::GreaterEqual,
                &TokenKind::Integer(10),
                &TokenKind::And,
                &TokenKind::Variable("status"),
                &TokenKind::Equal,
                &TokenKind::String("active"),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_float_not_dot() {
        let mut lexer = Lexer::new("3.14.toString");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Float(3.14),
                &TokenKind::Dot,
                &TokenKind::Identifier("toString"),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_unterminated_string() {
        let mut lexer = Lexer::new(r#""hello"#);
        let result = lexer.tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_variable() {
        let mut lexer = Lexer::new("$ ");
        let result = lexer.tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn test_power_operator() {
        let mut lexer = Lexer::new("2 ** 3");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Integer(2),
                &TokenKind::Power,
                &TokenKind::Integer(3),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_arrow_operator() {
        let mut lexer = Lexer::new("x => x + 1");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Identifier("x"),
                &TokenKind::Arrow,
                &TokenKind::Identifier("x"),
                &TokenKind::Plus,
                &TokenKind::Integer(1),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_utf8_identifiers() {
        let mut lexer = Lexer::new("hello world");
        let tokens = lexer.tokenize().unwrap();
        let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &TokenKind::Identifier("hello"),
                &TokenKind::Identifier("world"),
                &TokenKind::Eof
            ]
        );
    }
}
