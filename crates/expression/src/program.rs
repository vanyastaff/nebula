//! Retained syntax programs shared by validation and runtime evaluation.

use std::{fmt, sync::Arc};

use serde_json::Value;

use crate::{
    EvaluationContext, ExpressionError, ExpressionResult, Template, TemplatePart,
    ast::Expr,
    eval::{EvalFrame, Evaluator},
    lexer::Lexer,
    parser::Parser,
    template::Position,
};

/// The authored grammar of a program, independent of its resulting syntax tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgramSyntax {
    /// Raw syntax first, then envelopes or mixed text; lone envelopes retain type.
    Auto,
    /// Exactly one raw expression, without interpreting template envelopes.
    Expression,
    /// Text interpolation, always producing a JSON string, including lone envelopes.
    Template,
}

/// An immutable, parsed expression or text template.
///
/// Compilation checks syntax without resolving variables or calling functions.
/// Clones share the retained syntax; evaluation uses the engine's current policy
/// and registry and never parses the source again.
///
/// ```
/// use nebula_expression::{CompiledProgram, EvaluationContext, ExpressionEngine};
/// use serde_json::json;
///
/// let program = CompiledProgram::compile("{{ $input + 1 }}")?;
/// let context = EvaluationContext::builder().input(json!(4)).build();
/// assert_eq!(ExpressionEngine::new().evaluate_compiled(&program, &context)?, json!(5));
/// # Ok::<(), nebula_expression::ExpressionError>(())
/// ```
#[derive(Clone)]
pub struct CompiledProgram {
    source: Arc<str>,
    syntax: ProgramSyntax,
    body: Arc<ProgramBody>,
}

enum ProgramBody {
    Expression(Arc<Expr>),
    Template(Box<[Instruction]>),
}

enum Instruction {
    Text(Arc<str>),
    Expression {
        expression: Arc<Expr>,
        position: Position,
        strip_left: bool,
        strip_right: bool,
    },
}

impl CompiledProgram {
    /// Compile using an explicit authored grammar, retained by [`Self::syntax`].
    ///
    /// # Errors
    /// Returns a syntax or resource-limit error for the selected grammar.
    ///
    /// ```
    /// use nebula_expression::{CompiledProgram, ProgramSyntax};
    /// let program = CompiledProgram::compile_with_syntax("{{ 7 }}", ProgramSyntax::Template)?;
    /// assert_eq!(program.syntax(), ProgramSyntax::Template);
    /// # Ok::<(), nebula_expression::ExpressionError>(())
    /// ```
    #[tracing::instrument(level = "debug", skip_all, fields(source_bytes = source.len(), ?syntax))]
    pub fn compile_with_syntax(source: &str, syntax: ProgramSyntax) -> ExpressionResult<Self> {
        match syntax {
            ProgramSyntax::Auto => Self::compile(source),
            ProgramSyntax::Expression => Self::compile_expression(source),
            ProgramSyntax::Template => Self::compile_template(source),
        }
    }

    /// Compile raw syntax, a lone expression envelope, or mixed template text.
    ///
    /// Raw syntax takes precedence, including string literals containing template
    /// markers. A lone `{{ ... }}` envelope, with optional surrounding whitespace,
    /// keeps its JSON type. Mixed text evaluates to a string.
    ///
    /// # Errors
    /// Returns a syntax or parse error if neither form is valid.
    #[tracing::instrument(level = "debug", skip_all, fields(source_bytes = source.len()))]
    pub fn compile(source: &str) -> ExpressionResult<Self> {
        crate::limits::check_limit(
            "source bytes",
            source.len(),
            crate::limits::MAX_SOURCE_BYTES,
        )?;
        let mut program = match Self::compile_expression(source) {
            Ok(program) => program,
            Err(raw_error) => {
                let template = Template::new(source)?;
                if !template.has_expressions() {
                    return Err(raw_error);
                }
                let mut program = template.program().clone();
                if let ProgramBody::Template(instructions) = program.body.as_ref() {
                    let mut expressions =
                        instructions
                            .iter()
                            .filter_map(|instruction| match instruction {
                                Instruction::Expression { expression, .. } => Some(expression),
                                Instruction::Text(_) => None,
                            });
                    let single = expressions.next().filter(|_| expressions.next().is_none());
                    let only_whitespace = instructions.iter().all(|instruction| {
                        !matches!(instruction, Instruction::Text(text) if !text.trim().is_empty())
                    });
                    if let Some(expression) = single.filter(|_| only_whitespace) {
                        program.body = Arc::new(ProgramBody::Expression(Arc::clone(expression)));
                    }
                }
                program
            },
        };
        program.syntax = ProgramSyntax::Auto;
        Ok(program)
    }

    /// Compile exactly one raw expression, without interpreting template markers.
    ///
    /// # Errors
    /// Returns a syntax or parse error for invalid or trailing input.
    #[tracing::instrument(level = "debug", skip_all, fields(source_bytes = source.len()))]
    pub fn compile_expression(source: &str) -> ExpressionResult<Self> {
        let expression = parse_raw(source)?;
        Ok(Self {
            source: Arc::from(source),
            syntax: ProgramSyntax::Expression,
            body: Arc::new(ProgramBody::Expression(Arc::new(expression))),
        })
    }

    /// Compile a text template, including a template containing only static text.
    ///
    /// The result always evaluates to a JSON string, even for a lone envelope.
    ///
    /// # Errors
    /// Returns a syntax or parse error for invalid delimiters or embedded syntax.
    #[tracing::instrument(level = "debug", skip_all, fields(source_bytes = source.len()))]
    pub fn compile_template(source: &str) -> ExpressionResult<Self> {
        Ok(Template::new(source)?.program().clone())
    }

    /// Return the original, unmodified source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return the original compilation grammar, not the resulting body kind.
    #[must_use]
    pub const fn syntax(&self) -> ProgramSyntax {
        self.syntax
    }

    pub(crate) fn from_template_parts(
        source: Arc<str>,
        parts: &[TemplatePart],
    ) -> ExpressionResult<Self> {
        let instructions = parts
            .iter()
            .map(|part| match part {
                TemplatePart::Static { content, .. } => Ok(Instruction::Text(Arc::clone(content))),
                TemplatePart::Expression {
                    content,
                    position,
                    strip_left,
                    strip_right,
                    ..
                } => Ok(Instruction::Expression {
                    expression: Arc::new(parse_raw(content.trim())?),
                    position: *position,
                    strip_left: *strip_left,
                    strip_right: *strip_right,
                }),
            })
            .collect::<ExpressionResult<Box<[_]>>>()?;
        Ok(Self {
            source,
            syntax: ProgramSyntax::Template,
            body: Arc::new(ProgramBody::Template(instructions)),
        })
    }

    pub(crate) fn evaluate(
        &self,
        evaluator: &Evaluator,
        context: &EvaluationContext,
        frame: &mut EvalFrame,
    ) -> ExpressionResult<Value> {
        let instructions = match self.body.as_ref() {
            ProgramBody::Expression(expression) => {
                return evaluator.eval_with_frame(expression, context, frame);
            },
            ProgramBody::Template(instructions) => instructions,
        };

        let mut output = String::with_capacity(self.source.len());
        let mut strip_next_leading = false;
        for instruction in instructions {
            match instruction {
                Instruction::Text(text) => {
                    append_output(
                        &mut output,
                        if strip_next_leading {
                            text.trim_start()
                        } else {
                            text
                        },
                        frame,
                    )?;
                    strip_next_leading = false;
                },
                Instruction::Expression {
                    expression,
                    position,
                    strip_left,
                    strip_right,
                } => {
                    if *strip_left {
                        output.truncate(output.trim_end().len());
                    }
                    let value = evaluator
                        .eval_borrowed_with_frame(expression, context, frame)
                        .map_err(|error| match error {
                            ExpressionError::StepLimitExceeded { .. }
                            | ExpressionError::DepthExceeded { .. }
                            | ExpressionError::ResourceLimitExceeded { .. } => error,
                            source => ExpressionError::TemplateEvaluation {
                                position: *position,
                                source: Box::new(source),
                            },
                        })?;
                    match value.as_ref() {
                        Value::String(text) => append_output(&mut output, text, frame)?,
                        other => append_output(&mut output, &other.to_string(), frame)?,
                    }
                    strip_next_leading = *strip_right;
                },
            }
        }
        Ok(Value::String(output))
    }
}

impl fmt::Debug for CompiledProgram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompiledProgram")
            .field("source_bytes", &self.source.len())
            .field("syntax", &self.syntax)
            .field(
                "kind",
                &match self.body.as_ref() {
                    ProgramBody::Expression(_) => "expression",
                    ProgramBody::Template(_) => "template",
                },
            )
            .finish_non_exhaustive()
    }
}

pub(crate) fn parse_raw(source: &str) -> ExpressionResult<Expr> {
    let tokens = Lexer::for_expression(source).tokenize()?;
    Parser::new(tokens).parse()
}

fn append_output(output: &mut String, text: &str, frame: &EvalFrame) -> ExpressionResult<()> {
    crate::limits::check_limit(
        "template output bytes",
        output.len().saturating_add(text.len()),
        crate::limits::MAX_RESULT_BYTES,
    )?;
    frame.charge(text.len())?;
    output.push_str(text);
    Ok(())
}
