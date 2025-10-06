//! Parser for converting tokens into an AST
//!
//! This module implements a recursive descent parser with precedence climbing for operators.

use crate::core::ast::{BinaryOp, Expr};
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::core::token::Token;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Parser for converting tokens into an AST
pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    /// Create a new parser from a list of tokens
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    /// Parse the tokens into an expression AST
    pub fn parse(&mut self) -> ExpressionResult<Expr> {
        self.parse_expression()
    }

    /// Parse a full expression
    fn parse_expression(&mut self) -> ExpressionResult<Expr> {
        self.parse_conditional()
    }

    /// Parse conditional expression (if-then-else)
    fn parse_conditional(&mut self) -> ExpressionResult<Expr> {
        if self.match_token(&Token::If) {
            let condition = Box::new(self.parse_pipeline()?);
            self.expect_token(Token::Then)?;
            let then_expr = Box::new(self.parse_pipeline()?);
            self.expect_token(Token::Else)?;
            let else_expr = Box::new(self.parse_pipeline()?);

            Ok(Expr::Conditional {
                condition,
                then_expr,
                else_expr,
            })
        } else {
            self.parse_pipeline()
        }
    }

    /// Parse pipeline expression
    fn parse_pipeline(&mut self) -> ExpressionResult<Expr> {
        let mut expr = self.parse_binary_expression(0)?;

        while self.current_token() == &Token::Pipe {
            self.advance();

            // Expect function name
            let function = if let Token::Identifier(name) = self.current_token() {
                let name = name.clone();
                self.advance();
                name
            } else {
                return Err(NebulaError::expression_parse_error(
                    "Expected function name after |",
                ));
            };

            // Parse arguments if present
            let args = if self.current_token() == &Token::LeftParen {
                self.parse_function_args()?
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

    /// Parse binary expression with precedence climbing
    fn parse_binary_expression(&mut self, min_precedence: u8) -> ExpressionResult<Expr> {
        let mut left = self.parse_unary_expression()?;

        while self.current_token().is_binary_operator() {
            let op_token = self.current_token().clone();
            let precedence = op_token.precedence();

            if precedence < min_precedence {
                break;
            }

            self.advance();

            let next_min_precedence = if op_token.is_right_associative() {
                precedence
            } else {
                precedence + 1
            };

            let right = self.parse_binary_expression(next_min_precedence)?;

            let binary_op = match op_token {
                Token::Plus => BinaryOp::Add,
                Token::Minus => BinaryOp::Subtract,
                Token::Star => BinaryOp::Multiply,
                Token::Slash => BinaryOp::Divide,
                Token::Percent => BinaryOp::Modulo,
                Token::Power => BinaryOp::Power,
                Token::Equal => BinaryOp::Equal,
                Token::NotEqual => BinaryOp::NotEqual,
                Token::LessThan => BinaryOp::LessThan,
                Token::GreaterThan => BinaryOp::GreaterThan,
                Token::LessEqual => BinaryOp::LessEqual,
                Token::GreaterEqual => BinaryOp::GreaterEqual,
                Token::RegexMatch => BinaryOp::RegexMatch,
                Token::And => BinaryOp::And,
                Token::Or => BinaryOp::Or,
                _ => {
                    return Err(NebulaError::expression_parse_error(format!(
                        "Unexpected operator: {}",
                        op_token
                    )))
                }
            };

            left = Expr::Binary {
                left: Box::new(left),
                op: binary_op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse unary expression
    fn parse_unary_expression(&mut self) -> ExpressionResult<Expr> {
        match self.current_token() {
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary_expression()?;
                Ok(Expr::Negate(Box::new(expr)))
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_unary_expression()?;
                Ok(Expr::Not(Box::new(expr)))
            }
            _ => self.parse_postfix_expression(),
        }
    }

    /// Parse postfix expression (property access, index access)
    fn parse_postfix_expression(&mut self) -> ExpressionResult<Expr> {
        let mut expr = self.parse_primary_expression()?;

        loop {
            match self.current_token() {
                Token::Dot => {
                    self.advance();
                    let property = if let Token::Identifier(name) = self.current_token() {
                        let name = name.clone();
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
                Token::LeftBracket => {
                    self.advance();
                    let index = self.parse_expression()?;
                    self.expect_token(Token::RightBracket)?;

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

    /// Parse primary expression (literals, variables, function calls, etc.)
    fn parse_primary_expression(&mut self) -> ExpressionResult<Expr> {
        match self.current_token().clone() {
            // Literals
            Token::Integer(n) => {
                self.advance();
                Ok(Expr::Literal(Value::integer(n)))
            }
            Token::Float(n) => {
                self.advance();
                Ok(Expr::Literal(Value::float(n)))
            }
            Token::String(s) => {
                self.advance();
                Ok(Expr::Literal(Value::text(s)))
            }
            Token::Boolean(b) => {
                self.advance();
                Ok(Expr::Literal(Value::boolean(b)))
            }
            Token::Null => {
                self.advance();
                Ok(Expr::Literal(Value::null()))
            }

            // Variables
            Token::Variable(name) => {
                self.advance();
                Ok(Expr::Variable(name))
            }

            // Identifiers (could be function calls)
            Token::Identifier(name) => {
                self.advance();
                if self.current_token() == &Token::LeftParen {
                    // Function call
                    let args = self.parse_function_args()?;
                    Ok(Expr::FunctionCall { name, args })
                } else {
                    // Just an identifier
                    Ok(Expr::Identifier(name))
                }
            }

            // Parenthesized expression
            Token::LeftParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect_token(Token::RightParen)?;
                Ok(expr)
            }

            // Array literal
            Token::LeftBracket => {
                self.advance();
                let mut elements = Vec::new();

                if self.current_token() != &Token::RightBracket {
                    loop {
                        elements.push(self.parse_expression()?);
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }

                self.expect_token(Token::RightBracket)?;
                Ok(Expr::Array(elements))
            }

            // Object literal
            Token::LeftBrace => {
                self.advance();
                let mut pairs = Vec::new();

                if self.current_token() != &Token::RightBrace {
                    loop {
                        // Parse key
                        let key = match self.current_token() {
                            Token::Identifier(name) => {
                                let k = name.clone();
                                self.advance();
                                k
                            }
                            Token::String(s) => {
                                let k = s.clone();
                                self.advance();
                                k
                            }
                            _ => {
                                return Err(NebulaError::expression_parse_error(
                                    "Expected object key",
                                ))
                            }
                        };

                        self.expect_token(Token::Colon)?;
                        let value = self.parse_expression()?;
                        pairs.push((key, value));

                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }

                self.expect_token(Token::RightBrace)?;
                Ok(Expr::Object(pairs))
            }

            token => Err(NebulaError::expression_parse_error(format!(
                "Unexpected token: {}",
                token
            ))),
        }
    }

    /// Parse function arguments
    fn parse_function_args(&mut self) -> ExpressionResult<Vec<Expr>> {
        self.expect_token(Token::LeftParen)?;
        let mut args = Vec::new();

        if self.current_token() != &Token::RightParen {
            loop {
                // Check for lambda expression (param => body)
                if let Token::Identifier(param) = self.current_token() {
                    let param_name = param.clone();
                    self.advance();
                    if self.match_token(&Token::Arrow) {
                        // This is a lambda
                        let body = Box::new(self.parse_expression()?);
                        args.push(Expr::Lambda {
                            param: param_name,
                            body,
                        });
                    } else {
                        // Not a lambda, backtrack by parsing as identifier
                        let expr = Expr::Identifier(param_name);
                        // Continue parsing if there are more operations
                        let full_expr = self.parse_postfix_from_primary(expr)?;
                        args.push(full_expr);
                    }
                } else {
                    args.push(self.parse_expression()?);
                }

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        self.expect_token(Token::RightParen)?;
        Ok(args)
    }

    /// Parse postfix operations starting from a given primary expression
    fn parse_postfix_from_primary(&mut self, mut expr: Expr) -> ExpressionResult<Expr> {
        loop {
            match self.current_token() {
                Token::Dot => {
                    self.advance();
                    let property = if let Token::Identifier(name) = self.current_token() {
                        let name = name.clone();
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
                Token::LeftBracket => {
                    self.advance();
                    let index = self.parse_expression()?;
                    self.expect_token(Token::RightBracket)?;

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
    fn current_token(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }

    /// Advance to the next token
    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    /// Match a specific token and advance if it matches
    fn match_token(&mut self, expected: &Token) -> bool {
        if self.current_token() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Expect a specific token and advance, or return an error
    fn expect_token(&mut self, expected: Token) -> ExpressionResult<()> {
        if self.current_token() == &expected {
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
}
