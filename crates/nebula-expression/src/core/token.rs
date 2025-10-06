//! Token types for the expression lexer
//!
//! This module defines all tokens that can appear in an expression.

/// A token in the expression language
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    /// Integer literal (e.g., 42, -10)
    Integer(i64),
    /// Float literal (e.g., 3.14, -2.5)
    Float(f64),
    /// String literal (e.g., "hello", 'world')
    String(String),
    /// Boolean literal (true, false)
    Boolean(bool),
    /// Null literal
    Null,

    // Identifiers and variables
    /// Identifier (e.g., functionName, variableName)
    Identifier(String),
    /// Variable reference starting with $ (e.g., $node, $execution)
    Variable(String),

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

impl Token {
    /// Check if this token is a literal value
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Token::Integer(_)
                | Token::Float(_)
                | Token::String(_)
                | Token::Boolean(_)
                | Token::Null
        )
    }

    /// Check if this token is an operator
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::Percent
                | Token::Power
                | Token::Equal
                | Token::NotEqual
                | Token::LessThan
                | Token::GreaterThan
                | Token::LessEqual
                | Token::GreaterEqual
                | Token::RegexMatch
                | Token::And
                | Token::Or
                | Token::Not
                | Token::Pipe
        )
    }

    /// Check if this token is a binary operator
    pub fn is_binary_operator(&self) -> bool {
        matches!(
            self,
            Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::Percent
                | Token::Power
                | Token::Equal
                | Token::NotEqual
                | Token::LessThan
                | Token::GreaterThan
                | Token::LessEqual
                | Token::GreaterEqual
                | Token::RegexMatch
                | Token::And
                | Token::Or // Pipe is not a binary operator, it's used for pipeline expressions
        )
    }

    /// Get the precedence of this operator (higher number = higher precedence)
    pub fn precedence(&self) -> u8 {
        match self {
            Token::Or => 1,
            Token::And => 2,
            Token::Equal | Token::NotEqual => 3,
            Token::LessThan
            | Token::GreaterThan
            | Token::LessEqual
            | Token::GreaterEqual
            | Token::RegexMatch => 4,
            Token::Plus | Token::Minus => 5,
            Token::Star | Token::Slash | Token::Percent => 6,
            Token::Power => 7,
            // Pipe is not a binary operator, handled separately in parse_pipeline
            _ => 0,
        }
    }

    /// Check if this operator is right-associative
    pub fn is_right_associative(&self) -> bool {
        matches!(self, Token::Power)
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Integer(n) => write!(f, "{}", n),
            Token::Float(n) => write!(f, "{}", n),
            Token::String(s) => write!(f, "\"{}\"", s),
            Token::Boolean(b) => write!(f, "{}", b),
            Token::Null => write!(f, "null"),
            Token::Identifier(s) => write!(f, "{}", s),
            Token::Variable(s) => write!(f, "${}", s),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Percent => write!(f, "%"),
            Token::Power => write!(f, "**"),
            Token::Equal => write!(f, "=="),
            Token::NotEqual => write!(f, "!="),
            Token::LessThan => write!(f, "<"),
            Token::GreaterThan => write!(f, ">"),
            Token::LessEqual => write!(f, "<="),
            Token::GreaterEqual => write!(f, ">="),
            Token::RegexMatch => write!(f, "=~"),
            Token::And => write!(f, "&&"),
            Token::Or => write!(f, "||"),
            Token::Not => write!(f, "!"),
            Token::Pipe => write!(f, "|"),
            Token::LeftParen => write!(f, "("),
            Token::RightParen => write!(f, ")"),
            Token::LeftBracket => write!(f, "["),
            Token::RightBracket => write!(f, "]"),
            Token::LeftBrace => write!(f, "{{"),
            Token::RightBrace => write!(f, "}}"),
            Token::Dot => write!(f, "."),
            Token::Comma => write!(f, ","),
            Token::Colon => write!(f, ":"),
            Token::Question => write!(f, "?"),
            Token::Arrow => write!(f, "=>"),
            Token::If => write!(f, "if"),
            Token::Then => write!(f, "then"),
            Token::Else => write!(f, "else"),
            Token::TemplateStart => write!(f, "{{{{"),
            Token::TemplateEnd => write!(f, "}}}}"),
            Token::Eof => write!(f, "EOF"),
        }
    }
}
