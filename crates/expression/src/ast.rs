//! Abstract Syntax Tree (AST) node types
//!
//! This module defines the AST structure for parsed expressions.

use std::sync::Arc;

use crate::value::RuntimeValue;

/// An expression node in the AST
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // Literals
    /// Literal value, already converted to its runtime representation.
    ///
    /// Compilation converts the token once, so evaluation of a literal is a
    /// borrow rather than a JSON-to-runtime conversion per hit.
    Literal(RuntimeValue),

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
    /// Property access (`object.property`).
    ///
    /// `optional` is set by the `?.` operator: a missing property yields
    /// `Undefined` instead of erroring, regardless of the missing-lookup policy.
    PropertyAccess {
        object: Box<Expr>,
        property: Arc<str>,
        optional: bool,
    },

    /// Index access (`array[index]`).
    ///
    /// `optional` is set by `?.[`-style chaining; see [`Expr::PropertyAccess`].
    IndexAccess {
        object: Box<Expr>,
        index: Box<Expr>,
        optional: bool,
    },

    /// Method call (`object.method(args...)`).
    ///
    /// Methods dispatch on the receiver's runtime type. `optional` is set by
    /// `?.`: a missing receiver or unknown method yields `Undefined` instead of
    /// erroring.
    MethodCall {
        object: Box<Expr>,
        method: Arc<str>,
        args: Vec<Expr>,
        optional: bool,
    },

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
    /// Lambda expression (`param => body`, or `(left, right) => body`).
    Lambda {
        params: Box<[Arc<str>]>,
        body: Box<Expr>,
    },

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

    /// Nullish coalescing (`left ?? right`).
    ///
    /// Returns `left` unless it is `Null` or `Undefined`, in which case it
    /// evaluates `right`. Unlike `||` it does not treat `false`/`0`/`""` as
    /// missing.
    Coalesce,
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
            BinaryOp::Coalesce => "??",
        }
    }
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
