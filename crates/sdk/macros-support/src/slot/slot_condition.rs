//! Syntax-only validation; admitted Rule budgets remain validator-owned.

use syn::{Expr, ExprCall, Lit, Result, UnOp};

// Bound expansion work independently of the runtime checked Rule constructors.
const MAX_SYNTAX_DEPTH: usize = 64;
const MAX_SYNTAX_NODES: usize = 1_024;

/// Scope of syntax references, independent of runtime schema admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionScope {
    /// Slots refer to their separate associated DTO through root or named references.
    AssociatedData,
    /// Schema conditions may additionally refer to fields of their local record.
    LocalRecord,
}

/// Check condition DSL syntax without resolving a reference or evaluating data.
///
/// A local record field still needs expansion-time identifier lookup and canonical
/// inbound key mapping. Named/root targets and protected domains need the owning
/// schema's admission, and runtime checked Condition construction retains budgets.
pub fn validate_condition(expression: &Expr, scope: ConditionScope) -> Result<()> {
    condition(expression, scope, 1, &mut 0)
}

fn condition(
    expression: &Expr,
    scope: ConditionScope,
    depth: usize,
    nodes: &mut usize,
) -> Result<()> {
    *nodes += 1;
    if depth > MAX_SYNTAX_DEPTH || *nodes > MAX_SYNTAX_NODES {
        return Err(syn::Error::new_spanned(
            expression,
            "slot condition exceeds syntax depth/node budget",
        ));
    }
    let (name, call) = function(expression)?;
    match name.as_str() {
        "all" | "any" => {
            if call.args.is_empty() {
                return Err(syn::Error::new_spanned(
                    call,
                    "all/any requires at least one condition",
                ));
            }
            for child in &call.args {
                condition(child, scope, depth + 1, nodes)?;
            }
        },
        "not" => {
            arity(call, 1)?;
            condition(&call.args[0], scope, depth + 1, nodes)?;
        },
        "condition" => {
            arity(call, 1)?;
            let Expr::Path(path) = &call.args[0] else {
                return Err(syn::Error::new_spanned(
                    &call.args[0],
                    "condition requires an identifier",
                ));
            };
            if path.qself.is_some() || path.path.get_ident().is_none() {
                return Err(syn::Error::new_spanned(
                    path,
                    "condition requires an unqualified identifier",
                ));
            }
        },
        "eq" | "ne" | "gt" | "gte" | "lt" | "lte" => {
            arity(call, 2)?;
            reference(&call.args[0], scope)?;
            literal(
                &call.args[1],
                matches!(name.as_str(), "gt" | "gte" | "lt" | "lte"),
            )?;
        },
        "is_true" | "is_false" => {
            arity(call, 1)?;
            reference(&call.args[0], scope)?;
        },
        "one_of" => {
            arity(call, 2)?;
            reference(&call.args[0], scope)?;
            let Expr::Array(array) = &call.args[1] else {
                return Err(syn::Error::new_spanned(
                    &call.args[1],
                    "one_of requires a nonempty literal array",
                ));
            };
            if array.elems.is_empty() || array.elems.len() > MAX_SYNTAX_NODES {
                return Err(syn::Error::new_spanned(
                    array,
                    "one_of requires a bounded nonempty literal array",
                ));
            }
            for value in &array.elems {
                literal(value, false)?;
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &call.func,
                "unknown slot condition; expected eq/ne/gt/gte/lt/lte/one_of/is_true/is_false/all/any/not/condition",
            ));
        },
    }
    Ok(())
}

fn function(expression: &Expr) -> Result<(String, &ExprCall)> {
    let Expr::Call(call) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected a checked condition DSL call, not a Rust expression",
        ));
    };
    let Expr::Path(path) = &*call.func else {
        return Err(syn::Error::new_spanned(
            &call.func,
            "condition function must be an unqualified identifier",
        ));
    };
    let Some(name) = path.path.get_ident() else {
        return Err(syn::Error::new_spanned(
            path,
            "condition function must be an unqualified identifier",
        ));
    };
    if path.qself.is_some() {
        return Err(syn::Error::new_spanned(
            path,
            "qualified condition functions are unsupported",
        ));
    }
    Ok((name.to_string(), call))
}

fn arity(call: &ExprCall, expected: usize) -> Result<()> {
    if call.args.len() == expected {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            call,
            format!("condition requires exactly {expected} operand(s)"),
        ))
    }
}

fn reference(expression: &Expr, scope: ConditionScope) -> Result<()> {
    let (name, call) = function(expression)?;
    if name == "field" && scope == ConditionScope::LocalRecord {
        arity(call, 1)?;
        let Expr::Path(path) = &call.args[0] else {
            return Err(syn::Error::new_spanned(
                &call.args[0],
                "field requires a local Rust field identifier",
            ));
        };
        if path.qself.is_none() && path.path.get_ident().is_some() {
            return Ok(());
        }
        return Err(syn::Error::new_spanned(
            path,
            "field requires an unqualified local field identifier",
        ));
    }
    if name != "root" {
        return Err(syn::Error::new_spanned(
            expression,
            "slot conditions over associated DTOs require root(\"/path\") or a named condition; receiver field references are unsupported",
        ));
    }
    arity(call, 1)?;
    let Expr::Lit(value) = &call.args[0] else {
        return Err(syn::Error::new_spanned(
            &call.args[0],
            "root requires a literal RFC6901 pointer",
        ));
    };
    let Lit::Str(pointer) = &value.lit else {
        return Err(syn::Error::new_spanned(
            value,
            "root requires a literal RFC6901 pointer",
        ));
    };
    let text = pointer.value();
    let mut bytes = text.bytes();
    if !text.is_empty() && !text.starts_with('/') {
        return Err(syn::Error::new_spanned(
            pointer,
            "root pointer must be empty or start with /",
        ));
    }
    while let Some(byte) = bytes.next() {
        if byte == b'~' && !matches!(bytes.next(), Some(b'0' | b'1')) {
            return Err(syn::Error::new_spanned(
                pointer,
                "RFC6901 escape must be ~0 or ~1",
            ));
        }
    }
    if text.split('/').any(|part| part == "*") {
        return Err(syn::Error::new_spanned(
            pointer,
            "wildcard slot selectors are unsupported",
        ));
    }
    Ok(())
}

fn literal(expression: &Expr, numeric_only: bool) -> Result<()> {
    match expression {
        Expr::Lit(value) => match &value.lit {
            Lit::Int(number) if number.suffix().is_empty() => {
                let _: u64 = number.base10_parse()?;
                Ok(())
            },
            Lit::Float(number) if number.suffix().is_empty() => {
                let parsed: f64 = number.base10_parse()?;
                if parsed.is_finite() {
                    Ok(())
                } else {
                    Err(syn::Error::new_spanned(
                        number,
                        "condition number must be finite",
                    ))
                }
            },
            Lit::Str(_) | Lit::Bool(_) if !numeric_only => Ok(()),
            _ => Err(syn::Error::new_spanned(
                value,
                "condition requires an unsuffixed JSON scalar literal",
            )),
        },
        Expr::Unary(unary)
            if matches!(unary.op, UnOp::Neg(_)) && matches!(&*unary.expr, Expr::Lit(_)) =>
        {
            if let Expr::Lit(value) = &*unary.expr
                && let Lit::Int(number) = &value.lit
                && number.suffix().is_empty()
            {
                let magnitude: u64 = number.base10_parse()?;
                if magnitude > i64::MIN.unsigned_abs() {
                    return Err(syn::Error::new_spanned(
                        number,
                        "negative condition integer must fit a JSON i64",
                    ));
                }
            }
            literal(&unary.expr, true)
        },
        Expr::Path(path) if !numeric_only && path.path.is_ident("null") && path.qself.is_none() => {
            Ok(())
        },
        _ => Err(syn::Error::new_spanned(
            expression,
            "condition operands must be literal data, not Rust expressions",
        )),
    }
}
