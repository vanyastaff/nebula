//! Token types for the expression lexer
//!
//! This module defines all tokens that can appear in an expression.

use super::span::Span;

/// A token in the expression language with position information
#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> {
    /// The token kind
    pub kind: TokenKind<'a>,
    /// Source span for this token
    pub span: Span,
}

impl<'a> Token<'a> {
    /// Create a new token with span
    pub fn new(kind: TokenKind<'a>, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The kind of token
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    // Literals
    /// Integer literal (e.g., 42, -10)
    Integer(i64),
    /// Float literal (e.g., 3.14, -2.5)
    Float(f64),
    /// String literal (e.g., "hello", 'world')
    String(&'a str),
    /// Boolean literal (true, false)
    Boolean(bool),
    /// Null literal
    Null,

    // Identifiers and variables
    /// Identifier (e.g., functionName, variableName)
    Identifier(&'a str),
    /// Variable reference starting with $ (e.g., $node, $execution)
    Variable(&'a str),

    // Operators - Arithmetic
    /// Addition operator (+)
    Plus,
    /// Subtraction operator (-)
    Minus,
    /// Multiplication operator (*)
    Star,
    /// Division operator (/)
    Slash,
    /// Modulo operator (%)
    Percent,
    /// Exponentiation operator (**)
    Power,

    // Operators - Comparison
    /// Equal operator (==)
    Equal,
    /// Not equal operator (!=)
    NotEqual,
    /// Less than operator (<)
    LessThan,
    /// Greater than operator (>)
    GreaterThan,
    /// Less than or equal operator (<=)
    LessEqual,
    /// Greater than or equal operator (>=)
    GreaterEqual,
    /// Regex match operator (=~)
    RegexMatch,

    // Operators - Logical
    /// Logical AND operator (&&)
    And,
    /// Logical OR operator (||)
    Or,
    /// Logical NOT operator (!)
    Not,

    // Pipeline
    /// Pipeline operator (|)
    Pipe,

    // Delimiters
    /// Left parenthesis (()
    LeftParen,
    /// Right parenthesis ())
    RightParen,
    /// Left bracket ([)
    LeftBracket,
    /// Right bracket (])
    RightBracket,
    /// Left brace ({)
    LeftBrace,
    /// Right brace (})
    RightBrace,

    // Punctuation
    /// Dot operator (.)
    Dot,
    /// Comma separator (,)
    Comma,
    /// Colon (:)
    Colon,
    /// Question mark (?)
    Question,
    /// Arrow for lambdas (=>)
    Arrow,

    // Keywords
    /// if keyword
    If,
    /// then keyword
    Then,
    /// else keyword
    Else,

    // Template delimiters
    /// Template start ({{)
    TemplateStart,
    /// Template end (}})
    TemplateEnd,

    // Special
    /// End of input
    Eof,
}

impl<'a> TokenKind<'a> {
    /// Check if this token is a literal value
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            TokenKind::Integer(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::Boolean(_)
                | TokenKind::Null
        )
    }

    /// Check if this token is an operator
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::Power
                | TokenKind::Equal
                | TokenKind::NotEqual
                | TokenKind::LessThan
                | TokenKind::GreaterThan
                | TokenKind::LessEqual
                | TokenKind::GreaterEqual
                | TokenKind::RegexMatch
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Not
                | TokenKind::Pipe
        )
    }

    /// Check if this token is a binary operator
    pub fn is_binary_operator(&self) -> bool {
        matches!(
            self,
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::Power
                | TokenKind::Equal
                | TokenKind::NotEqual
                | TokenKind::LessThan
                | TokenKind::GreaterThan
                | TokenKind::LessEqual
                | TokenKind::GreaterEqual
                | TokenKind::RegexMatch
                | TokenKind::And
                | TokenKind::Or // Pipe is not a binary operator, it's used for pipeline expressions
        )
    }

    /// Get the precedence of this operator (higher number = higher precedence)
    pub fn precedence(&self) -> u8 {
        match self {
            TokenKind::Or => 1,
            TokenKind::And => 2,
            TokenKind::Equal | TokenKind::NotEqual => 3,
            TokenKind::LessThan
            | TokenKind::GreaterThan
            | TokenKind::LessEqual
            | TokenKind::GreaterEqual
            | TokenKind::RegexMatch => 4,
            TokenKind::Plus | TokenKind::Minus => 5,
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => 6,
            TokenKind::Power => 7,
            // Pipe is not a binary operator, handled separately in parse_pipeline
            _ => 0,
        }
    }

    /// Check if this operator is right-associative
    pub fn is_right_associative(&self) -> bool {
        matches!(self, TokenKind::Power)
    }
}

impl<'a> std::fmt::Display for Token<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl<'a> std::fmt::Display for TokenKind<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenKind::Integer(n) => write!(f, "{}", n),
            TokenKind::Float(n) => write!(f, "{}", n),
            TokenKind::String(s) => write!(f, "\"{}\"", s),
            TokenKind::Boolean(b) => write!(f, "{}", b),
            TokenKind::Null => write!(f, "null"),
            TokenKind::Identifier(s) => write!(f, "{}", s),
            TokenKind::Variable(s) => write!(f, "${}", s),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::Power => write!(f, "**"),
            TokenKind::Equal => write!(f, "=="),
            TokenKind::NotEqual => write!(f, "!="),
            TokenKind::LessThan => write!(f, "<"),
            TokenKind::GreaterThan => write!(f, ">"),
            TokenKind::LessEqual => write!(f, "<="),
            TokenKind::GreaterEqual => write!(f, ">="),
            TokenKind::RegexMatch => write!(f, "=~"),
            TokenKind::And => write!(f, "&&"),
            TokenKind::Or => write!(f, "||"),
            TokenKind::Not => write!(f, "!"),
            TokenKind::Pipe => write!(f, "|"),
            TokenKind::LeftParen => write!(f, "("),
            TokenKind::RightParen => write!(f, ")"),
            TokenKind::LeftBracket => write!(f, "["),
            TokenKind::RightBracket => write!(f, "]"),
            TokenKind::LeftBrace => write!(f, "{{"),
            TokenKind::RightBrace => write!(f, "}}"),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Question => write!(f, "?"),
            TokenKind::Arrow => write!(f, "=>"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Then => write!(f, "then"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::TemplateStart => write!(f, "{{{{"),
            TokenKind::TemplateEnd => write!(f, "}}}}"),
            TokenKind::Eof => write!(f, "EOF"),
        }
    }
}
