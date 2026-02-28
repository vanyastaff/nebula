# Archived From "docs/archive/crates-architecture.md"

## 8. nebula-template

**Purpose**: Template engine for expressions and text generation.

```rust
// nebula-template/src/lib.rs
pub mod engine;
pub mod expression;
pub mod functions;

// nebula-template/src/engine.rs
pub struct TemplateEngine {
    handlebars: Handlebars<'static>,
    custom_functions: HashMap<String, Box<dyn TemplateFunction>>,
}

impl TemplateEngine {
    pub fn new() -> Self {
        let mut handlebars = Handlebars::new();
        
        // Register custom helpers
        handlebars.register_helper("json", Box::new(json_helper));
        handlebars.register_helper("base64", Box::new(base64_helper));
        
        Self {
            handlebars,
            custom_functions: HashMap::new(),
        }
    }
    
    pub fn render(&self, template: &str, context: &TemplateContext) -> Result<String, Error> {
        self.handlebars.render_template(template, context)
            .map_err(|e| Error::TemplateError(e.to_string()))
    }
}

// nebula-template/src/expression.rs
pub struct ExpressionParser {
    parser: pest::Parser,
}

impl ExpressionParser {
    pub fn parse(&self, expression: &str) -> Result<Expression, Error> {
        // Parse expressions like {{ $node("http_request").json.data }}
    }
}

// nebula-template/src/functions.rs
pub trait TemplateFunction: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, args: &[Value]) -> Result<Value, Error>;
}

pub struct DateFormatFunction;

impl TemplateFunction for DateFormatFunction {
    fn name(&self) -> &str {
        "date_format"
    }
    
    fn execute(&self, args: &[Value]) -> Result<Value, Error> {
        // Implementation
    }
}
```

