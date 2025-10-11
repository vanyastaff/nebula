//! Parser for converting tokens into an AST
//!
//! This module implements a recursive descent parser with precedence climbing for operators.

use crate::core::ast::{BinaryOp, Expr};
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::core::span::Span;
use crate::core::token::{Token, TokenKind};
use nebula_error::NebulaError;
use nebula_value::Value;
use std::sync::Arc;

/// Maximum recursion depth for parser
const MAX_PARSER_DEPTH: usize = 256;

/// EOF token constant
const EOF_TOKEN: Token<'static> = Token {
    kind: TokenKind::Eof,
    span: Span { start: 0, end: 0 },
};

/// Parser for converting tokens into an AST
pub struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    position: usize,
}

impl<'a> Parser<'a> {
    /// Create a new parser from a list of tokens
    pub fn new(tokens: Vec<Token<'a>>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    /// Parse the tokens into an expression AST
    pub fn parse(&mut self) -> ExpressionResult<Expr> {
        self.parse_expression_with_depth(0)
    }

    /// Parse expression with depth tracking
    fn parse_expression_with_depth(&mut self, depth: usize) -> ExpressionResult<Expr> {
        if depth > MAX_PARSER_DEPTH {
            return Err(NebulaError::expression_parse_error(format!(
                "Maximum parser recursion depth ({}) exceeded",
                MAX_PARSER_DEPTH
            )));
        }
        self.parse_conditional_with_depth(depth)
    }

    /// Parse conditional with depth tracking
    fn parse_conditional_with_depth(&mut self, depth: usize) -> ExpressionResult<Expr> {
        if self.match_token(&TokenKind::If) {
            let condition = Box::new(self.parse_pipeline_with_depth(depth + 1)?);
            self.expect_token(TokenKind::Then)?;
            let then_expr = Box::new(self.parse_pipeline_with_depth(depth + 1)?);
            self.expect_token(TokenKind::Else)?;
            let else_expr = Box::new(self.parse_pipeline_with_depth(depth + 1)?);

            Ok(Expr::Conditional {
                condition,
                then_expr,
                else_expr,
            })
        } else {
            self.parse_pipeline_with_depth(depth + 1)
        }
    }

    /// Parse pipeline expression with depth tracking
    fn parse_pipeline_with_depth(&mut self, depth: usize) -> ExpressionResult<Expr> {
        let mut expr = self.parse_binary_op_with_depth(0, depth + 1)?;

        while self.current_token().kind == TokenKind::Pipe {
            self.advance();

            // Expect function name
            let function = if let TokenKind::Identifier(name) = &self.current_token().kind {
                let name = Arc::from(*name);
                self.advance();
                name
            } else {
                return Err(NebulaError::expression_parse_error(
                    "Expected function name after |",
                ));
            };

            // Parse arguments if present
            let args = if self.current_token().kind == TokenKind::LeftParen {
                self.parse_function_args_with_depth(depth + 1)?
            } else {
                Vec::new()
            };

            expr = Expr::Pipeline {
                value: Box::new(expr),
                function,
                args,
            };
        }

        Ok(expr)
    }

    /// Parse binary expression with precedence climbing and depth tracking
    fn parse_binary_op_with_depth(
        &mut self,
        min_precedence: u8,
        depth: usize,
    ) -> ExpressionResult<Expr> {
        let mut left = self.parse_unary_with_depth(depth + 1)?;

        while self.current_token().kind.is_binary_operator() {
            // Extract token info before advancing
            let precedence = self.current_token().kind.precedence();

            if precedence < min_precedence {
                break;
            }

            let is_right_assoc = self.current_token().kind.is_right_associative();
            let binary_op = match &self.current_token().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Subtract,
                TokenKind::Star => BinaryOp::Multiply,
                TokenKind::Slash => BinaryOp::Divide,
                TokenKind::Percent => BinaryOp::Modulo,
                TokenKind::Power => BinaryOp::Power,
                TokenKind::Equal => BinaryOp::Equal,
                TokenKind::NotEqual => BinaryOp::NotEqual,
                TokenKind::LessThan => BinaryOp::LessThan,
                TokenKind::GreaterThan => BinaryOp::GreaterThan,
                TokenKind::LessEqual => BinaryOp::LessEqual,
                TokenKind::GreaterEqual => BinaryOp::GreaterEqual,
                TokenKind::RegexMatch => BinaryOp::RegexMatch,
                TokenKind::And => BinaryOp::And,
                TokenKind::Or => BinaryOp::Or,
                _ => {
                    return Err(NebulaError::expression_parse_error(format!(
                        "Unexpected operator: {}",
                        self.current_token()
                    )));
                }
            };

            self.advance();

            let next_min_precedence = if is_right_assoc {
                precedence
            } else {
                precedence + 1
            };

            let right = self.parse_binary_op_with_depth(next_min_precedence, depth + 1)?;

            left = Expr::Binary {
                left: Box::new(left),
                op: binary_op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse unary expression with depth tracking
    fn parse_unary_with_depth(&mut self, depth: usize) -> ExpressionResult<Expr> {
        match &self.current_token().kind {
            TokenKind::Minus => {
                self.advance();
                let expr = self.parse_unary_with_depth(depth + 1)?;
                Ok(Expr::Negate(Box::new(expr)))
            }
            TokenKind::Not => {
                self.advance();
                let expr = self.parse_unary_with_depth(depth + 1)?;
                Ok(Expr::Not(Box::new(expr)))
            }
            _ => self.parse_postfix_with_depth(depth + 1),
        }
    }

    /// Parse postfix expression with depth tracking
    fn parse_postfix_with_depth(&mut self, depth: usize) -> ExpressionResult<Expr> {
        let mut expr = self.parse_primary_with_depth(depth + 1)?;

        loop {
            match &self.current_token().kind {
                TokenKind::Dot => {
                    self.advance();
                    let property = if let TokenKind::Identifier(name) = &self.current_token().kind {
                        let name = Arc::from(*name);
                        self.advance();
                        name
                    } else {
                        return Err(NebulaError::expression_parse_error(
                            "Expected property name after .",
                        ));
                    };

                    expr = Expr::PropertyAccess {
                        object: Box::new(expr),
                        property,
                    };
                }
                TokenKind::LeftBracket => {
                    self.advance();
                    let index = self.parse_expression_with_depth(depth + 1)?;
                    self.expect_token(TokenKind::RightBracket)?;

                    expr = Expr::IndexAccess {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    /// Parse primary expression with depth tracking
    fn parse_primary_with_depth(&mut self, depth: usize) -> ExpressionResult<Expr> {
        match &self.current_token().kind.clone() {
            // Literals
            TokenKind::Integer(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Literal(Value::integer(n)))
            }
            TokenKind::Float(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Literal(Value::float(n)))
            }
            TokenKind::String(s) => {
                let s: Arc<str> = Arc::from(*s);
                self.advance();
                Ok(Expr::Literal(Value::text(s.as_ref())))
            }
            TokenKind::Boolean(b) => {
                let b = *b;
                self.advance();
                Ok(Expr::Literal(Value::boolean(b)))
            }
            TokenKind::Null => {
                self.advance();
                Ok(Expr::Literal(Value::null()))
            }

            // Variables
            TokenKind::Variable(name) => {
                let name = Arc::from(*name);
                self.advance();
                Ok(Expr::Variable(name))
            }

            // Identifiers (could be function calls)
            TokenKind::Identifier(name) => {
                let name = Arc::from(*name);
                self.advance();
                if self.current_token().kind == TokenKind::LeftParen {
                    // Function call
                    let args = self.parse_function_args_with_depth(depth + 1)?;
                    Ok(Expr::FunctionCall { name, args })
                } else {
                    // Just an identifier
                    Ok(Expr::Identifier(name))
                }
            }

            // Parenthesized expression
            TokenKind::LeftParen => {
                self.advance();
                let expr = self.parse_expression_with_depth(depth + 1)?;
                self.expect_token(TokenKind::RightParen)?;
                Ok(expr)
            }

            // Array literal
            TokenKind::LeftBracket => {
                self.advance();
                let mut elements = Vec::new();

                if self.current_token().kind != TokenKind::RightBracket {
                    loop {
                        elements.push(self.parse_expression_with_depth(depth + 1)?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }

                self.expect_token(TokenKind::RightBracket)?;
                Ok(Expr::Array(elements))
            }

            // Object literal
            TokenKind::LeftBrace => {
                self.advance();
                let mut pairs = Vec::new();

                if self.current_token().kind != TokenKind::RightBrace {
                    loop {
                        // Parse key
                        let key = match &self.current_token().kind {
                            TokenKind::Identifier(name) => {
                                let k: Arc<str> = Arc::from(*name);
                                self.advance();
                                k
                            }
                            TokenKind::String(s) => {
                                let k: Arc<str> = Arc::from(*s);
                                self.advance();
                                k
                            }
                            _ => {
                                return Err(NebulaError::expression_parse_error(
                                    "Expected object key",
                                ));
                            }
                        };

                        self.expect_token(TokenKind::Colon)?;
                        let value = self.parse_expression_with_depth(depth + 1)?;
                        pairs.push((key, value));

                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }

                self.expect_token(TokenKind::RightBrace)?;
                Ok(Expr::Object(pairs))
            }

            _ => Err(NebulaError::expression_parse_error(format!(
                "Unexpected token: {}",
                self.current_token()
            ))),
        }
    }

    /// Parse function arguments with depth tracking
    fn parse_function_args_with_depth(&mut self, depth: usize) -> ExpressionResult<Vec<Expr>> {
        self.expect_token(TokenKind::LeftParen)?;
        let mut args = Vec::new();

        if self.current_token().kind != TokenKind::RightParen {
            loop {
                // Check for lambda expression (param => body)
                if let TokenKind::Identifier(param) = &self.current_token().kind {
                    let param_name = Arc::from(*param);
                    self.advance();
                    if self.match_token(&TokenKind::Arrow) {
                        // This is a lambda
                        let body = Box::new(self.parse_expression_with_depth(depth + 1)?);
                        args.push(Expr::Lambda {
                            param: param_name,
                            body,
                        });
                    } else {
                        // Not a lambda, backtrack by parsing as identifier
                        let expr = Expr::Identifier(param_name);
                        // Continue parsing if there are more operations
                        let full_expr =
                            self.parse_postfix_from_primary_with_depth(expr, depth + 1)?;
                        args.push(full_expr);
                    }
                } else {
                    args.push(self.parse_expression_with_depth(depth + 1)?);
                }

                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }

        self.expect_token(TokenKind::RightParen)?;
        Ok(args)
    }

    /// Parse postfix operations starting from a given primary expression with depth tracking
    fn parse_postfix_from_primary_with_depth(
        &mut self,
        mut expr: Expr,
        depth: usize,
    ) -> ExpressionResult<Expr> {
        loop {
            match &self.current_token().kind {
                TokenKind::Dot => {
                    self.advance();
                    let property = if let TokenKind::Identifier(name) = &self.current_token().kind {
                        let name = Arc::from(*name);
                        self.advance();
                        name
                    } else {
                        return Err(NebulaError::expression_parse_error(
                            "Expected property name after .",
                        ));
                    };

                    expr = Expr::PropertyAccess {
                        object: Box::new(expr),
                        property,
                    };
                }
                TokenKind::LeftBracket => {
                    self.advance();
                    let index = self.parse_expression_with_depth(depth + 1)?;
                    self.expect_token(TokenKind::RightBracket)?;

                    expr = Expr::IndexAccess {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// Get the current token
    fn current_token(&self) -> &Token<'a> {
        self.tokens.get(self.position).unwrap_or(&EOF_TOKEN)
    }

    /// Advance to the next token
    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    /// Match a specific token and advance if it matches
    fn match_token(&mut self, expected: &TokenKind) -> bool {
        if self.current_token().kind == *expected {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Expect a specific token and advance, or return an error
    fn expect_token(&mut self, expected: TokenKind) -> ExpressionResult<()> {
        if self.current_token().kind == expected {
            self.advance();
            Ok(())
        } else {
            Err(NebulaError::expression_parse_error(format!(
                "Expected {}, found {}",
                expected,
                self.current_token()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(input: &str) -> ExpressionResult<Expr> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        parser.parse()
    }

    #[test]
    fn test_parse_literal() {
        let expr = parse("42").unwrap();
        assert!(matches!(expr, Expr::Literal(_)));
    }

    #[test]
    fn test_parse_binary_expression() {
        let expr = parse("2 + 3").unwrap();
        assert!(matches!(
            expr,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn test_parse_variable() {
        let expr = parse("$node").unwrap();
        assert!(matches!(expr, Expr::Variable(_)));
    }

    #[test]
    fn test_parse_function_call() {
        let expr = parse("length('hello')").unwrap();
        assert!(matches!(expr, Expr::FunctionCall { .. }));
    }

    #[test]
    fn test_parse_property_access() {
        let expr = parse("$node.data").unwrap();
        assert!(matches!(expr, Expr::PropertyAccess { .. }));
    }

    #[test]
    fn test_parse_conditional() {
        let expr = parse("if true then 1 else 2").unwrap();
        assert!(matches!(expr, Expr::Conditional { .. }));
    }

    #[test]
    fn test_parser_recursion_depth_safe() {
        // Create a moderately nested expression that should parse successfully
        // Each level of parentheses consumes ~6 depth increments (through the call chain)
        // So 40 parentheses = ~240 depth, which is safely under MAX_PARSER_DEPTH of 256
        let mut expr = String::from("1");
        for _ in 0..40 {
            expr = format!("({})", expr);
        }

        let result = parse(&expr);
        assert!(
            result.is_ok(),
            "Parser should handle 40 levels of nesting: {}",
            result
                .as_ref()
                .err()
                .map(|e| format!("{:?}", e))
                .unwrap_or_default()
        );
    }

    #[test]
    fn test_parser_recursion_depth_within_limits() {
        // Test various expression types with nesting

        // Nested arithmetic
        let expr = parse("1 + (2 + (3 + (4 + 5)))").unwrap();
        assert!(matches!(expr, Expr::Binary { .. }));

        // Nested conditionals
        let expr = parse("if true then (if false then 1 else 2) else 3").unwrap();
        assert!(matches!(expr, Expr::Conditional { .. }));

        // Nested property access
        let expr = parse("$node.data.items.first.value").unwrap();
        assert!(matches!(expr, Expr::PropertyAccess { .. }));
    }
}
