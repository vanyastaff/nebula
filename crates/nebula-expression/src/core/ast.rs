//! Abstract Syntax Tree (AST) node types
//!
//! This module defines the AST structure for parsed expressions.

use nebula_value::Value;
use std::sync::Arc;

/// An expression node in the AST
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // Literals
    /// Literal value
    Literal(Value),

    // Variables and identifiers
    /// Variable reference (e.g., $node, $execution)
    Variable(Arc<str>),

    /// Identifier (for function names, etc.)
    Identifier(Arc<str>),

    // Unary operations
    /// Unary negation (-expr)
    Negate(Box<Expr>),

    /// Logical NOT (!expr)
    Not(Box<Expr>),

    // Binary operations
    /// Binary operation (left op right)
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },

    // Access operations
    /// Property access (object.property)
    PropertyAccess {
        object: Box<Expr>,
        property: Arc<str>,
    },

    /// Index access (array[index])
    IndexAccess { object: Box<Expr>, index: Box<Expr> },

    // Function calls
    /// Function call (functionName(args...))
    FunctionCall { name: Arc<str>, args: Vec<Expr> },

    // Pipeline
    /// Pipeline operation (expr | function(args...))
    Pipeline {
        value: Box<Expr>,
        function: Arc<str>,
        args: Vec<Expr>,
    },

    // Conditional
    /// Conditional expression (if condition then value1 else value2)
    Conditional {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },

    // Lambda
    /// Lambda expression (param => body)
    Lambda { param: Arc<str>, body: Box<Expr> },

    // Array and Object literals
    /// Array literal ([expr1, expr2, ...])
    Array(Vec<Expr>),

    /// Object literal ({key1: value1, key2: value2, ...})
    Object(Vec<(Arc<str>, Expr)>),
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,

    // Comparison
    Equal,
    NotEqual,
    LessThan,
    GreaterThan,
    LessEqual,
    GreaterEqual,
    RegexMatch,

    // Logical
    And,
    Or,
}

impl BinaryOp {
    /// Get a human-readable name for the operator
    pub fn name(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Modulo => "%",
            BinaryOp::Power => "**",
            BinaryOp::Equal => "==",
            BinaryOp::NotEqual => "!=",
            BinaryOp::LessThan => "<",
            BinaryOp::GreaterThan => ">",
            BinaryOp::LessEqual => "<=",
            BinaryOp::GreaterEqual => ">=",
            BinaryOp::RegexMatch => "=~",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl Expr {
    /// Check if this expression is a literal constant
    pub fn is_literal(&self) -> bool {
        matches!(self, Expr::Literal(_))
    }

    /// Try to extract a literal value if this is a literal expression
    pub fn as_literal(&self) -> Option<&Value> {
        match self {
            Expr::Literal(val) => Some(val),
            _ => None,
        }
    }
}
