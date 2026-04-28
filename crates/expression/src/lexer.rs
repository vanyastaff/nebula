//! Lexer for tokenizing expression strings
//!
//! This module implements a lexer that converts expression strings into tokens.

use std::borrow::Cow;

use crate::{
    ExpressionError,
    error::{ExpressionErrorExt, ExpressionResult},
    span::Span,
    token::{Token, TokenKind},
};

/// Parse two ASCII hex digits into a single byte (`\xNN` escape).
fn parse_hex_pair(d1: char, d2: char) -> ExpressionResult<u8> {
    let mut buf = [0u8; 2];
    let s = {
        d1.encode_utf8(&mut buf[0..1]);
        d2.encode_utf8(&mut buf[1..2]);
        // Both digits are guaranteed ASCII, but if a non-hex char slipped
        // through, `from_str_radix` will surface a clear error.
        std::str::from_utf8(&buf).map_err(|e| {
            ExpressionError::expression_syntax_error(format!(
                "\\x escape contains non-UTF-8 digits: {e}"
            ))
        })?
    };
    u8::from_str_radix(s, 16).map_err(|_| {
        ExpressionError::expression_syntax_error(format!(
            "Invalid hex digits in \\x escape: '{d1}{d2}'",
        ))
    })
}

/// Parse a hex code-point string (1–6 digits, no `0x` prefix) into a `char`.
fn parse_codepoint(hex: &str) -> ExpressionResult<char> {
    let cp = u32::from_str_radix(hex, 16).map_err(|_| {
        ExpressionError::expression_syntax_error(format!("Invalid hex code point: '{hex}'"))
    })?;
    char::from_u32(cp).ok_or_else(|| {
        ExpressionError::expression_syntax_error(format!(
            "Code point U+{hex} is not a valid Unicode scalar value (surrogate or out of range)",
        ))
    })
}

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
            },
            '}' if self.peek() == Some('}') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::TemplateEnd, Span::new(start, self.position))
            },

            // Single character delimiters
            '(' => {
                self.advance();
                Token::new(TokenKind::LeftParen, Span::new(start, self.position))
            },
            ')' => {
                self.advance();
                Token::new(TokenKind::RightParen, Span::new(start, self.position))
            },
            '[' => {
                self.advance();
                Token::new(TokenKind::LeftBracket, Span::new(start, self.position))
            },
            ']' => {
                self.advance();
                Token::new(TokenKind::RightBracket, Span::new(start, self.position))
            },
            '{' => {
                self.advance();
                Token::new(TokenKind::LeftBrace, Span::new(start, self.position))
            },
            '}' => {
                self.advance();
                Token::new(TokenKind::RightBrace, Span::new(start, self.position))
            },
            ',' => {
                self.advance();
                Token::new(TokenKind::Comma, Span::new(start, self.position))
            },
            '.' => {
                self.advance();
                Token::new(TokenKind::Dot, Span::new(start, self.position))
            },
            ':' => {
                self.advance();
                Token::new(TokenKind::Colon, Span::new(start, self.position))
            },
            '?' => {
                self.advance();
                Token::new(TokenKind::Question, Span::new(start, self.position))
            },

            // Operators
            '+' => {
                self.advance();
                Token::new(TokenKind::Plus, Span::new(start, self.position))
            },
            '-' => {
                self.advance();
                Token::new(TokenKind::Minus, Span::new(start, self.position))
            },
            '*' if self.peek() == Some('*') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Power, Span::new(start, self.position))
            },
            '*' => {
                self.advance();
                Token::new(TokenKind::Star, Span::new(start, self.position))
            },
            '/' => {
                self.advance();
                Token::new(TokenKind::Slash, Span::new(start, self.position))
            },
            '%' => {
                self.advance();
                Token::new(TokenKind::Percent, Span::new(start, self.position))
            },

            // Comparison operators
            '=' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Equal, Span::new(start, self.position))
            },
            '=' if self.peek() == Some('~') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::RegexMatch, Span::new(start, self.position))
            },
            '=' if self.peek() == Some('>') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Arrow, Span::new(start, self.position))
            },
            '!' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::NotEqual, Span::new(start, self.position))
            },
            '!' => {
                self.advance();
                Token::new(TokenKind::Not, Span::new(start, self.position))
            },
            '<' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::LessEqual, Span::new(start, self.position))
            },
            '<' => {
                self.advance();
                Token::new(TokenKind::LessThan, Span::new(start, self.position))
            },
            '>' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::GreaterEqual, Span::new(start, self.position))
            },
            '>' => {
                self.advance();
                Token::new(TokenKind::GreaterThan, Span::new(start, self.position))
            },

            // Logical operators
            '&' if self.peek() == Some('&') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::And, Span::new(start, self.position))
            },
            '|' if self.peek() == Some('|') => {
                self.advance();
                self.advance();
                Token::new(TokenKind::Or, Span::new(start, self.position))
            },
            '|' => {
                self.advance();
                Token::new(TokenKind::Pipe, Span::new(start, self.position))
            },

            // String literals
            '"' | '\'' => self.read_string(ch)?,

            // Variable references
            '$' => self.read_variable()?,

            // Numbers
            ch if ch.is_ascii_digit() => self.read_number()?,

            // Identifiers and keywords
            ch if ch.is_alphabetic() || ch == '_' => self.read_identifier_or_keyword()?,

            _ => {
                return Err(ExpressionError::expression_syntax_error(format!(
                    "Unexpected character '{}' at position {}",
                    ch, self.position
                )));
            },
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
                        TokenKind::String(Cow::Borrowed(&self.input[start_pos + 1..end_pos])),
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

        Err(ExpressionError::expression_syntax_error(
            "Unterminated string literal",
        ))
    }

    /// Read a string with escape sequences (requires allocation).
    ///
    /// Supported escapes:
    /// - `\n`, `\t`, `\r` — control characters
    /// - `\\`, `\"`, `\'` — literal backslash / quote
    /// - `\xNN` — exactly two hex digits, byte (`\x41` → `A`)
    /// - `\uNNNN` — exactly four hex digits, BMP code point (`é` → `é`)
    /// - `\u{...}` — 1-6 hex digits, full Unicode code point (`\u{1F642}` → `🙂`)
    ///
    /// Unknown escapes (e.g. `\q`) pass through verbatim for backward
    /// compatibility — they don't error. Malformed `\u` / `\x` sequences
    /// (wrong digit count, invalid code point, etc.) DO error.
    fn read_string_with_escapes(
        &self,
        start: usize,
        end: usize,
        span: Span,
    ) -> ExpressionResult<Token<'a>> {
        let raw = &self.input[start..end];
        let mut result = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch != '\\' {
                result.push(ch);
                continue;
            }
            let Some(escaped) = chars.next() else {
                return Err(ExpressionError::expression_syntax_error(
                    "Trailing backslash in string literal",
                ));
            };
            match escaped {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                'r' => result.push('\r'),
                '\\' => result.push('\\'),
                '"' => result.push('"'),
                '\'' => result.push('\''),
                'x' => {
                    let d1 = chars.next().ok_or_else(|| {
                        ExpressionError::expression_syntax_error(
                            "Truncated \\x escape: expected 2 hex digits",
                        )
                    })?;
                    let d2 = chars.next().ok_or_else(|| {
                        ExpressionError::expression_syntax_error(
                            "Truncated \\x escape: expected 2 hex digits",
                        )
                    })?;
                    let value = parse_hex_pair(d1, d2)?;
                    result.push(value as char);
                },
                'u' => {
                    if chars.peek() == Some(&'{') {
                        chars.next(); // consume '{'
                        let mut hex = String::with_capacity(6);
                        loop {
                            match chars.next() {
                                Some('}') => break,
                                Some(c) if c.is_ascii_hexdigit() => {
                                    if hex.len() >= 6 {
                                        return Err(ExpressionError::expression_syntax_error(
                                            "\\u{...} escape exceeds 6 hex digits",
                                        ));
                                    }
                                    hex.push(c);
                                },
                                Some(c) => {
                                    return Err(ExpressionError::expression_syntax_error(format!(
                                        "Invalid hex digit '{c}' in \\u{{...}} escape"
                                    )));
                                },
                                None => {
                                    return Err(ExpressionError::expression_syntax_error(
                                        "Unterminated \\u{...} escape: missing '}'",
                                    ));
                                },
                            }
                        }
                        if hex.is_empty() {
                            return Err(ExpressionError::expression_syntax_error(
                                "Empty \\u{} escape: expected 1-6 hex digits",
                            ));
                        }
                        result.push(parse_codepoint(&hex)?);
                    } else {
                        let mut hex = String::with_capacity(4);
                        for _ in 0..4 {
                            match chars.next() {
                                Some(c) if c.is_ascii_hexdigit() => hex.push(c),
                                Some(c) => {
                                    return Err(ExpressionError::expression_syntax_error(format!(
                                        "Invalid hex digit '{c}' in \\uNNNN escape"
                                    )));
                                },
                                None => {
                                    return Err(ExpressionError::expression_syntax_error(
                                        "Truncated \\uNNNN escape: expected 4 hex digits",
                                    ));
                                },
                            }
                        }
                        result.push(parse_codepoint(&hex)?);
                    }
                },
                other => result.push(other),
            }
        }

        Ok(Token::new(TokenKind::String(Cow::Owned(result)), span))
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
            return Err(ExpressionError::expression_syntax_error(
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
                if let Some(next) = self.peek()
                    && next.is_ascii_digit()
                {
                    is_float = true;
                    self.advance(); // consume '.'
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
                .map_err(|_| ExpressionError::expression_syntax_error("Invalid float literal"))
        } else {
            num_str
                .parse::<i64>()
                .map(|i| Token::new(TokenKind::Integer(i), span))
                .map_err(|_| ExpressionError::expression_syntax_error("Invalid integer literal"))
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

    #[expect(
        clippy::approx_constant,
        reason = "3.14 is intentional test data, not an approximation of π"
    )]
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
                &TokenKind::String(Cow::Borrowed("hello")),
                &TokenKind::String(Cow::Borrowed("world")),
                &TokenKind::Eof
            ]
        );
    }

    #[test]
    fn test_string_escapes() {
        let mut lexer = Lexer::new(r#""hello\nworld" 'test\'quote'"#);
        let tokens = lexer.tokenize().unwrap();
        match &tokens[0].kind {
            TokenKind::String(s) => assert_eq!(s.as_ref(), "hello\nworld"),
            _ => panic!("Expected string token"),
        }
        match &tokens[1].kind {
            TokenKind::String(s) => assert_eq!(s.as_ref(), "test'quote"),
            _ => panic!("Expected string token"),
        }
    }

    /// Lex a single string literal and return the decoded contents.
    /// Panics on lex error or non-string token — caller-friendly helper
    /// for the escape-sequence tests below.
    fn lex_string(src: &str) -> String {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize().expect("expected successful lex");
        match &tokens[0].kind {
            TokenKind::String(s) => s.as_ref().to_owned(),
            other => panic!("expected string token, got {other:?}"),
        }
    }

    fn lex_string_err(src: &str) -> String {
        let mut lexer = Lexer::new(src);
        lexer
            .tokenize()
            .expect_err("expected lex failure")
            .to_string()
    }

    #[test]
    fn lexer_parses_hex_byte_escape() {
        // `\x41` → 'A' (single ASCII byte)
        assert_eq!(lex_string(r#""\x41\x42\x43""#), "ABC");
    }

    #[test]
    fn lexer_parses_unicode_codepoint_escape_brace_form() {
        // `\u{1F642}` → 🙂 (slightly smiling face, U+1F642).
        assert_eq!(lex_string(r#""\u{1F642}""#), "🙂");
        // 1-digit form is also valid.
        assert_eq!(lex_string(r#""\u{41}""#), "A");
        // 6-digit max.
        assert_eq!(lex_string(r#""\u{10FFFF}""#), "\u{10FFFF}");
    }

    #[test]
    fn lexer_parses_unicode_bmp_escape_4_digit_form() {
        // `é` → 'é' (Latin small e with acute, U+00E9, 4-digit BMP form)
        assert_eq!(lex_string(r#""é""#), "é");
    }

    #[test]
    fn lexer_rejects_truncated_hex_escape() {
        let err = lex_string_err(r#""\x4""#);
        assert!(
            err.contains("Truncated") || err.contains("\\x"),
            "got: {err}"
        );
    }

    #[test]
    fn lexer_rejects_non_hex_in_unicode_escape() {
        let err = lex_string_err(r#""\u{ZZ}""#);
        assert!(err.contains("Invalid hex"), "got: {err}");
    }

    #[test]
    fn lexer_rejects_unterminated_braced_unicode_escape() {
        let err = lex_string_err(r#""\u{1F642""#);
        assert!(
            err.contains("Unterminated") || err.contains("\\u"),
            "got: {err}"
        );
    }

    #[test]
    fn lexer_rejects_surrogate_codepoint() {
        // U+D800 is a low surrogate, not a valid Unicode scalar value.
        let err = lex_string_err(r#""\u{D800}""#);
        assert!(err.contains("not a valid"), "got: {err}");
    }

    #[test]
    fn lexer_rejects_oversized_braced_unicode_escape() {
        // 7 hex digits — moves past Unicode's 6-digit max.
        let err = lex_string_err(r#""\u{1234567}""#);
        assert!(err.contains("exceeds 6"), "got: {err}");
    }

    #[test]
    fn lexer_unknown_escape_passes_through() {
        // Backward compat: `\q` is unknown but doesn't error — the `q`
        // is preserved verbatim. Adding strict mode is a separate task.
        assert_eq!(lex_string(r#""\q""#), "q");
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
                &TokenKind::String(Cow::Borrowed("active")),
                &TokenKind::Eof
            ]
        );
    }

    #[expect(
        clippy::approx_constant,
        reason = "3.14 is a representative float literal in lexer test, not an approximation of π"
    )]
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
    fn escaped_string_produces_correct_value() {
        let input = r#""hello\nworld""#;
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 2); // String + Eof
        match &tokens[0].kind {
            TokenKind::String(s) => assert_eq!(s.as_ref(), "hello\nworld"),
            other => panic!("expected String, got {other:?}"),
        }
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
