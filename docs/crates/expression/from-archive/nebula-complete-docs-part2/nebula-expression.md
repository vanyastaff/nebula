---

# nebula-expression

## Purpose

`nebula-expression` provides a powerful expression language for dynamic value computation in workflows. It enables users to reference node outputs, transform data, and create complex logic without code.

## Responsibilities

- Expression parsing and validation
- AST construction and optimization
- Expression evaluation with context
- Function library management
- Type checking and coercion
- Performance optimization

## Architecture

### Core Components

```rust
pub struct ExpressionEngine {
    parser: Parser,
    evaluator: Evaluator,
    functions: FunctionRegistry,
    operators: OperatorRegistry,
    type_checker: TypeChecker,
    optimizer: Optimizer,
}
```

### Expression Grammar

```ebnf
expression     = ternary
ternary        = logical_or ("?" expression ":" expression)?
logical_or     = logical_and ("||" logical_and)*
logical_and    = equality ("&&" equality)*
equality       = comparison (("==" | "!=") comparison)*
comparison     = addition (("<" | ">" | "<=" | ">=") addition)*
addition       = multiplication (("+" | "-") multiplication)*
multiplication = unary (("*" | "/" | "%") unary)*
unary          = ("!" | "-")? postfix
postfix        = primary (accessor | call | index)*
primary        = literal | variable | "(" expression ")"

accessor       = "." identifier
call           = "(" arguments? ")"
index          = "[" expression "]"
arguments      = expression ("," expression)*

variable       = "$" identifier ("." identifier)*
literal        = string | number | boolean | null
```

## Expression Types

### Variable Access

```rust
// Node outputs
$nodes.http_request.body
$nodes.transform.result.users[0]

// Workflow variables  
$vars.api_key
$vars.user_settings.theme

// System variables
$context.execution_id
$context.workflow_name
$context.current_node

// Environment variables
$env.DATABASE_URL
$env.API_ENDPOINT
```

### Operators

```rust
// Arithmetic
$nodes.calc.value + 10
$nodes.price.amount * 1.2
$nodes.total.sum / $nodes.count.value

// Comparison
$nodes.age.value >= 18
$nodes.status.code == 200
$nodes.name.value != ""

// Logical
$nodes.is_active && $nodes.is_verified
$nodes.error || $nodes.fallback
!$nodes.completed

// String concatenation
$nodes.first_name + " " + $nodes.last_name

// Null coalescing
$nodes.optional.value ?? "default"
```

### Functions

```rust
// String functions
concat($nodes.first, " ", $nodes.last)
substring($nodes.text, 0, 10)
toLowerCase($nodes.input)
toUpperCase($nodes.input)
trim($nodes.text)
split($nodes.csv, ",")
join($nodes.array, ", ")
replace($nodes.text, "old", "new")

// Array functions
length($nodes.items)
first($nodes.array)
last($nodes.array)
slice($nodes.array, 1, 3)
contains($nodes.array, "value")
unique($nodes.array)
sort($nodes.array)
reverse($nodes.array)

// Object functions
keys($nodes.object)
values($nodes.object)
entries($nodes.object)
merge($nodes.obj1, $nodes.obj2)

// Date functions
now()
today()
formatDate($nodes.date, "YYYY-MM-DD")
parseDate($nodes.string, "DD/MM/YYYY")
addDays($nodes.date, 7)
diffDays($nodes.start, $nodes.end)

// Math functions
abs($nodes.number)
round($nodes.float, 2)
floor($nodes.float)
ceil($nodes.float)
min($nodes.a, $nodes.b)
max($nodes.a, $nodes.b)
sum($nodes.array)
avg($nodes.array)

// Type conversion
toString($nodes.number)
toNumber($nodes.string)
toBoolean($nodes.value)
toArray($nodes.value)
toObject($nodes.entries)

// JSON functions
parseJson($nodes.string)
stringifyJson($nodes.object)
jsonPath($nodes.data, "$.users[*].email")
```

### Pipe Operations

```rust
// Data transformation pipeline
$nodes.users.data
  | filter(u => u.active)
  | map(u => { name: u.fullName, email: u.email })
  | sortBy("name")
  | take(10)

// Method chaining
$nodes.text.content
  .trim()
  .toLowerCase()
  .replace(" ", "-")
```

## Implementation

### Parser

```rust
pub struct Parser {
    lexer: Lexer,
    current: Token,
    peek: Token,
}

impl Parser {
    pub fn parse(&mut self, input: &str) -> Result<Expression, ParseError> {
        self.lexer = Lexer::new(input);
        self.advance()?;
        self.advance()?;
        self.parse_expression()
    }
    
    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        self.parse_ternary()
    }
    
    fn parse_ternary(&mut self) -> Result<Expression, ParseError> {
        let mut expr = self.parse_logical_or()?;
        
        if self.match_token(TokenType::Question) {
            let then_expr = Box::new(self.parse_expression()?);
            self.expect(TokenType::Colon)?;
            let else_expr = Box::new(self.parse_expression()?);
            
            expr = Expression::Ternary {
                condition: Box::new(expr),
                then_expr,
                else_expr,
            };
        }
        
        Ok(expr)
    }
}
```

### AST Types

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    // Literals
    Null,
    Boolean(bool),
    Number(f64),
    String(String),
    
    // Variables
    Variable(VariablePath),
    
    // Operations
    Binary {
        op: BinaryOp,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    
    Unary {
        op: UnaryOp,
        expr: Box<Expression>,
    },
    
    // Ternary
    Ternary {
        condition: Box<Expression>,
        then_expr: Box<Expression>,
        else_expr: Box<Expression>,
    },
    
    // Access
    Property {
        object: Box<Expression>,
        property: String,
    },
    
    Index {
        object: Box<Expression>,
        index: Box<Expression>,
    },
    
    // Function call
    Call {
        function: String,
        args: Vec<Expression>,
    },
    
    // Array/Object
    Array(Vec<Expression>),
    Object(Vec<(String, Expression)>),
}
```

### Evaluator

```rust
pub struct Evaluator {
    functions: FunctionRegistry,
    type_coercer: TypeCoercer,
}

impl Evaluator {
    pub async fn eval(
        &self,
        expr: &Expression,
        context: &ExpressionContext,
    ) -> Result<Value, EvalError> {
        match expr {
            Expression::Variable(path) => {
                self.resolve_variable(path, context).await
            }
            
            Expression::Binary { op, left, right } => {
                let left_val = self.eval(left, context).await?;
                let right_val = self.eval(right, context).await?;
                self.apply_binary_op(op, left_val, right_val)
            }
            
            Expression::Call { function, args } => {
                let arg_values = self.eval_args(args, context).await?;
                self.call_function(function, arg_values).await
            }
            
            // ... other cases
        }
    }
}
```

### Function Registry

```rust
pub struct FunctionRegistry {
    functions: HashMap<String, Box<dyn Function>>,
}

#[async_trait]
pub trait Function: Send + Sync {
    fn name(&self) -> &str;
    fn arity(&self) -> Arity;
    fn return_type(&self) -> ValueType;
    async fn call(&self, args: Vec<Value>) -> Result<Value, Error>;
}

pub enum Arity {
    Fixed(usize),
    Range(usize, usize),
    Variadic { min: usize },
}

// Example function implementation
pub struct ConcatFunction;

#[async_trait]
impl Function for ConcatFunction {
    fn name(&self) -> &str { "concat" }
    fn arity(&self) -> Arity { Arity::Variadic { min: 1 } }
    fn return_type(&self) -> ValueType { ValueType::String }
    
    async fn call(&self, args: Vec<Value>) -> Result<Value, Error> {
        let result = args.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("");
        Ok(Value::String(result))
    }
}
```

### Type System

```rust
pub struct TypeChecker {
    type_registry: TypeRegistry,
}

impl TypeChecker {
    pub fn check(
        &self,
        expr: &Expression,
        context: &TypeContext,
    ) -> Result<ValueType, TypeError> {
        match expr {
            Expression::Number(_) => Ok(ValueType::Number),
            Expression::String(_) => Ok(ValueType::String),
            Expression::Boolean(_) => Ok(ValueType::Boolean),
            
            Expression::Binary { op, left, right } => {
                let left_type = self.check(left, context)?;
                let right_type = self.check(right, context)?;
                self.check_binary_op(op, left_type, right_type)
            }
            
            // ... other cases
        }
    }
}
```

### Optimization

```rust
pub struct Optimizer {
    const_folder: ConstantFolder,
    dead_code_eliminator: DeadCodeEliminator,
    common_subexpr_eliminator: CommonSubexpressionEliminator,
}

impl Optimizer {
    pub fn optimize(&self, expr: Expression) -> Expression {
        let expr = self.const_folder.fold(expr);
        let expr = self.dead_code_eliminator.eliminate(expr);
        let expr = self.common_subexpr_eliminator.eliminate(expr);
        expr
    }
}

// Constant folding example
impl ConstantFolder {
    fn fold(&self, expr: Expression) -> Expression {
        match expr {
            Expression::Binary { op: BinaryOp::Add, left, right } => {
                match (left.as_ref(), right.as_ref()) {
                    (Expression::Number(a), Expression::Number(b)) => {
                        Expression::Number(a + b)
                    }
                    _ => Expression::Binary { op: BinaryOp::Add, left, right }
                }
            }
            // ... other cases
        }
    }
}
```

## Error Handling

### Parse Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Unexpected token: {0}")]
    UnexpectedToken(Token),
    
    #[error("Expected {expected}, found {found}")]
    ExpectedToken { expected: TokenType, found: Token },
    
    #[error("Invalid expression: {0}")]
    InvalidExpression(String),
    
    #[error("Unterminated string at position {0}")]
    UnterminatedString(usize),
}
```

### Evaluation Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("Variable not found: {0}")]
    VariableNotFound(String),
    
    #[error("Type mismatch: expected {expected}, got {got}")]
    TypeMismatch { expected: ValueType, got: ValueType },
    
    #[error("Function not found: {0}")]
    FunctionNotFound(String),
    
    #[error("Invalid argument count for {function}: expected {expected}, got {got}")]
    InvalidArity { function: String, expected: String, got: usize },
    
    #[error("Division by zero")]
    DivisionByZero,
    
    #[error("Index out of bounds: {0}")]
    IndexOutOfBounds(usize),
}
```

## Performance

### Caching

```rust
pub struct ExpressionCache {
    parsed: LruCache<String, Expression>,
    compiled: LruCache<String, CompiledExpression>,
}

pub struct CompiledExpression {
    bytecode: Vec<Instruction>,
    constants: Vec<Value>,
}
```

### Benchmarks

```rust
#[bench]
fn bench_simple_expression(b: &mut Bencher) {
    let engine = ExpressionEngine::new();
    let context = create_test_context();
    
    b.iter(|| {
        engine.eval("$nodes.input.value + 10", &context)
    });
}

#[bench]
fn bench_complex_expression(b: &mut Bencher) {
    let engine = ExpressionEngine::new();
    let context = create_test_context();
    
    b.iter(|| {
        engine.eval(r#"
            $nodes.users.list
            | filter(u => u.age >= 18)
            | map(u => u.email)
            | join(", ")
        "#, &context)
    });
}
```

---

