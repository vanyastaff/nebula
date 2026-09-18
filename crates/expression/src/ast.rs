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
        /// Left operand.
        left: Box<Expr>,
        /// Operator applied to both operands.
        op: BinaryOp,
        /// Right operand.
        right: Box<Expr>,
    },

    // Access operations
    /// Property access (`object.property`).
    ///
    /// `optional` is set by the `?.` operator: a missing property yields
    /// `Undefined` instead of erroring, regardless of the missing-lookup policy.
    PropertyAccess {
        /// Expression the property is read from.
        object: Box<Expr>,
        /// Static property name (safe to echo in diagnostics).
        property: Arc<str>,
        /// Set by `?.`: a nullish receiver short-circuits to `Undefined`.
        optional: bool,
    },

    /// Index access (`array[index]`).
    ///
    /// `optional` is set by `?.[`-style chaining; see [`Expr::PropertyAccess`].
    IndexAccess {
        /// Array or object being indexed.
        object: Box<Expr>,
        /// Index expression; arrays take an integer, objects a string key.
        index: Box<Expr>,
        /// Set by `?.[`: a nullish receiver short-circuits to `Undefined`.
        optional: bool,
    },

    /// Method call (`object.method(args...)`).
    ///
    /// Methods dispatch on the receiver's runtime type. `optional` is set by
    /// `?.`: a missing receiver or unknown method yields `Undefined` instead of
    /// erroring.
    MethodCall {
        /// Receiver expression, passed as the method's first argument.
        object: Box<Expr>,
        /// Method name, resolved through the alias table.
        method: Arc<str>,
        /// Remaining call arguments.
        args: Vec<Expr>,
        /// Set by `?.`: a nullish receiver short-circuits to `Undefined`.
        optional: bool,
    },

    // Function calls
    /// Function call (functionName(args...))
    FunctionCall {
        /// Registered function name.
        name: Arc<str>,
        /// Call arguments; a lambda stays unevaluated until the builtin asks.
        args: Vec<Expr>,
    },

    // Pipeline
    /// Pipeline operation (expr | function(args...))
    Pipeline {
        /// Value piped into the function as its first argument.
        value: Box<Expr>,
        /// Registered function name.
        function: Arc<str>,
        /// Remaining call arguments.
        args: Vec<Expr>,
    },

    // Conditional
    /// Conditional expression (if condition then value1 else value2)
    Conditional {
        /// Condition; only the taken branch is evaluated.
        condition: Box<Expr>,
        /// Branch evaluated when the condition is truthy.
        then_expr: Box<Expr>,
        /// Branch evaluated otherwise.
        else_expr: Box<Expr>,
    },

    // Lambda
    /// Lambda expression (`param => body`, or `(left, right) => body`).
    Lambda {
        /// Parameter names, bound positionally by the invoking builtin.
        params: Box<[Arc<str>]>,
        /// Body evaluated once per invocation.
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
    /// `+`: integer or float addition, or string concatenation.
    Add,
    /// `-`: numeric subtraction.
    Subtract,
    /// `*`: numeric multiplication.
    Multiply,
    /// `/`: floating-point division; integer operands are widened.
    Divide,
    /// `%`: remainder; integer modulo when both sides are integers.
    Modulo,
    /// `**`: exponentiation, always floating-point.
    Power,

    // Comparison
    /// `==`: structural equality with exact mixed-number comparison.
    Equal,
    /// `!=`: negation of [`Self::Equal`].
    NotEqual,
    /// `<`.
    LessThan,
    /// `>`.
    GreaterThan,
    /// `<=`.
    LessEqual,
    /// `>=`.
    GreaterEqual,
    /// `=~`: regex match of the left string against the right pattern.
    RegexMatch,

    // Logical
    /// `&&`: short-circuiting boolean conjunction.
    And,
    /// `||`: short-circuiting boolean disjunction.
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
