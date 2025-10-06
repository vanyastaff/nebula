//! Lexer for tokenizing expression strings
//!
//! This module implements a lexer that converts expression strings into tokens.

use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::core::token::Token;
use nebula_error::NebulaError;

/// Lexer for tokenizing expression strings
pub struct Lexer {
    input: Vec<char>,
    position: usize,
    current_char: Option<char>,
}

impl Lexer {
    /// Create a new lexer from an input string
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let current_char = chars.first().copied();
        Self {
            input: chars,
            position: 0,
            current_char,
        }
    }

    /// Tokenize the entire input string
    pub fn tokenize(&mut self) -> ExpressionResult<Vec<Token>> {
        let mut tokens = Vec::new();

        loop {
            let token = self.next_token()?;
            if token == Token::Eof {
                tokens.push(token);
                break;
            }
            tokens.push(token);
        }

        Ok(tokens)
    }

    /// Get the next token from the input
    pub fn next_token(&mut self) -> ExpressionResult<Token> {
        self.skip_whitespace();

        let Some(ch) = self.current_char else {
            return Ok(Token::Eof);
        };

        let token = match ch {
            // Template delimiters
            '{' if self.peek() == Some('{') => {
                self.advance();
                self.advance();
                Token::TemplateStart
            }
            '}' if self.peek() == Some('}') => {
                self.advance();
                self.advance();
                Token::TemplateEnd
            }

            // Single character delimiters
            '(' => {
                self.advance();
                Token::LeftParen
            }
            ')' => {
                self.advance();
                Token::RightParen
            }
            '[' => {
                self.advance();
                Token::LeftBracket
            }
            ']' => {
                self.advance();
                Token::RightBracket
            }
            '{' => {
                self.advance();
                Token::LeftBrace
            }
            '}' => {
                self.advance();
                Token::RightBrace
            }
            ',' => {
                self.advance();
                Token::Comma
            }
            '.' => {
                self.advance();
                Token::Dot
            }
            ':' => {
                self.advance();
                Token::Colon
            }
            '?' => {
                self.advance();
                Token::Question
            }

            // Operators
            '+' => {
                self.advance();
                Token::Plus
            }
            '-' => {
                self.advance();
                Token::Minus
            }
            '*' if self.peek() == Some('*') => {
                self.advance();
                self.advance();
                Token::Power
            }
            '*' => {
                self.advance();
                Token::Star
            }
            '/' => {
                self.advance();
                Token::Slash
            }
            '%' => {
                self.advance();
                Token::Percent
            }

            // Comparison operators
            '=' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::Equal
            }
            '=' if self.peek() == Some('~') => {
                self.advance();
                self.advance();
                Token::RegexMatch
            }
            '=' if self.peek() == Some('>') => {
                self.advance();
                self.advance();
                Token::Arrow
            }
            '!' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::NotEqual
            }
            '!' => {
                self.advance();
                Token::Not
            }
            '<' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::LessEqual
            }
            '<' => {
                self.advance();
                Token::LessThan
            }
            '>' if self.peek() == Some('=') => {
                self.advance();
                self.advance();
                Token::GreaterEqual
            }
            '>' => {
                self.advance();
                Token::GreaterThan
            }

            // Logical operators
            '&' if self.peek() == Some('&') => {
                self.advance();
                self.advance();
                Token::And
            }
            '|' if self.peek() == Some('|') => {
                self.advance();
                self.advance();
                Token::Or
            }
            '|' => {
                self.advance();
                Token::Pipe
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

    /// Skip whitespace characters
    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Advance to the next character
    fn advance(&mut self) {
        self.position += 1;
        self.current_char = if self.position < self.input.len() {
            Some(self.input[self.position])
        } else {
            None
        };
    }

    /// Peek at the next character without advancing
    fn peek(&self) -> Option<char> {
        if self.position + 1 < self.input.len() {
            Some(self.input[self.position + 1])
        } else {
            None
        }
    }

    /// Read a string literal
    fn read_string(&mut self, quote: char) -> ExpressionResult<Token> {
        let mut value = String::new();
        self.advance(); // Skip opening quote

        while let Some(ch) = self.current_char {
            if ch == quote {
                self.advance(); // Skip closing quote
                return Ok(Token::String(value));
            } else if ch == '\\' {
                // Handle escape sequences
                self.advance();
                if let Some(escaped) = self.current_char {
                    let escaped_char = match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
                        _ => escaped,
                    };
                    value.push(escaped_char);
                    self.advance();
                }
            } else {
                value.push(ch);
                self.advance();
            }
        }

        Err(NebulaError::expression_syntax_error(
            "Unterminated string literal",
        ))
    }

    /// Read a variable reference
    fn read_variable(&mut self) -> ExpressionResult<Token> {
        self.advance(); // Skip $
        let mut name = String::new();

        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' {
                name.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if name.is_empty() {
            return Err(NebulaError::expression_syntax_error(
                "Expected variable name after $",
            ));
        }

        Ok(Token::Variable(name))
    }

    /// Read a number (integer or float)
    fn read_number(&mut self) -> ExpressionResult<Token> {
        let mut num_str = String::new();
        let mut is_float = false;

        while let Some(ch) = self.current_char {
            if ch.is_ascii_digit() {
                num_str.push(ch);
                self.advance();
            } else if ch == '.' && !is_float && self.peek().map_or(false, |c| c.is_ascii_digit()) {
                is_float = true;
                num_str.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if is_float {
            num_str
                .parse::<f64>()
                .map(Token::Float)
                .map_err(|_| NebulaError::expression_syntax_error("Invalid float literal"))
        } else {
            num_str
                .parse::<i64>()
                .map(Token::Integer)
                .map_err(|_| NebulaError::expression_syntax_error("Invalid integer literal"))
        }
    }

    /// Read an identifier or keyword
    fn read_identifier_or_keyword(&mut self) -> ExpressionResult<Token> {
        let mut name = String::new();

        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' {
                name.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        // Check for keywords
        let token = match name.as_str() {
            "true" => Token::Boolean(true),
            "false" => Token::Boolean(false),
            "null" => Token::Null,
            "if" => Token::If,
            "then" => Token::Then,
            "else" => Token::Else,
            _ => Token::Identifier(name),
        };

        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_tokens() {
        let mut lexer = Lexer::new("+ - * / %");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Plus,
                Token::Minus,
                Token::Star,
                Token::Slash,
                Token::Percent,
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_numbers() {
        let mut lexer = Lexer::new("42 3.14 -10");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Integer(42),
                Token::Float(3.14),
                Token::Minus,
                Token::Integer(10),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_strings() {
        let mut lexer = Lexer::new(r#""hello" 'world'"#);
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::String("hello".to_string()),
                Token::String("world".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_variables() {
        let mut lexer = Lexer::new("$node $execution");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Variable("node".to_string()),
                Token::Variable("execution".to_string()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_operators() {
        let mut lexer = Lexer::new("== != <= >= && || =~");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Equal,
                Token::NotEqual,
                Token::LessEqual,
                Token::GreaterEqual,
                Token::And,
                Token::Or,
                Token::RegexMatch,
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_keywords() {
        let mut lexer = Lexer::new("if true then false else null");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::If,
                Token::Boolean(true),
                Token::Then,
                Token::Boolean(false),
                Token::Else,
                Token::Null,
                Token::Eof
            ]
        );
    }
}
