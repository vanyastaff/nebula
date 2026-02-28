# Nebula Complete Documentation - Part 2

---
## FILE: docs/crates/nebula-derive.md
---

# nebula-derive

## Purpose

`nebula-derive` provides procedural macros to reduce boilerplate code when creating nodes and defining parameters. It enables compile-time validation and automatic code generation.

## Responsibilities

- Generate parameter collection code
- Generate action trait implementations  
- Validate attributes at compile time
- Generate serialization/deserialization code
- Create builder patterns automatically

## Architecture

### Macro Types

```rust
// Function-like macros
#[proc_macro]
pub fn node(input: TokenStream) -> TokenStream;

// Derive macros
#[proc_macro_derive(Parameters, attributes(param, validate, display))]
pub fn derive_parameters(input: TokenStream) -> TokenStream;

#[proc_macro_derive(Action, attributes(action, node))]
pub fn derive_action(input: TokenStream) -> TokenStream;

// Attribute macros
#[proc_macro_attribute]
pub fn trigger(args: TokenStream, input: TokenStream) -> TokenStream;
```

### Core Components

```rust
mod parameters;    // Parameter derive implementation
mod action;       // Action derive implementation  
mod validation;   // Compile-time validation
mod utils;        // Shared utilities
mod error;        // Error handling
```

## Derive Macros

### Parameters Derive

```rust
#[derive(Parameters)]
struct MyNodeParams {
    #[param(
        label = "API Key",
        description = "Your API key for authentication",
        required = true,
        secret = true
    )]
    api_key: String,
    
    #[param(
        label = "Timeout",
        description = "Request timeout in seconds",
        default = 30,
        min = 1,
        max = 300
    )]
    timeout: u32,
    
    #[param(
        label = "Retry Count", 
        default = 3,
        display(show_when(field = "advanced_mode", value = true))
    )]
    retry_count: u32,
}

// Generated code:
impl Parameters for MyNodeParams {
    fn parameter_collection() -> ParameterCollection {
        ParameterCollection::new()
            .add(TextParameter {
                key: Key::new("api_key"),
                metadata: ParameterMetadata {
                    label: "API Key",
                    description: Some("Your API key for authentication"),
                    required: true,
                },
                secret: true,
                ..Default::default()
            })
            .add(NumberParameter {
                key: Key::new("timeout"),
                metadata: ParameterMetadata {
                    label: "Timeout",
                    description: Some("Request timeout in seconds"),
                    required: false,
                },
                default: Some(30),
                min: Some(1),
                max: Some(300),
                ..Default::default()
            })
            // ...
    }
    
    fn from_values(values: HashMap<Key, ParameterValue>) -> Result<Self, Error> {
        Ok(Self {
            api_key: values.get(&Key::new("api_key"))
                .ok_or_else(|| Error::MissingRequired("api_key"))?
                .as_string()?,
            timeout: values.get(&Key::new("timeout"))
                .map(|v| v.as_number())
                .transpose()?
                .unwrap_or(30),
            // ...
        })
    }
}
```

### Action Derive

```rust
#[derive(Action)]
#[action(
    id = "http_request",
    name = "HTTP Request",
    category = "Network",
    version = "1.0.0"
)]
struct HttpRequestNode {
    #[parameters]
    params: HttpRequestParams,
}

// Generated code:
impl Action for HttpRequestNode {
    type Input = WorkflowDataItem;
    type Output = WorkflowDataItem;
    type Error = NodeError;
    
    fn metadata(&self) -> ActionMetadata {
        ActionMetadata {
            id: "http_request",
            name: "HTTP Request",
            category: "Network",
            version: Version::parse("1.0.0").unwrap(),
            ..Default::default()
        }
    }
    
    fn parameters(&self) -> &impl Parameters {
        &self.params
    }
}
```

## Validation Attributes

### Field Validation

```rust
#[derive(Parameters)]
struct ValidationExample {
    #[validate(min_length = 3, max_length = 50)]
    #[validate(regex = r"^[a-zA-Z0-9_]+$")]
    username: String,
    
    #[validate(email)]
    email: String,
    
    #[validate(url)]
    webhook_url: String,
    
    #[validate(range = 1..=100)]
    percentage: u8,
    
    #[validate(custom = "validate_api_key")]
    api_key: String,
}

fn validate_api_key(value: &str) -> Result<(), ValidationError> {
    if value.starts_with("sk_") && value.len() == 32 {
        Ok(())
    } else {
        Err(ValidationError::Custom("Invalid API key format"))
    }
}
```

### Cross-field Validation

```rust
#[derive(Parameters)]
#[validate(custom = "validate_dates")]
struct DateRangeParams {
    start_date: DateTime<Utc>,
    
    #[validate(greater_than = "start_date")]
    end_date: DateTime<Utc>,
}

fn validate_dates(params: &DateRangeParams) -> Result<(), ValidationError> {
    if params.end_date <= params.start_date {
        return Err(ValidationError::Custom("End date must be after start date"));
    }
    Ok(())
}
```

## Display Control

### Conditional Display

```rust
#[derive(Parameters)]
struct ConditionalParams {
    #[param(label = "Mode")]
    mode: String,
    
    #[display(show_when(field = "mode", value = "advanced"))]
    advanced_options: AdvancedOptions,
    
    #[display(hide_when(field = "mode", value = "simple"))]
    expert_settings: ExpertSettings,
    
    #[display(show_when(
        condition = "any",
        rules = [
            (field = "mode", value = "custom"),
            (field = "enable_custom", value = true)
        ]
    ))]
    custom_config: String,
}
```

## Special Attributes

### Mode Parameters

```rust
#[derive(Parameters)]
struct ModeExample {
    #[mode(
        text(key = "manual", label = "Manual Input", placeholder = "Enter ID"),
        list(key = "select", label = "Select from List", options_from = "load_options"),
        expression(key = "dynamic", label = "Dynamic Expression")
    )]
    user_selection: String,
}

fn load_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("user1", "John Doe"),
        SelectOption::new("user2", "Jane Smith"),
    ]
}
```

### Array Parameters

```rust
#[derive(Parameters)]
struct ArrayExample {
    #[param(
        type = "array",
        label = "Tags",
        min_items = 1,
        max_items = 10,
        unique = true
    )]
    tags: Vec<String>,
    
    #[param(
        type = "array",
        label = "Headers",
        item_type = "object",
        item_schema = HeaderSchema
    )]
    headers: Vec<Header>,
}
```

## Error Handling

### Compile-time Errors

```rust
// This will fail at compile time
#[derive(Parameters)]
struct InvalidParams {
    #[validate(required)]  // Error: required on Option<T>
    optional_field: Option<String>,
    
    #[param(min = 10, max = 5)]  // Error: min > max
    invalid_range: u32,
}
```

### Error Messages

```rust
// Custom error messages
#[derive(Parameters)]
struct CustomErrors {
    #[validate(
        min_length = 8,
        message = "Password must be at least 8 characters long"
    )]
    password: String,
    
    #[validate(
        custom = "validate_complex",
        message = "Value must match the required format"
    )]
    complex_field: String,
}
```

## Performance Considerations

- All validation code is generated at compile time
- No runtime reflection or dynamic dispatch
- Minimal allocations in generated code
- Efficient parameter collection building

## Testing

### Unit Tests

```rust
#[test]
fn test_parameter_generation() {
    let collection = MyNodeParams::parameter_collection();
    assert_eq!(collection.len(), 3);
    assert!(collection.get("api_key").is_some());
}

#[test]
fn test_from_values() {
    let mut values = HashMap::new();
    values.insert(Key::new("api_key"), ParameterValue::String("test_key".into()));
    
    let params = MyNodeParams::from_values(values).unwrap();
    assert_eq!(params.api_key, "test_key");
    assert_eq!(params.timeout, 30); // default value
}
```

### Compile Tests

```rust
// tests/compile_fail/invalid_required.rs
#[derive(Parameters)]
struct Invalid {
    #[validate(required)]
    //~^ ERROR: 'required' validator cannot be used on Option<T> types
    field: Option<String>,
}
```

---
## FILE: docs/crates/nebula-expression.md
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
## FILE: docs/crates/nebula-engine.md
---

# nebula-engine

## Purpose

`nebula-engine` is the workflow orchestration engine responsible for scheduling, executing, and managing workflow lifecycles. It handles the core execution logic and state management.

## Responsibilities

- Workflow orchestration and scheduling
- Execution state management
- Event processing and routing
- DAG traversal and execution
- Error handling and recovery
- Resource coordination

## Architecture

### Core Components

```rust
pub struct WorkflowEngine {
    // Event processing
    event_bus: Arc<dyn EventBus>,
    
    // State management
    state_manager: Arc<StateManager>,
    
    // Execution scheduling
    scheduler: Arc<Scheduler>,
    
    // DAG processor
    dag_processor: Arc<DagProcessor>,
    
    // Resource coordinator
    resource_coordinator: Arc<ResourceCoordinator>,
    
    // Metrics collector
    metrics: Arc<MetricsCollector>,
}
```

### Event-Driven Architecture

```rust
#[derive(Debug, Clone)]
pub enum EngineEvent {
    // Workflow lifecycle
    WorkflowDeployed { id: WorkflowId, definition: WorkflowDefinition },
    WorkflowActivated { id: WorkflowId },
    WorkflowDeactivated { id: WorkflowId },
    
    // Execution lifecycle
    ExecutionCreated { id: ExecutionId, workflow_id: WorkflowId },
    ExecutionStarted { id: ExecutionId },
    ExecutionCompleted { id: ExecutionId, result: ExecutionResult },
    ExecutionFailed { id: ExecutionId, error: Error },
    ExecutionCancelled { id: ExecutionId },
    
    // Node execution
    NodeReady { execution_id: ExecutionId, node_id: NodeId },
    NodeStarted { execution_id: ExecutionId, node_id: NodeId },
    NodeCompleted { execution_id: ExecutionId, node_id: NodeId, output: Value },
    NodeFailed { execution_id: ExecutionId, node_id: NodeId, error: Error },
    
    // Control flow
    ExecutionSuspended { id: ExecutionId, reason: SuspendReason },
    ExecutionResumed { id: ExecutionId },
}
```

## Execution Flow

### Workflow Deployment

```rust
impl WorkflowEngine {
    pub async fn deploy_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, Error> {
        // Validate workflow
        self.validate_workflow(&definition)?;
        
        // Generate workflow ID
        let workflow_id = WorkflowId::new();
        
        // Store workflow definition
        self.state_manager
            .store_workflow(&workflow_id, &definition)
            .await?;
        
        // Process triggers
        for trigger in &definition.triggers {
            self.activate_trigger(&workflow_id, trigger).await?;
        }
        
        // Emit deployment event
        self.event_bus.publish(EngineEvent::WorkflowDeployed {
            id: workflow_id.clone(),
            definition,
        }).await?;
        
        Ok(workflow_id)
    }
}
```

### Execution Creation

```rust
pub struct ExecutionRequest {
    pub workflow_id: WorkflowId,
    pub input: WorkflowDataItem,
    pub trigger: TriggerInfo,
    pub parent_execution: Option<ExecutionId>,
}

impl WorkflowEngine {
    pub async fn create_execution(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionHandle, Error> {
        // Load workflow
        let workflow = self.state_manager
            .load_workflow(&request.workflow_id)
            .await?;
        
        // Create execution state
        let execution = Execution::new(
            request.workflow_id,
            request.input,
            request.trigger,
        );
        
        // Store initial state
        self.state_manager
            .create_execution(&execution)
            .await?;
        
        // Schedule execution
        self.scheduler
            .schedule_execution(&execution.id)
            .await?;
        
        // Emit creation event
        self.event_bus.publish(EngineEvent::ExecutionCreated {
            id: execution.id.clone(),
            workflow_id: request.workflow_id,
        }).await?;
        
        Ok(ExecutionHandle {
            execution_id: execution.id,
            status_receiver: self.create_status_receiver(&execution.id),
        })
    }
}
```

### DAG Processing

```rust
pub struct DagProcessor {
    graph_analyzer: GraphAnalyzer,
    execution_planner: ExecutionPlanner,
}

impl DagProcessor {
    pub async fn process_workflow(
        &self,
        workflow: &Workflow,
        execution: &Execution,
    ) -> Result<ExecutionPlan, Error> {
        // Analyze graph structure
        let analysis = self.graph_analyzer.analyze(&workflow.graph)?;
        
        // Check for cycles
        if analysis.has_cycles {
            return Err(Error::CyclicWorkflow);
        }
        
        // Create execution plan
        let plan = self.execution_planner.create_plan(
            &workflow.graph,
            &analysis,
            execution,
        )?;
        
        Ok(plan)
    }
}

pub struct ExecutionPlan {
    // Topologically sorted nodes
    pub stages: Vec<ExecutionStage>,
    
    // Parallel execution opportunities
    pub parallelism_map: HashMap<StageId, Vec<NodeId>>,
    
    // Dependencies
    pub dependencies: HashMap<NodeId, Vec<NodeId>>,
    
    // Conditional branches
    pub branches: Vec<ConditionalBranch>,
}
```

### State Management

```rust
pub struct StateManager {
    // Workflow definitions
    workflows: Arc<RwLock<HashMap<WorkflowId, WorkflowDefinition>>>,
    
    // Execution states
    executions: Arc<RwLock<HashMap<ExecutionId, ExecutionState>>>,
    
    // Persistence layer
    storage: Arc<dyn StorageBackend>,
    
    // State snapshots
    snapshots: Arc<SnapshotManager>,
}

#[derive(Debug, Clone)]
pub struct ExecutionState {
    pub id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub status: ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub current_nodes: HashSet<NodeId>,
    pub completed_nodes: HashSet<NodeId>,
    pub node_outputs: HashMap<NodeId, WorkflowDataItem>,
    pub variables: HashMap<String, Value>,
    pub error: Option<ExecutionError>,
}

impl StateManager {
    pub async fn update_execution_state<F>(
        &self,
        execution_id: &ExecutionId,
        updater: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut ExecutionState) -> Result<(), Error>,
    {
        let mut executions = self.executions.write().await;
        
        let state = executions
            .get_mut(execution_id)
            .ok_or(Error::ExecutionNotFound)?;
        
        // Apply update
        updater(state)?;
        
        // Persist changes
        self.storage
            .save_execution_state(execution_id, state)
            .await?;
        
        Ok(())
    }
}
```

### Scheduler

```rust
pub struct Scheduler {
    // Work queue
    work_queue: Arc<WorkQueue>,
    
    // Scheduling strategy
    strategy: Box<dyn SchedulingStrategy>,
    
    // Worker pool
    worker_pool: Arc<WorkerPool>,
    
    // Load balancer
    load_balancer: Arc<LoadBalancer>,
}

#[async_trait]
pub trait SchedulingStrategy: Send + Sync {
    async fn select_worker(
        &self,
        task: &Task,
        workers: &[WorkerInfo],
    ) -> Result<WorkerId, Error>;
    
    async fn prioritize_tasks(
        &self,
        tasks: Vec<Task>,
    ) -> Vec<Task>;
}

pub struct PrioritySchedulingStrategy {
    priority_calculator: Box<dyn PriorityCalculator>,
}

impl Scheduler {
    pub async fn schedule_node(
        &self,
        execution_id: &ExecutionId,
        node_id: &NodeId,
    ) -> Result<(), Error> {
        // Create task
        let task = Task {
            id: TaskId::new(),
            execution_id: execution_id.clone(),
            node_id: node_id.clone(),
            priority: self.calculate_priority(execution_id, node_id).await?,
            created_at: Utc::now(),
        };
        
        // Add to queue
        self.work_queue.push(task).await?;
        
        // Notify workers
        self.worker_pool.notify_available_work().await?;
        
        Ok(())
    }
}
```

## Error Handling

### Error Recovery

```rust
pub struct ErrorHandler {
    retry_policy: RetryPolicy,
    fallback_manager: FallbackManager,
    compensation_engine: CompensationEngine,
}

pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_strategy: BackoffStrategy,
    pub retryable_errors: HashSet<ErrorType>,
}

pub enum BackoffStrategy {
    Fixed { delay: Duration },
    Exponential { base: Duration, factor: f64, max: Duration },
    Linear { increment: Duration },
}

impl ErrorHandler {
    pub async fn handle_node_error(
        &self,
        execution_id: &ExecutionId,
        node_id: &NodeId,
        error: Error,
    ) -> Result<ErrorRecovery, Error> {
        // Check if retryable
        if self.retry_policy.is_retryable(&error) {
            let attempt = self.get_retry_attempt(execution_id, node_id).await?;
            
            if attempt < self.retry_policy.max_attempts {
                let delay = self.retry_policy.calculate_backoff(attempt);
                
                return Ok(ErrorRecovery::Retry {
                    delay,
                    attempt: attempt + 1,
                });
            }
        }
        
        // Check for fallback
        if let Some(fallback) = self.fallback_manager.get_fallback(node_id).await? {
            return Ok(ErrorRecovery::Fallback { node_id: fallback });
        }
        
        // Check for compensation
        if let Some(compensation) = self.compensation_engine.get_compensation(execution_id).await? {
            return Ok(ErrorRecovery::Compensate { 
                workflow_id: compensation,
            });
        }
        
        // No recovery available
        Ok(ErrorRecovery::Fail)
    }
}
```

### Compensation Logic

```rust
pub struct CompensationEngine {
    saga_definitions: HashMap<WorkflowId, SagaDefinition>,
}

pub struct SagaDefinition {
    pub steps: Vec<SagaStep>,
    pub compensation_strategy: CompensationStrategy,
}

pub struct SagaStep {
    pub forward_action: NodeId,
    pub compensating_action: Option<NodeId>,
}

impl CompensationEngine {
    pub async fn compensate(
        &self,
        execution_id: &ExecutionId,
        failed_at: &NodeId,
    ) -> Result<(), Error> {
        let execution = self.load_execution(execution_id).await?;
        let saga = self.saga_definitions.get(&execution.workflow_id)
            .ok_or(Error::NoSagaDefined)?;
        
        // Find completed steps that need compensation
        let steps_to_compensate = self.find_steps_to_compensate(
            &execution,
            failed_at,
            saga,
        )?;
        
        // Execute compensation in reverse order
        for step in steps_to_compensate.iter().rev() {
            if let Some(compensating_action) = &step.compensating_action {
                self.execute_compensation(
                    execution_id,
                    compensating_action,
                ).await?;
            }
        }
        
        Ok(())
    }
}
```

## Performance Optimization

### Execution Cache

```rust
pub struct ExecutionCache {
    // Hot executions in memory
    hot_cache: Arc<RwLock<LruCache<ExecutionId, ExecutionState>>>,
    
    // Warm executions in Redis
    warm_cache: Arc<RedisCache>,
    
    // Metrics
    metrics: Arc<CacheMetrics>,
}

impl ExecutionCache {
    pub async fn get(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, Error> {
        // Check hot cache
        if let Some(state) = self.hot_cache.read().await.get(id) {
            self.metrics.record_hit(CacheLevel::Hot);
            return Ok(Some(state.clone()));
        }
        
        // Check warm cache
        if let Some(state) = self.warm_cache.get(id).await? {
            self.metrics.record_hit(CacheLevel::Warm);
            
            // Promote to hot cache
            self.hot_cache.write().await.put(id.clone(), state.clone());
            
            return Ok(Some(state));
        }
        
        self.metrics.record_miss();
        Ok(None)
    }
}
```

### Parallel Execution

```rust
pub struct ParallelExecutor {
    concurrency_limit: usize,
    semaphore: Arc<Semaphore>,
}

impl ParallelExecutor {
    pub async fn execute_parallel_nodes(
        &self,
        nodes: Vec<NodeId>,
        execution_context: &ExecutionContext,
    ) -> Result<Vec<(NodeId, Result<WorkflowDataItem, Error>)>, Error> {
        let mut handles = Vec::new();
        
        for node_id in nodes {
            let permit = self.semaphore.acquire().await?;
            let context = execution_context.clone();
            
            let handle = tokio::spawn(async move {
                let result = execute_node(&node_id, &context).await;
                drop(permit); // Release semaphore
                (node_id, result)
            });
            
            handles.push(handle);
        }
        
        // Wait for all to complete
        let results = futures::future::join_all(handles).await;
        
        results.into_iter()
            .map(|r| r.map_err(Error::from))
            .collect()
    }
}
```

## Monitoring

### Metrics Collection

```rust
pub struct EngineMetrics {
    // Workflow metrics
    workflows_deployed: Counter,
    workflows_active: Gauge,
    
    // Execution metrics
    executions_created: Counter,
    executions_completed: Counter,
    executions_failed: Counter,
    execution_duration: Histogram,
    
    // Node metrics
    nodes_executed: Counter,
    node_duration: Histogram,
    node_errors: Counter,
    
    // Queue metrics
    queue_depth: Gauge,
    queue_wait_time: Histogram,
}

impl EngineMetrics {
    pub fn record_execution_completed(&self, duration: Duration) {
        self.executions_completed.increment();
        self.execution_duration.record(duration.as_secs_f64());
    }
    
    pub fn record_node_executed(&self, node_type: &str, duration: Duration) {
        self.nodes_executed
            .with_label_values(&[node_type])
            .increment();
        
        self.node_duration
            .with_label_values(&[node_type])
            .record(duration.as_secs_f64());
    }
}
```

### Health Checks

```rust
pub struct EngineHealth {
    components: Vec<Box<dyn HealthCheck>>,
}

#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn check(&self) -> HealthStatus;
    fn component_name(&self) -> &str;
}

pub struct HealthStatus {
    pub status: Status,
    pub message: Option<String>,
    pub details: HashMap<String, Value>,
}

pub enum Status {
    Healthy,
    Degraded,
    Unhealthy,
}

impl EngineHealth {
    pub async fn check_health(&self) -> OverallHealth {
        let mut results = HashMap::new();
        let mut overall_status = Status::Healthy;
        
        for component in &self.components {
            let status = component.check().await;
            
            match status.status {
                Status::Unhealthy => overall_status = Status::Unhealthy,
                Status::Degraded if overall_status == Status::Healthy => {
                    overall_status = Status::Degraded;
                }
                _ => {}
            }
            
            results.insert(component.component_name().to_string(), status);
        }
        
        OverallHealth {
            status: overall_status,
            components: results,
            timestamp: Utc::now(),
        }
    }
}
```