# nebula-sdk

## Purpose

`nebula-sdk` provides a comprehensive toolkit for developers building custom nodes and integrations with Nebula, offering a streamlined development experience with built-in best practices.

## Responsibilities

- Simplified node creation
- Type-safe builders
- Testing utilities
- Development tools
- Code generation
- Documentation generation

## Architecture

### Core Components

```rust
pub struct NebulaSDK {
    // Node development
    node_builder: NodeBuilder,
    
    // Testing framework
    test_framework: TestFramework,
    
    // Code generation
    code_generator: CodeGenerator,
    
    // Development server
    dev_server: DevServer,
    
    // Documentation generator
    doc_generator: DocGenerator,
}

// Main entry point for developers
pub mod prelude {
    pub use nebula_core::prelude::*;
    pub use serde_json::Value;
    pub use nebula_macros::{node, action};
    
    pub use crate::builders::{NodeBuilder, TriggerBuilder};
    pub use crate::testing::{TestContext, MockExecution};
    pub use crate::helpers::*;
}
```

### Node Builder API

```rust
pub struct NodeBuilder {
    metadata: NodeMetadataBuilder,
    parameters: Vec<ParameterBuilder>,
    implementation: Option<Box<dyn Action>>,
}

impl NodeBuilder {
    pub fn new(id: &str) -> Self {
        Self {
            metadata: NodeMetadataBuilder::new(id),
            parameters: Vec::new(),
            implementation: None,
        }
    }
    
    pub fn name(mut self, name: &str) -> Self {
        self.metadata.name(name);
        self
    }
    
    pub fn description(mut self, desc: &str) -> Self {
        self.metadata.description(desc);
        self
    }
    
    pub fn category(mut self, category: NodeCategory) -> Self {
        self.metadata.category(category);
        self
    }
    
    pub fn parameter(mut self, param: ParameterBuilder) -> Self {
        self.parameters.push(param);
        self
    }
    
    pub fn implementation<T: Action + 'static>(mut self, action: T) -> Self {
        self.implementation = Some(Box::new(action));
        self
    }
    
    pub fn build(self) -> Result<BuiltNode, Error> {
        let metadata = self.metadata.build()?;
        
        let node = BuiltNode {
            metadata,
            parameters: self.parameters
                .into_iter()
                .map(|p| p.build())
                .collect::<Result<Vec<_>, _>>()?,
            implementation: self.implementation
                .ok_or(Error::NoImplementation)?,
        };
        
        node.validate()?;
        
        Ok(node)
    }
}

// Fluent parameter builder
pub struct ParameterBuilder {
    descriptor: ParameterDescriptor,
}

impl ParameterBuilder {
    pub fn string(name: &str) -> Self {
        Self {
            descriptor: ParameterDescriptor {
                name: name.to_string(),
                parameter_type: ParameterType::String,
                required: true,
                default_value: None,
                description: None,
                validation: None,
            },
        }
    }
    
    pub fn integer(name: &str) -> Self {
        Self {
            descriptor: ParameterDescriptor {
                name: name.to_string(),
                parameter_type: ParameterType::Integer,
                required: true,
                default_value: None,
                description: None,
                validation: None,
            },
        }
    }
    
    pub fn optional(mut self) -> Self {
        self.descriptor.required = false;
        self
    }
    
    pub fn default<V: Into<Value>>(mut self, value: V) -> Self {
        self.descriptor.default_value = Some(value.into());
        self
    }
    
    pub fn description(mut self, desc: &str) -> Self {
        self.descriptor.description = Some(desc.to_string());
        self
    }
    
    pub fn validate<F>(mut self, validator: F) -> Self
    where
        F: Fn(&Value) -> Result<(), String> + 'static,
    {
        self.descriptor.validation = Some(Box::new(validator));
        self
    }
}
```

### Simplified Node Creation

```rust
// Using derive macros
#[derive(Node)]
#[node(
    id = "transform_text",
    name = "Transform Text",
    category = "Text Processing"
)]
pub struct TransformTextNode {
    #[parameter(description = "Input text to transform")]
    input: String,
    
    #[parameter(
        description = "Transformation to apply",
        default = "uppercase"
    )]
    operation: TransformOperation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransformOperation {
    Uppercase,
    Lowercase,
    Capitalize,
    Reverse,
}

#[async_trait]
impl Action for TransformTextNode {
    async fn execute(&self, ctx: &ExecutionContext) -> Result<Value, Error> {
        let result = match self.operation {
            TransformOperation::Uppercase => self.input.to_uppercase(),
            TransformOperation::Lowercase => self.input.to_lowercase(),
            TransformOperation::Capitalize => capitalize(&self.input),
            TransformOperation::Reverse => self.input.chars().rev().collect(),
        };
        
        Ok(Value::String(result))
    }
}

// Using builder API
pub fn create_http_node() -> Result<BuiltNode, Error> {
    NodeBuilder::new("http_request")
        .name("HTTP Request")
        .description("Make HTTP requests")
        .category(NodeCategory::Network)
        .parameter(
            ParameterBuilder::string("url")
                .description("Target URL")
                .validate(|v| {
                    if let Value::String(s) = v {
                        Url::parse(s).map_err(|e| e.to_string())?;
                        Ok(())
                    } else {
                        Err("URL must be a string".to_string())
                    }
                })
        )
        .parameter(
            ParameterBuilder::string("method")
                .description("HTTP method")
                .default("GET")
                .validate(|v| {
                    if let Value::String(s) = v {
                        match s.as_str() {
                            "GET" | "POST" | "PUT" | "DELETE" | "PATCH" => Ok(()),
                            _ => Err("Invalid HTTP method".to_string()),
                        }
                    } else {
                        Err("Method must be a string".to_string())
                    }
                })
        )
        .parameter(
            ParameterBuilder::object("headers")
                .description("Request headers")
                .optional()
        )
        .parameter(
            ParameterBuilder::any("body")
                .description("Request body")
                .optional()
        )
        .implementation(HttpRequestNode)
        .build()
}
```

### Testing Framework

```rust
pub struct TestFramework {
    runner: TestRunner,
    mock_factory: MockFactory,
}

pub struct TestContext {
    execution_id: ExecutionId,
    workflow_id: WorkflowId,
    variables: HashMap<String, Value>,
    resources: HashMap<String, Box<dyn Any>>,
    progress_receiver: mpsc::Receiver<Progress>,
}

impl TestContext {
    pub fn new() -> Self {
        Self {
            execution_id: ExecutionId::new(),
            workflow_id: WorkflowId::new(),
            variables: HashMap::new(),
            resources: HashMap::new(),
            progress_receiver: mpsc::channel(100).1,
        }
    }
    
    pub fn with_variable(mut self, name: &str, value: Value) -> Self {
        self.variables.insert(name.to_string(), value);
        self
    }
    
    pub fn with_mock<T: 'static>(mut self, mock: T) -> Self {
        self.resources.insert(
            std::any::type_name::<T>().to_string(),
            Box::new(mock),
        );
        self
    }
}

// Test utilities
#[macro_export]
macro_rules! assert_node_output {
    ($node:expr, $input:expr, $expected:expr) => {
        let ctx = TestContext::new();
        let result = $node.execute($input, &ctx).await?;
        assert_eq!(result, $expected);
    };
}

#[macro_export]
macro_rules! test_node {
    ($name:ident, $node:expr, $($test_name:ident: $input:expr => $expected:expr),* $(,)?) => {
        #[cfg(test)]
        mod $name {
            use super::*;
            
            $(
                #[tokio::test]
                async fn $test_name() -> Result<(), Box<dyn std::error::Error>> {
                    assert_node_output!($node, $input, $expected);
                    Ok(())
                }
            )*
        }
    };
}

// Example test
test_node!(
    transform_text_tests,
    TransformTextNode::new(),
    uppercase_test: "hello" => "HELLO",
    lowercase_test: "WORLD" => "world",
    capitalize_test: "rust lang" => "Rust Lang",
);

// Integration testing
pub struct MockExecution {
    nodes: HashMap<NodeId, Box<dyn Action>>,
    connections: Vec<Connection>,
    test_data: HashMap<String, Value>,
}

impl MockExecution {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            connections: Vec::new(),
            test_data: HashMap::new(),
        }
    }
    
    pub fn add_node(mut self, id: &str, node: impl Action + 'static) -> Self {
        self.nodes.insert(NodeId::from(id), Box::new(node));
        self
    }
    
    pub fn connect(mut self, from: &str, to: &str) -> Self {
        self.connections.push(Connection {
            from: NodeId::from(from),
            to: NodeId::from(to),
            from_output: "output".to_string(),
            to_input: "input".to_string(),
        });
        self
    }
    
    pub fn with_input(mut self, data: Value) -> Self {
        self.test_data.insert("input".to_string(), data);
        self
    }
    
    pub async fn execute(&self) -> Result<ExecutionResult, Error> {
        let mut results = HashMap::new();
        let ctx = TestContext::new();
        
        // Simple topological execution
        for (id, node) in &self.nodes {
            let input = self.test_data.get("input").cloned()
                .unwrap_or(Value::Null);
                
            let output = node.execute(input, &ctx).await?;
            results.insert(id.clone(), output);
        }
        
        Ok(ExecutionResult { results })
    }
}
```

### Development Server

```rust
pub struct DevServer {
    config: DevServerConfig,
    node_registry: Arc<NodeRegistry>,
    file_watcher: FileWatcher,
}

pub struct DevServerConfig {
    pub port: u16,
    pub hot_reload: bool,
    pub watch_paths: Vec<PathBuf>,
    pub auto_document: bool,
}

impl DevServer {
    pub async fn start(&self) -> Result<(), Error> {
        info!("Starting development server on port {}", self.config.port);
        
        // Start file watcher for hot reload
        if self.config.hot_reload {
            self.start_file_watcher().await?;
        }
        
        // Start HTTP server
        let app = Router::new()
            .route("/", get(self.dashboard))
            .route("/api/nodes", get(self.list_nodes))
            .route("/api/nodes/:id/test", post(self.test_node))
            .route("/api/nodes/:id/docs", get(self.node_docs))
            .route("/playground", get(self.playground))
            .route("/ws", get(self.websocket_handler));
            
        Server::bind(&([127, 0, 0, 1], self.config.port).into())
            .serve(app.into_make_service())
            .await?;
            
        Ok(())
    }
    
    async fn start_file_watcher(&self) -> Result<(), Error> {
        let (tx, rx) = mpsc::channel();
        
        let mut watcher = notify::watcher(tx, Duration::from_secs(1))?;
        
        for path in &self.config.watch_paths {
            watcher.watch(path, RecursiveMode::Recursive)?;
        }
        
        tokio::spawn(async move {
            while let Ok(event) = rx.recv() {
                match event {
                    DebouncedEvent::Write(path) | 
                    DebouncedEvent::Create(path) => {
                        if path.extension() == Some("rs".as_ref()) {
                            info!("Detected change in {:?}, reloading...", path);
                            // Trigger rebuild and reload
                            self.reload_nodes().await?;
                        }
                    }
                    _ => {}
                }
            }
        });
        
        Ok(())
    }
}
```

### Code Generation

```rust
pub struct CodeGenerator {
    templates: TemplateEngine,
    analyzer: CodeAnalyzer,
}

impl CodeGenerator {
    pub fn generate_node_template(
        &self,
        config: NodeTemplateConfig,
    ) -> Result<String, Error> {
        let template = match config.template_type {
            TemplateType::Basic => include_str!("templates/basic_node.rs"),
            TemplateType::Trigger => include_str!("templates/trigger_node.rs"),
            TemplateType::Transform => include_str!("templates/transform_node.rs"),
            TemplateType::Integration => include_str!("templates/integration_node.rs"),
        };
        
        self.templates.render(template, &config)
    }
    
    pub fn generate_from_openapi(
        &self,
        spec: OpenApiSpec,
        config: OpenApiConfig,
    ) -> Result<GeneratedCode, Error> {
        let mut nodes = Vec::new();
        
        for (path, item) in spec.paths {
            for (method, operation) in item.operations() {
                let node = self.generate_api_node(
                    &path,
                    method,
                    operation,
                    &config,
                )?;
                nodes.push(node);
            }
        }
        
        Ok(GeneratedCode {
            nodes,
            common: self.generate_common_code(&spec, &config)?,
        })
    }
    
    pub fn generate_workflow_types(
        &self,
        workflow: &Workflow,
    ) -> Result<String, Error> {
        let mut code = String::new();
        
        // Generate input/output types
        writeln!(code, "// Auto-generated workflow types")?;
        writeln!(code, "#[derive(Debug, Clone, Serialize, Deserialize)]")?;
        writeln!(code, "pub struct {}Input {{", workflow.name)?;
        
        for param in &workflow.parameters {
            writeln!(
                code,
                "    pub {}: {},",
                param.name,
                self.rust_type_for_value_type(&param.value_type)
            )?;
        }
        
        writeln!(code, "}}")?;
        
        Ok(code)
    }
}
```

### Documentation Generator

```rust
pub struct DocGenerator {
    markdown_renderer: MarkdownRenderer,
    example_extractor: ExampleExtractor,
}

impl DocGenerator {
    pub fn generate_node_docs(
        &self,
        node: &dyn Action,
    ) -> Result<NodeDocumentation, Error> {
        let metadata = node.metadata();
        let examples = self.example_extractor.extract_examples(node)?;
        
        let docs = NodeDocumentation {
            id: metadata.id.clone(),
            name: metadata.name.clone(),
            description: metadata.description.clone(),
            category: metadata.category.clone(),
            parameters: self.document_parameters(&metadata.parameters),
            outputs: self.document_outputs(&metadata.outputs),
            examples,
            errors: self.document_errors(node),
            see_also: self.find_related_nodes(&metadata),
        };
        
        Ok(docs)
    }
    
    pub fn generate_workflow_docs(
        &self,
        workflow: &Workflow,
    ) -> Result<String, Error> {
        let mut md = String::new();
        
        writeln!(md, "# {}", workflow.name)?;
        writeln!(md, "\n{}", workflow.description)?;
        
        writeln!(md, "\n## Parameters\n")?;
        for param in &workflow.parameters {
            writeln!(
                md,
                "- **{}** ({}) - {}",
                param.name,
                param.value_type,
                param.description
            )?;
        }
        
        writeln!(md, "\n## Nodes\n")?;
        for node in &workflow.nodes {
            writeln!(
                md,
                "### {} ({})",
                node.name,
                node.node_type
            )?;
            writeln!(md, "\n{}", node.description)?;
        }
        
        writeln!(md, "\n## Flow\n")?;
        writeln!(md, "```mermaid")?;
        writeln!(md, "{}", self.generate_mermaid_diagram(workflow)?)?;
        writeln!(md, "```")?;
        
        Ok(md)
    }
}
```

### CLI Tools

```rust
pub struct NebulaCliTool {
    commands: HashMap<String, Box<dyn Command>>,
}

#[async_trait]
pub trait Command: Send + Sync {
    async fn execute(&self, args: &ArgMatches) -> Result<(), Error>;
    fn name(&self) -> &str;
    fn about(&self) -> &str;
    fn args(&self) -> Vec<Arg>;
}

pub struct InitCommand;

#[async_trait]
impl Command for InitCommand {
    async fn execute(&self, args: &ArgMatches) -> Result<(), Error> {
        let project_name = args.value_of("name").unwrap_or("my-nebula-nodes");
        let template = args.value_of("template").unwrap_or("basic");
        
        println!("Creating new Nebula node project: {}", project_name);
        
        // Create project structure
        let project_dir = PathBuf::from(project_name);
        fs::create_dir_all(&project_dir).await?;
        
        // Generate Cargo.toml
        let cargo_toml = format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
nebula-sdk = "0.1"
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
async-trait = "0.1"

[lib]
crate-type = ["cdylib", "rlib"]

[profile.release]
lto = true
"#,
            project_name
        );
        
        fs::write(project_dir.join("Cargo.toml"), cargo_toml).await?;
        
        // Generate src/lib.rs
        let lib_rs = match template {
            "trigger" => include_str!("templates/init/trigger.rs"),
            "transform" => include_str!("templates/init/transform.rs"),
            _ => include_str!("templates/init/basic.rs"),
        };
        
        let src_dir = project_dir.join("src");
        fs::create_dir_all(&src_dir).await?;
        fs::write(src_dir.join("lib.rs"), lib_rs).await?;
        
        // Generate example
        let examples_dir = project_dir.join("examples");
        fs::create_dir_all(&examples_dir).await?;
        fs::write(
            examples_dir.join("basic.rs"),
            include_str!("templates/init/example.rs")
        ).await?;
        
        println!("✨ Project created successfully!");
        println!("\nNext steps:");
        println!("  cd {}", project_name);
        println!("  cargo build");
        println!("  cargo run --example basic");
        
        Ok(())
    }
    
    fn name(&self) -> &str { "init" }
    fn about(&self) -> &str { "Create a new Nebula node project" }
    fn args(&self) -> Vec<Arg> {
        vec![
            Arg::with_name("name")
                .help("Project name")
                .required(true),
            Arg::with_name("template")
                .help("Project template")
                .long("template")
                .takes_value(true)
                .possible_values(&["basic", "trigger", "transform"]),
        ]
    }
}
```

## SDK Features

### 1. Type-Safe Builders
- Fluent API for node construction
- Compile-time parameter validation
- Auto-completion friendly

### 2. Testing Utilities
- Unit test helpers
- Integration test framework
- Mock execution environment
- Property-based testing support

### 3. Development Tools
- Hot reload development server
- Interactive playground
- Performance profiling
- Debug visualizations

### 4. Code Generation
- Node templates
- OpenAPI to nodes
- Workflow type generation
- Documentation generation

### 5. Best Practices
- Built-in error handling patterns
- Resource management helpers
- Logging and tracing integration
- Metrics collection

## Usage Examples

### Creating a Simple Node

```rust
use nebula_sdk::prelude::*;

#[derive(Node)]
#[node(id = "delay", name = "Delay", category = "Utility")]
pub struct DelayNode {
    #[parameter(description = "Delay duration in milliseconds")]
    duration_ms: u64,
}

#[async_trait]
impl Action for DelayNode {
    async fn execute(&self, input: Value, ctx: &ExecutionContext) -> Result<Value, Error> {
        tokio::time::sleep(Duration::from_millis(self.duration_ms)).await;
        Ok(input)
    }
}
```

### Creating a Trigger Node

```rust
use nebula_sdk::prelude::*;

#[derive(Trigger)]
#[trigger(
    id = "cron_trigger",
    name = "Cron Schedule",
    category = "Scheduling"
)]
pub struct CronTrigger {
    #[parameter(description = "Cron expression")]
    schedule: String,
    
    #[parameter(description = "Timezone", default = "UTC")]
    timezone: String,
}

#[async_trait]
impl TriggerAction for CronTrigger {
    async fn start(&self, ctx: &TriggerContext) -> Result<(), Error> {
        let schedule = Schedule::from_str(&self.schedule)?;
        let tz: Tz = self.timezone.parse()?;
        
        loop {
            let next = schedule.upcoming(tz).next().unwrap();
            let until_next = (next - Utc::now()).to_std()?;
            
            tokio::time::sleep(until_next).await;
            
            ctx.emit(json!({
                "triggered_at": Utc::now(),
                "schedule": self.schedule,
            })).await?;
        }
    }
}
```

### Testing a Node

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_sdk::testing::*;
    
    #[tokio::test]
    async fn test_delay_node() {
        let node = DelayNode { duration_ms: 100 };
        let ctx = TestContext::new();
        
        let start = Instant::now();
        let result = node.execute(json!("test"), &ctx).await.unwrap();
        let elapsed = start.elapsed();
        
        assert!(elapsed >= Duration::from_millis(100));
        assert_eq!(result, json!("test"));
    }
    
    #[tokio::test]
    async fn test_workflow_integration() {
        let execution = MockExecution::new()
            .add_node("input", InputNode::new())
            .add_node("transform", TransformTextNode::new())
            .add_node("output", OutputNode::new())
            .connect("input", "transform")
            .connect("transform", "output")
            .with_input(json!({ "text": "hello world" }));
            
        let result = execution.execute().await.unwrap();
        
        assert_eq!(
            result.get_output("output"),
            Some(&json!({ "text": "HELLO WORLD" }))
        );
    }
}
```

## Performance Optimization

### Best Practices
1. Use value references when possible
2. Implement streaming for large data
3. Pool expensive resources
4. Use async operations efficiently

### Profiling Tools
```rust
use nebula_sdk::profiling::*;

#[profile]
async fn expensive_operation() {
    // Automatically tracked
}

// Manual profiling
let _guard = profile_scope!("custom_operation");
```

## Distribution

### Publishing Nodes
1. Package as standard Rust crate
2. Include in crates.io or git
3. Document with examples
4. Version appropriately

### Binary Distribution
```bash
# Build optimized binary
nebula-sdk build --release

# Package with manifest
nebula-sdk package --output my-nodes.nbp
```