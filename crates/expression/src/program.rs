//! Retained syntax programs shared by validation and runtime evaluation.

use std::{collections::BTreeMap, fmt, sync::Arc};

use crate::{
    EvaluationContext, ExpressionError, ExpressionResult, Template, TemplatePart,
    ast::Expr,
    eval::{EvalFrame, Evaluator},
    lexer::Lexer,
    parser::Parser,
    template::Position,
    value::RuntimeValue,
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
    Template(Arc<[TemplateNode]>),
}

/// One node of a compiled template tree.
///
/// Static text and expressions are leaves; `if`/`for` nest their bodies.
enum TemplateNode {
    Text(Arc<str>),
    Expression {
        expression: Arc<Expr>,
        position: Position,
        strip_left: bool,
        strip_right: bool,
    },
    /// `{% if cond %}…{% elif cond %}…{% else %}…{% endif %}`
    If {
        branches: Box<[IfBranch]>,
        otherwise: Box<[TemplateNode]>,
        /// `{%-` on the opening tag: strip output before the block.
        open_strip_left: bool,
        /// `-%}` on the opening tag: strip the selected body's leading text.
        open_strip_right: bool,
        /// `{%-` on the closing tag: strip output before the closing tag.
        close_strip_left: bool,
        /// `-%}` on the closing tag: strip text after the block.
        close_strip_right: bool,
    },
    /// `{% for name in iterable %}…{% else %}…{% endfor %}`
    ///
    /// `otherwise` renders when the iterable is empty.
    For {
        parameter: Arc<str>,
        iterable: Arc<Expr>,
        body: Box<[TemplateNode]>,
        otherwise: Box<[TemplateNode]>,
        position: Position,
        /// `{%-` on the opening tag: strip output before the block.
        open_strip_left: bool,
        /// `-%}` on the opening tag: strip each body iteration's leading text.
        open_strip_right: bool,
        /// `{%-` on the closing tag: strip output before the closing tag.
        close_strip_left: bool,
        /// `-%}` on the closing tag: strip text after the block.
        close_strip_right: bool,
    },
}

/// A single condition and the body it guards.
struct IfBranch {
    condition: Arc<Expr>,
    body: Box<[TemplateNode]>,
    position: Position,
}

/// A raw tag body taken from a [`TemplatePart::Tag`].
struct RawTag {
    content: Arc<str>,
    position: Position,
    strip_left: bool,
    strip_right: bool,
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
                // A lone `{{ … }}` envelope, with only whitespace around it,
                // keeps its JSON type. Anything else — mixed text or a
                // control-flow tag — stays a string template.
                if let ProgramBody::Template(nodes) = program.body.as_ref()
                    && let Some(expression) = lone_envelope_expression(nodes)
                {
                    program.body = Arc::new(ProgramBody::Expression(expression));
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
        let nodes = build_template_tree(parts)?;
        Ok(Self {
            source,
            syntax: ProgramSyntax::Template,
            body: Arc::new(ProgramBody::Template(nodes.into())),
        })
    }

    pub(crate) fn evaluate(
        &self,
        evaluator: &Evaluator,
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<RuntimeValue> {
        match self.body.as_ref() {
            ProgramBody::Expression(expression) => {
                evaluator.eval_with_frame(expression, context, frame)
            },
            ProgramBody::Template(_) => Ok(RuntimeValue::string(
                self.render_text(evaluator, context, frame)?,
            )),
        }
    }

    /// Whether the retained body renders text rather than a typed value.
    pub(crate) fn is_template(&self) -> bool {
        matches!(self.body.as_ref(), ProgramBody::Template(_))
    }

    /// Render a template body to text.
    ///
    /// Separate from [`Self::evaluate`] so the text path never round-trips
    /// through `RuntimeValue`: wrapping the rendered `String` in an `Arc<str>`
    /// and unwrapping it again would copy the whole output twice.
    pub(crate) fn render_text(
        &self,
        evaluator: &Evaluator,
        context: &EvaluationContext,
        frame: &EvalFrame,
    ) -> ExpressionResult<String> {
        match self.body.as_ref() {
            ProgramBody::Template(nodes) => {
                let mut output = String::with_capacity(self.source.len());
                let mut strip_next_leading = false;
                render_nodes(
                    nodes,
                    evaluator,
                    context,
                    frame,
                    &mut output,
                    &mut strip_next_leading,
                )?;
                Ok(output)
            },
            // An auto-compiled lone envelope renders its value as text.
            ProgramBody::Expression(expression) => evaluator
                .eval_borrowed_with_frame(expression, context, frame)
                .map(|value| value.to_display_string()),
        }
    }
}

/// The single expression of a lone `{{ … }}` envelope, if that is the shape.
///
/// AUTO compilation keeps the JSON type of a lone envelope, with optional
/// surrounding whitespace. Mixed text and any control-flow tag make the result
/// a string, so this returns `None` for them.
fn lone_envelope_expression(nodes: &[TemplateNode]) -> Option<Arc<Expr>> {
    let mut expression = None;
    for node in nodes {
        match node {
            TemplateNode::Text(text) if text.trim().is_empty() => {},
            TemplateNode::Expression {
                expression: found, ..
            } if expression.is_none() => {
                expression = Some(Arc::clone(found));
            },
            _ => return None,
        }
    }
    expression
}

/// Render a node list into `output`.
///
/// `strip_next_leading` carries `-}}`/`-%}` whitespace control across sibling
/// nodes; branch bodies start with it cleared because each branch is a fresh
/// output region.
fn render_nodes(
    nodes: &[TemplateNode],
    evaluator: &Evaluator,
    context: &EvaluationContext,
    frame: &EvalFrame,
    output: &mut String,
    strip_next_leading: &mut bool,
) -> ExpressionResult<()> {
    for node in nodes {
        match node {
            TemplateNode::Text(text) => {
                append_output(
                    output,
                    if *strip_next_leading {
                        text.trim_start()
                    } else {
                        text
                    },
                    frame,
                )?;
                *strip_next_leading = false;
            },
            TemplateNode::Expression {
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
                    .map_err(|error| template_error(error, *position))?;
                // A string renders by reference: `to_display_string` would
                // allocate a copy of every interpolated string, and this is
                // the hot path of template rendering.
                match value.as_ref() {
                    RuntimeValue::String(text) => append_output(output, text, frame)?,
                    other => append_output(output, &other.to_display_string(), frame)?,
                }
                *strip_next_leading = *strip_right;
            },
            TemplateNode::If {
                branches,
                otherwise,
                open_strip_left,
                open_strip_right,
                close_strip_left,
                close_strip_right,
            } => {
                if *open_strip_left {
                    output.truncate(output.trim_end().len());
                }
                let mut taken = false;
                for branch in branches {
                    let condition = evaluator
                        .eval_borrowed_with_frame(&branch.condition, context, frame)
                        .map_err(|error| template_error(error, branch.position))?;
                    if crate::value_utils::to_boolean(&condition) {
                        let mut branch_strip = *open_strip_right;
                        render_nodes(
                            &branch.body,
                            evaluator,
                            context,
                            frame,
                            output,
                            &mut branch_strip,
                        )?;
                        taken = true;
                        break;
                    }
                }
                if !taken {
                    let mut branch_strip = *open_strip_right;
                    render_nodes(
                        otherwise,
                        evaluator,
                        context,
                        frame,
                        output,
                        &mut branch_strip,
                    )?;
                }
                if *close_strip_left {
                    output.truncate(output.trim_end().len());
                }
                *strip_next_leading = *close_strip_right;
            },
            TemplateNode::For {
                parameter,
                iterable,
                body,
                otherwise,
                position,
                open_strip_left,
                open_strip_right,
                close_strip_left,
                close_strip_right,
            } => {
                if *open_strip_left {
                    output.truncate(output.trim_end().len());
                }
                let value = evaluator
                    .eval_borrowed_with_frame(iterable, context, frame)
                    .map_err(|error| template_error(error, *position))?;
                let items =
                    value
                        .as_array()
                        .ok_or_else(|| ExpressionError::TemplateEvaluation {
                            position: *position,
                            source: Box::new(ExpressionError::type_error(
                                "array",
                                crate::value_utils::value_type_name(&value),
                            )),
                        })?;
                if items.is_empty() {
                    let mut branch_strip = *open_strip_right;
                    render_nodes(
                        otherwise,
                        evaluator,
                        context,
                        frame,
                        output,
                        &mut branch_strip,
                    )?;
                    if *close_strip_left {
                        output.truncate(output.trim_end().len());
                    }
                    *strip_next_leading = *close_strip_right;
                    continue;
                }
                for (index, item) in items.iter().enumerate() {
                    let mut iteration_context = context.clone();
                    iteration_context.set_lambda_var(parameter, item.clone());
                    bind_loop_object(&mut iteration_context, index, items.len());
                    let mut body_strip = *open_strip_right;
                    render_nodes(
                        body,
                        evaluator,
                        &iteration_context,
                        frame,
                        output,
                        &mut body_strip,
                    )?;
                    // `{%- endfor %}` trims the text before the tag, and the
                    // tag stands at the end of every iteration.
                    if *close_strip_left {
                        output.truncate(output.trim_end().len());
                    }
                }
                *strip_next_leading = *close_strip_right;
            },
        }
    }
    Ok(())
}

/// Wrap evaluation failures in the template position, preserving budget errors.
fn template_error(error: ExpressionError, position: Position) -> ExpressionError {
    match error {
        ExpressionError::StepLimitExceeded { .. }
        | ExpressionError::DepthExceeded { .. }
        | ExpressionError::ResourceLimitExceeded { .. }
        | ExpressionError::BuiltinOutputLimitExceeded { .. } => error,
        source => ExpressionError::TemplateEvaluation {
            position,
            source: Box::new(source),
        },
    }
}

/// Bind the `loop` object for one `{% for %}` iteration.
///
/// Mirrors the Jinja/n8n surface: `loop.index` (1-based), `loop.index0`
/// (0-based), `loop.first`, `loop.last`, and `loop.length`.
fn bind_loop_object(context: &mut EvaluationContext, index: usize, length: usize) {
    let index = index as i64;
    let length = length as i64;
    let mut fields: BTreeMap<Arc<str>, RuntimeValue> = BTreeMap::new();
    fields.insert(Arc::from("index"), RuntimeValue::Integer(index + 1));
    fields.insert(Arc::from("index0"), RuntimeValue::Integer(index));
    fields.insert(Arc::from("first"), RuntimeValue::Bool(index == 0));
    fields.insert(Arc::from("last"), RuntimeValue::Bool(index + 1 == length));
    fields.insert(Arc::from("length"), RuntimeValue::Integer(length));
    context.set_lambda_var("loop", RuntimeValue::Object(Arc::new(fields)));
}

/// Build the nested template tree from the flat part stream.
///
/// Blocks nest by construction: `{% if %}`/`{% elif %}`/`{% else %}`/`{% endif %}`
/// and `{% for %}`/`{% else %}`/`{% endfor %}` are matched here, so a malformed
/// template fails at compile time with the opening tag's position.
fn build_template_tree(parts: &[TemplatePart]) -> ExpressionResult<Vec<TemplateNode>> {
    let mut cursor = 0;
    let nodes = build_nodes(parts, &mut cursor, None)?;
    if cursor != parts.len() {
        return Err(ExpressionError::parse_error(
            "Template block structure is unbalanced",
        ));
    }
    Ok(nodes)
}

/// Parse one node list until `terminator` or the end of input.
///
/// Returns the nodes; consumes the terminator tag when one matches.
fn build_nodes(
    parts: &[TemplatePart],
    cursor: &mut usize,
    terminator: Option<&[&str]>,
) -> ExpressionResult<Vec<TemplateNode>> {
    let mut nodes = Vec::new();
    while *cursor < parts.len() {
        match &parts[*cursor] {
            TemplatePart::Static { content, .. } => {
                nodes.push(TemplateNode::Text(Arc::clone(content)));
                *cursor += 1;
            },
            TemplatePart::Expression { .. } => {
                nodes.push(expression_node(parts, *cursor)?);
                *cursor += 1;
            },
            TemplatePart::Tag { .. } => {
                let tag = raw_tag(parts, *cursor)?;
                let keyword = tag_keyword(&tag.content);
                if let Some(terminators) = terminator
                    && terminators.contains(&keyword)
                {
                    return Ok(nodes);
                }
                match keyword {
                    "if" => nodes.push(build_if(parts, cursor)?),
                    "for" => nodes.push(build_for(parts, cursor)?),
                    "endif" | "endfor" | "else" | "elif" => {
                        return Err(ExpressionError::parse_error_at(
                            tag.position,
                            format!("Unexpected `{keyword}` without a matching opening tag"),
                        ));
                    },
                    other => {
                        return Err(ExpressionError::parse_error_at(
                            tag.position,
                            format!("Unknown template tag `{other}`"),
                        ));
                    },
                }
            },
        }
    }
    Ok(nodes)
}

fn expression_node(parts: &[TemplatePart], index: usize) -> ExpressionResult<TemplateNode> {
    let TemplatePart::Expression {
        content,
        position,
        strip_left,
        strip_right,
        ..
    } = &parts[index]
    else {
        return Err(ExpressionError::internal("expected an expression part"));
    };
    Ok(TemplateNode::Expression {
        expression: Arc::new(parse_raw(content.trim())?),
        position: *position,
        strip_left: *strip_left,
        strip_right: *strip_right,
    })
}

fn raw_tag(parts: &[TemplatePart], index: usize) -> ExpressionResult<RawTag> {
    let TemplatePart::Tag {
        content,
        position,
        strip_left,
        strip_right,
    } = &parts[index]
    else {
        return Err(ExpressionError::internal("expected a tag part"));
    };
    Ok(RawTag {
        content: Arc::clone(content),
        position: *position,
        strip_left: *strip_left,
        strip_right: *strip_right,
    })
}

/// The first word of a tag body, e.g. `if` in `if $input.count > 0`.
fn tag_keyword(content: &str) -> &str {
    content.split_whitespace().next().unwrap_or("")
}

/// The remainder of a tag body after its keyword.
fn tag_argument(content: &str) -> &str {
    let trimmed = content.trim();
    trimmed
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim())
}

/// Parse `{% if cond %}…{% elif cond %}…{% else %}…{% endif %}`.
///
/// The cursor points at the opening tag on entry and at `{% endif %}` on exit.
fn build_if(parts: &[TemplatePart], cursor: &mut usize) -> ExpressionResult<TemplateNode> {
    let opening = raw_tag(parts, *cursor)?;
    let mut branches: Vec<IfBranch> = Vec::new();

    let condition_source = tag_argument(&opening.content);
    if condition_source.is_empty() {
        return Err(ExpressionError::parse_error_at(
            opening.position,
            "`if` requires a condition",
        ));
    }
    let mut current_condition = Arc::new(parse_raw(condition_source)?);
    let mut current_position = opening.position;
    *cursor += 1;

    loop {
        let body = build_nodes(parts, cursor, Some(&["elif", "else", "endif"]))?;
        branches.push(IfBranch {
            condition: Arc::clone(&current_condition),
            body: body.into_boxed_slice(),
            position: current_position,
        });
        let Some(_) = parts.get(*cursor) else {
            return Err(ExpressionError::parse_error_at(
                current_position,
                "Unclosed `{% if %}` - expected `{% endif %}`",
            ));
        };
        let tag = raw_tag(parts, *cursor)?;
        match tag_keyword(&tag.content) {
            "elif" => {
                let source = tag_argument(&tag.content);
                if source.is_empty() {
                    return Err(ExpressionError::parse_error_at(
                        tag.position,
                        "`elif` requires a condition",
                    ));
                }
                current_condition = Arc::new(parse_raw(source)?);
                current_position = tag.position;
                *cursor += 1;
            },
            "else" => {
                *cursor += 1;
                let otherwise = build_nodes(parts, cursor, Some(&["endif"]))?;
                let Some(_) = parts.get(*cursor) else {
                    return Err(ExpressionError::parse_error_at(
                        opening.position,
                        "Unclosed `{% if %}` - expected `{% endif %}`",
                    ));
                };
                let closing = raw_tag(parts, *cursor)?;
                if tag_keyword(&closing.content) != "endif" {
                    return Err(ExpressionError::parse_error_at(
                        closing.position,
                        "Expected `{% endif %}`",
                    ));
                }
                *cursor += 1;
                return Ok(TemplateNode::If {
                    branches: branches.into_boxed_slice(),
                    otherwise: otherwise.into_boxed_slice(),
                    open_strip_left: opening.strip_left,
                    open_strip_right: opening.strip_right,
                    close_strip_left: closing.strip_left,
                    close_strip_right: closing.strip_right,
                });
            },
            "endif" => {
                *cursor += 1;
                return Ok(TemplateNode::If {
                    branches: branches.into_boxed_slice(),
                    otherwise: Box::default(),
                    open_strip_left: opening.strip_left,
                    open_strip_right: opening.strip_right,
                    close_strip_left: tag.strip_left,
                    close_strip_right: tag.strip_right,
                });
            },
            other => {
                return Err(ExpressionError::parse_error_at(
                    tag.position,
                    format!(
                        "Expected `{{% elif %}}`, `{{% else %}}`, or `{{% endif %}}`, found `{other}`"
                    ),
                ));
            },
        }
    }
}

/// Parse `{% for name in iterable %}…{% else %}…{% endfor %}`.
///
/// The cursor points at the opening tag on entry and past `{% endfor %}` on
/// exit.
fn build_for(parts: &[TemplatePart], cursor: &mut usize) -> ExpressionResult<TemplateNode> {
    let opening = raw_tag(parts, *cursor)?;
    let argument = tag_argument(&opening.content);
    let (parameter, iterable_source) = argument.split_once(" in ").ok_or_else(|| {
        ExpressionError::parse_error_at(
            opening.position,
            "`for` requires `for <name> in <expression>`",
        )
    })?;
    let parameter = parameter.trim();
    if parameter.is_empty() || !is_valid_binding_name(parameter) {
        return Err(ExpressionError::parse_error_at(
            opening.position,
            format!("Invalid `for` binding name `{parameter}`"),
        ));
    }
    let iterable_source = iterable_source.trim();
    if iterable_source.is_empty() {
        return Err(ExpressionError::parse_error_at(
            opening.position,
            "`for` requires an iterable expression",
        ));
    }
    let iterable = Arc::new(parse_raw(iterable_source)?);
    let parameter: Arc<str> = Arc::from(parameter);
    *cursor += 1;

    let body = build_nodes(parts, cursor, Some(&["else", "endfor"]))?;
    let Some(_) = parts.get(*cursor) else {
        return Err(ExpressionError::parse_error_at(
            opening.position,
            "Unclosed `{% for %}` - expected `{% endfor %}`",
        ));
    };
    let tag = raw_tag(parts, *cursor)?;
    let closing_strip_right;
    let closing_strip_left;
    let otherwise = match tag_keyword(&tag.content) {
        "endfor" => {
            closing_strip_right = tag.strip_right;
            closing_strip_left = tag.strip_left;
            *cursor += 1;
            Vec::new()
        },
        "else" => {
            *cursor += 1;
            let otherwise = build_nodes(parts, cursor, Some(&["endfor"]))?;
            let Some(_) = parts.get(*cursor) else {
                return Err(ExpressionError::parse_error_at(
                    opening.position,
                    "Unclosed `{% for %}` - expected `{% endfor %}`",
                ));
            };
            let closing = raw_tag(parts, *cursor)?;
            if tag_keyword(&closing.content) != "endfor" {
                return Err(ExpressionError::parse_error_at(
                    closing.position,
                    "Expected `{% endfor %}`",
                ));
            }
            closing_strip_right = closing.strip_right;
            closing_strip_left = closing.strip_left;
            *cursor += 1;
            otherwise
        },
        other => {
            return Err(ExpressionError::parse_error_at(
                tag.position,
                format!("Expected `{{% else %}}` or `{{% endfor %}}`, found `{other}`"),
            ));
        },
    };

    Ok(TemplateNode::For {
        parameter,
        iterable,
        body: body.into_boxed_slice(),
        otherwise: otherwise.into_boxed_slice(),
        position: opening.position,
        open_strip_left: opening.strip_left,
        open_strip_right: opening.strip_right,
        close_strip_left: closing_strip_left,
        close_strip_right: closing_strip_right,
    })
}

/// Whether a `for` binding is an identifier the expression language can use.
fn is_valid_binding_name(name: &str) -> bool {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {},
        _ => return false,
    }
    characters.all(|character| character.is_alphanumeric() || character == '_')
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
