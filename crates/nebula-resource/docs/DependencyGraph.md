---

## title: Dependency Graph
tags: [nebula-resource, how-to, dependencies, graph]
status: stable
created: 2025-08-17
---

# Dependency Graph

Guide to managing complex resource dependencies with automatic resolution, circular dependency detection, and initialization ordering.

## Overview

Resources often depend on other resources. This system provides:

- Automatic dependency resolution
- Circular dependency detection
- Topological initialization ordering
- Lazy loading with dependency injection
- Cascade failure handling

## Basic Dependencies

### Step 1: Declare Dependencies

```rust
use nebula_resource::prelude::*;
use nebula_resource::dependencies::*;

#[derive(Resource)]
#[resource(
    id = "email_service",
    name = "Email Service",
    // Declare what this resource depends on
    depends_on = ["smtp_client", "template_engine", "logger"]
)]
pub struct EmailServiceResource;

#[derive(Resource)]
#[resource(
    id = "smtp_client",
    name = "SMTP Client",
    depends_on = ["logger"]  // SMTP needs logger
)]
pub struct SmtpClientResource;

#[derive(Resource)]
#[resource(
    id = "template_engine", 
    name = "Template Engine",
    depends_on = ["logger", "cache"]  // Template engine needs logger and cache
)]
pub struct TemplateEngineResource;

#[derive(Resource)]
#[resource(
    id = "cache",
    name = "Cache Service",
    depends_on = []  // No dependencies
)]
pub struct CacheResource;

#[derive(Resource)]
#[resource(
    id = "logger",
    name = "Logger",
    depends_on = []  // No dependencies
)]
pub struct LoggerResource;
```

### Step 2: Dependency Graph Construction

```rust
/// Manages resource dependencies
pub struct DependencyGraph {
    nodes: HashMap<String, ResourceNode>,
    edges: HashMap<String, HashSet<String>>,
    reverse_edges: HashMap<String, HashSet<String>>,
}

#[derive(Clone)]
pub struct ResourceNode {
    pub id: String,
    pub resource_type: String,
    pub dependencies: Vec<String>,
    pub metadata: ResourceMetadata,
    pub state: ResourceState,
}

#[derive(Clone, Debug)]
pub enum ResourceState {
    NotInitialized,
    Initializing,
    Ready,
    Failed(String),
    Shutdown,
}

impl DependencyGraph {
    /// Add resource to dependency graph
    pub fn add_resource<R: Resource>(&mut self) -> Result<(), DependencyError> {
        let metadata = R::metadata();
        let dependencies = R::dependencies();
        
        // Check for self-dependency
        if dependencies.contains(&metadata.id) {
            return Err(DependencyError::SelfDependency(metadata.id));
        }
        
        // Add node
        let node = ResourceNode {
            id: metadata.id.clone(),
            resource_type: std::any::type_name::<R>().to_string(),
            dependencies: dependencies.clone(),
            metadata: metadata.clone(),
            state: ResourceState::NotInitialized,
        };
        
        self.nodes.insert(metadata.id.clone(), node);
        
        // Add edges
        for dep in dependencies {
            self.edges
                .entry(metadata.id.clone())
                .or_default()
                .insert(dep.clone());
            
            self.reverse_edges
                .entry(dep)
                .or_default()
                .insert(metadata.id.clone());
        }
        
        // Check for circular dependencies
        if self.has_circular_dependency()? {
            self.nodes.remove(&metadata.id);
            return Err(DependencyError::CircularDependency(
                self.find_circular_path()
            ));
        }
        
        Ok(())
    }
    
    /// Get initialization order using topological sort
    pub fn get_initialization_order(&self) -> Result<Vec<String>, DependencyError> {
        let mut visited = HashSet::new();
        let mut stack = Vec::new();
        let mut temp_visited = HashSet::new();
        
        for node_id in self.nodes.keys() {
            if !visited.contains(node_id) {
                self.topological_sort_dfs(
                    node_id,
                    &mut visited,
                    &mut temp_visited,
                    &mut stack,
                )?;
            }
        }
        
        stack.reverse();
        Ok(stack)
    }
    
    fn topological_sort_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        temp_visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), DependencyError> {
        if temp_visited.contains(node) {
            return Err(DependencyError::CircularDependency(vec![node.to_string()]));
        }
        
        if visited.contains(node) {
            return Ok(());
        }
        
        temp_visited.insert(node.to_string());
        
        if let Some(deps) = self.edges.get(node) {
            for dep in deps {
                self.topological_sort_dfs(dep, visited, temp_visited, stack)?;
            }
        }
        
        temp_visited.remove(node);
        visited.insert(node.to_string());
        stack.push(node.to_string());
        
        Ok(())
    }
}
```

## Advanced Dependency Management

### Lazy Loading with Dependency Injection

```rust
/// Dependency injection container
pub struct ResourceContainer {
    graph: Arc<RwLock<DependencyGraph>>,
    instances: Arc<DashMap<String, Arc<dyn Any + Send + Sync>>>,
    factories: Arc<DashMap<String, Box<dyn ResourceFactory>>>,
    initialization_lock: Arc<Mutex<()>>,
}

#[async_trait]
pub trait ResourceFactory: Send + Sync {
    async fn create(
        &self,
        container: &ResourceContainer,
        context: &ResourceContext,
    ) -> Result<Arc<dyn Any + Send + Sync>, ResourceError>;
    
    fn resource_id(&self) -> &str;
    fn dependencies(&self) -> Vec<String>;
}

impl ResourceContainer {
    /// Get or create resource with automatic dependency resolution
    pub async fn get<T: Resource + 'static>(&self) -> Result<Arc<T::Instance>, ResourceError> {
        let resource_id = T::metadata().id;
        
        // Fast path: already initialized
        if let Some(instance) = self.instances.get(&resource_id) {
            return instance
                .downcast_ref::<Arc<T::Instance>>()
                .cloned()
                .ok_or_else(|| ResourceError::TypeMismatch(resource_id));
        }
        
        // Slow path: need to initialize
        let _lock = self.initialization_lock.lock().await;
        
        // Double-check after acquiring lock
        if let Some(instance) = self.instances.get(&resource_id) {
            return instance
                .downcast_ref::<Arc<T::Instance>>()
                .cloned()
                .ok_or_else(|| ResourceError::TypeMismatch(resource_id));
        }
        
        // Initialize with dependencies
        self.initialize_with_dependencies::<T>().await
    }
    
    async fn initialize_with_dependencies<T: Resource + 'static>(
        &self,
    ) -> Result<Arc<T::Instance>, ResourceError> {
        let resource_id = T::metadata().id.clone();
        
        // Get initialization order for this resource and its dependencies
        let init_order = self.get_initialization_chain(&resource_id).await?;
        
        // Initialize in order
        for dep_id in init_order {
            if !self.instances.contains_key(&dep_id) {
                self.initialize_resource(&dep_id).await?;
            }
        }
        
        // Now initialize the requested resource
        self.initialize_resource(&resource_id).await?;
        
        // Return the instance
        self.instances
            .get(&resource_id)
            .and_then(|entry| entry.downcast_ref::<Arc<T::Instance>>().cloned())
            .ok_or_else(|| ResourceError::InitializationFailed(resource_id))
    }
    
    async fn initialize_resource(&self, resource_id: &str) -> Result<(), ResourceError> {
        let factory = self.factories
            .get(resource_id)
            .ok_or_else(|| ResourceError::NotRegistered(resource_id.to_string()))?;
        
        // Update state
        self.update_resource_state(resource_id, ResourceState::Initializing).await?;
        
        // Create instance with context
        let context = self.create_context(resource_id).await?;
        
        match factory.create(self, &context).await {
            Ok(instance) => {
                self.instances.insert(resource_id.to_string(), instance);
                self.update_resource_state(resource_id, ResourceState::Ready).await?;
                Ok(())
            }
            Err(e) => {
                self.update_resource_state(
                    resource_id,
                    ResourceState::Failed(e.to_string())
                ).await?;
                Err(e)
            }
        }
    }
}
```

### Circular Dependency Detection

```rust
impl DependencyGraph {
    /// Detect circular dependencies using DFS
    pub fn has_circular_dependency(&self) -> Result<bool, DependencyError> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        
        for node in self.nodes.keys() {
            if !visited.contains(node) {
                if self.has_cycle_dfs(node, &mut visited, &mut rec_stack)? {
                    return Ok(true);
                }
            }
        }
        
        Ok(false)
    }
    
    fn has_cycle_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> Result<bool, DependencyError> {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        
        if let Some(neighbors) = self.edges.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if self.has_cycle_dfs(neighbor, visited, rec_stack)? {
                        return Ok(true);
                    }
                } else if rec_stack.contains(neighbor) {
                    return Ok(true);
                }
            }
        }
        
        rec_stack.remove(node);
        Ok(false)
    }
    
    /// Find the circular dependency path for error reporting
    pub fn find_circular_path(&self) -> Vec<String> {
        // Use Tarjan's algorithm to find strongly connected components
        let sccs = self.find_strongly_connected_components();
        
        // Find a cycle in SCCs with more than one node
        for scc in sccs {
            if scc.len() > 1 {
                return self.extract_cycle_path(scc);
            }
        }
        
        Vec::new()
    }
}
```

### Cascade Failure Handling

```rust
/// Handle cascading failures in dependency chain
pub struct CascadeFailureHandler {
    graph: Arc<RwLock<DependencyGraph>>,
    failure_policies: HashMap<String, FailurePolicy>,
}

#[derive(Clone)]
pub enum FailurePolicy {
    /// Stop all dependent resources
    StopDependents,
    
    /// Try to continue with degraded functionality
    ContinueDegraded,
    
    /// Switch to backup resources
    Failover { backup_mapping: HashMap<String, String> },
    
    /// Isolate the failure
    Isolate,
}

impl CascadeFailureHandler {
    pub async fn handle_resource_failure(
        &self,
        failed_resource: &str,
        error: &ResourceError,
    ) -> Result<FailureRecoveryPlan, CascadeError> {
        let graph = self.graph.read().await;
        
        // Find all affected resources
        let affected = self.find_affected_resources(&graph, failed_resource)?;
        
        // Determine policy
        let policy = self.failure_policies
            .get(failed_resource)
            .cloned()
            .unwrap_or(FailurePolicy::StopDependents);
        
        // Create recovery plan
        let mut plan = FailureRecoveryPlan::new();
        
        match policy {
            FailurePolicy::StopDependents => {
                for resource in affected {
                    plan.add_action(RecoveryAction::Stop(resource));
                }
            }
            FailurePolicy::ContinueDegraded => {
                for resource in affected {
                    plan.add_action(RecoveryAction::SwitchToDegraded(resource));
                }
            }
            FailurePolicy::Failover { backup_mapping } => {
                if let Some(backup) = backup_mapping.get(failed_resource) {
                    plan.add_action(RecoveryAction::Failover {
                        failed: failed_resource.to_string(),
                        backup: backup.clone(),
                    });
                }
                
                // Update dependents to use backup
                for resource in affected {
                    plan.add_action(RecoveryAction::UpdateDependency {
                        resource,
                        old_dep: failed_resource.to_string(),
                        new_dep: backup_mapping.get(failed_resource)
                            .cloned()
                            .unwrap_or_else(|| failed_resource.to_string()),
                    });
                }
            }
            FailurePolicy::Isolate => {
                plan.add_action(RecoveryAction::Isolate(failed_resource.to_string()));
                
                // Notify dependents but don't stop them
                for resource in affected {
                    plan.add_action(RecoveryAction::NotifyDegradation(resource));
                }
            }
        }
        
        Ok(plan)
    }
    
    fn find_affected_resources(
        &self,
        graph: &DependencyGraph,
        failed_resource: &str,
    ) -> Result<Vec<String>, CascadeError> {
        let mut affected = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        
        // Start with direct dependents
        if let Some(dependents) = graph.reverse_edges.get(failed_resource) {
            for dep in dependents {
                queue.push_back(dep.clone());
            }
        }
        
        // BFS to find all affected resources
        while let Some(resource) = queue.pop_front() {
            if visited.insert(resource.clone()) {
                affected.push(resource.clone());
                
                // Add this resource's dependents
                if let Some(dependents) = graph.reverse_edges.get(&resource) {
                    for dep in dependents {
                        if !visited.contains(dep) {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }
        
        Ok(affected)
    }
}
```

### Dependency Visualization

```rust
/// Generate dependency graph visualization
pub struct DependencyVisualizer;

impl DependencyVisualizer {
    /// Generate Graphviz DOT format
    pub fn to_dot(&self, graph: &DependencyGraph) -> String {
        let mut dot = String::from("digraph Dependencies {\n");
        dot.push_str("  rankdir=LR;\n");
        dot.push_str("  node [shape=box];\n\n");
        
        // Add nodes with state coloring
        for (id, node) in &graph.nodes {
            let color = match node.state {
                ResourceState::Ready => "green",
                ResourceState::Failed(_) => "red",
                ResourceState::Initializing => "yellow",
                ResourceState::NotInitialized => "gray",
                ResourceState::Shutdown => "black",
            };
            
            dot.push_str(&format!(
                "  \"{}\" [color={}, label=\"{}\\n{}\"];\n",
                id,
                color,
                id,
                node.resource_type.split("::").last().unwrap_or(&node.resource_type)
            ));
        }
        
        dot.push_str("\n");
        
        // Add edges
        for (from, tos) in &graph.edges {
            for to in tos {
                dot.push_str(&format!("  \"{}\" -> \"{}\";\n", from, to));
            }
        }
        
        dot.push_str("}\n");
        dot
    }
    
    /// Generate Mermaid diagram
    pub fn to_mermaid(&self, graph: &DependencyGraph) -> String {
        let mut mermaid = String::from("graph TD\n");
        
        // Add nodes
        for (id, node) in &graph.nodes {
            let shape = match node.state {
                ResourceState::Ready => format!("{}[{}]", id, id),
                ResourceState::Failed(_) => format!("{}[{}]:::error", id, id),
                ResourceState::Initializing => format!("{}[{}]:::warning", id, id),
                _ => format!("{}[{}]", id, id),
            };
            mermaid.push_str(&format!("    {}\n", shape));
        }
        
        // Add edges
        for (from, tos) in &graph.edges {
            for to in tos {
                mermaid.push_str(&format!("    {} --> {}\n", from, to));
            }
        }
        
        // Add styles
        mermaid.push_str("\n    classDef error fill:#f96\n");
        mermaid.push_str("    classDef warning fill:#fc6\n");
        
        mermaid
    }
}
```

## Using Dependency Management

```rust
#[derive(Action)]
#[action(id = "send_email")]
#[resources([EmailServiceResource])]  // Automatically pulls in all dependencies
pub struct SendEmailAction;

impl ProcessAction for SendEmailAction {
    async fn execute(
        &self,
        input: EmailInput,
        context: &ExecutionContext,
    ) -> Result<ActionResult<EmailOutput>, ActionError> {
        // Email service and all its dependencies are available
        let email_service = context.get_resource::<EmailServiceResource>().await?;
        
        // Dependencies are automatically available through the service
        let result = email_service
            .send_email(input.to, input.subject, input.body)
            .await?;
        
        Ok(ActionResult::Success(EmailOutput {
            message_id: result.message_id,
            sent_at: result.sent_at,
        }))
    }
}

// Resource manager handles initialization order
impl ResourceManager {
    pub async fn initialize_all(&mut self) -> Result<(), ResourceError> {
        // Build dependency graph
        let mut graph = DependencyGraph::new();
        
        // Add all registered resources
        graph.add_resource::<LoggerResource>()?;
        graph.add_resource::<CacheResource>()?;
        graph.add_resource::<SmtpClientResource>()?;
        graph.add_resource::<TemplateEngineResource>()?;
        graph.add_resource::<EmailServiceResource>()?;
        
        // Get initialization order
        let init_order = graph.get_initialization_order()?;
        
        println!("Initialization order: {:?}", init_order);
        // Output: ["logger", "cache", "smtp_client", "template_engine", "email_service"]
        
        // Initialize in order
        for resource_id in init_order {
            self.initialize_resource(&resource_id).await?;
        }
        
        Ok(())
    }
}
```

## Configuration

```toml
# Dependency configuration
[dependencies]
# Define resource dependencies
[dependencies.email_service]
depends_on = ["smtp_client", "template_engine", "logger"]
failure_policy = "isolate"
health_check_cascade = true

[dependencies.smtp_client]
depends_on = ["logger"]
failure_policy = "stop_dependents"
retry_on_failure = true
max_retry_attempts = 3

[dependencies.template_engine]
depends_on = ["logger", "cache"]
failure_policy = "continue_degraded"
optional_dependencies = ["metrics"]  # Optional, won't fail if missing

# Circular dependency detection
[dependencies.validation]
detect_circular = true
allow_self_reference = false
max_depth = 10

# Cascade failure policies
[failure_policies]
default = "stop_dependents"

[failure_policies.database]
policy = "failover"
backup_resource = "database_replica"
health_check_interval = "10s"

[failure_policies.cache]
policy = "continue_degraded"
degradation_timeout = "5m"
```

## Testing Dependencies

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resource::testing::*;
    
    #[tokio::test]
    async fn test_dependency_resolution() {
        let mut graph = DependencyGraph::new();
        
        // Add resources with dependencies
        graph.add_resource::<LoggerResource>().unwrap();
        graph.add_resource::<CacheResource>().unwrap();
        graph.add_resource::<TemplateEngineResource>().unwrap();
        
        // Get initialization order
        let order = graph.get_initialization_order().unwrap();
        
        // Logger and Cache should come before TemplateEngine
        let logger_pos = order.iter().position(|x| x == "logger").unwrap();
        let cache_pos = order.iter().position(|x| x == "cache").unwrap();
        let template_pos = order.iter().position(|x| x == "template_engine").unwrap();
        
        assert!(logger_pos < template_pos);
        assert!(cache_pos < template_pos);
    }
    
    #[tokio::test]
    async fn test_circular_dependency_detection() {
        let mut graph = DependencyGraph::new();
        
        // Create circular dependency
        #[derive(Resource)]
        #[resource(id = "resource_a", depends_on = ["resource_b"])]
        struct ResourceA;
        
        #[derive(Resource)]
        #[resource(id = "resource_b", depends_on = ["resource_c"])]
        struct ResourceB;
        
        #[derive(Resource)]
        #[resource(id = "resource_c", depends_on = ["resource_a"])]  // Circular!
        struct ResourceC;
        
        graph.add_resource::<ResourceA>().unwrap();
        graph.add_resource::<ResourceB>().unwrap();
        
        let result = graph.add_resource::<ResourceC>();
        assert!(matches!(result, Err(DependencyError::CircularDependency(_))));
    }
    
    #[tokio::test]
    async fn test_cascade_failure() {
        let container = ResourceContainer::new();
        let handler = CascadeFailureHandler::new();
        
        // Simulate failure
        let plan = handler.handle_resource_failure(
            "smtp_client",
            &ResourceError::ConnectionFailed("SMTP server down".into()),
        ).await.unwrap();
        
        // Email service should be affected
        assert!(plan.affected_resources().contains(&"email_service".to_string()));
    }
    
    #[tokio::test]
    async fn test_lazy_loading() {
        let container = ResourceContainer::new();
        
        // Register factories
        container.register_factory(LoggerResourceFactory::new());
        container.register_factory(EmailServiceResourceFactory::new());
        
        // Get email service - should automatically initialize logger first
        let email_service = container.get::<EmailServiceResource>().await.unwrap();
        
        // Logger should also be initialized
        assert!(container.is_initialized("logger"));
        assert!(container.is_initialized("email_service"));
    }
}
```

## Best Practices

1. **Keep dependencies minimal** - Only declare essential dependencies
2. **Avoid circular dependencies** - Use interfaces or events instead
3. **Handle optional dependencies** - Don't fail if optional deps missing
4. **Test dependency chains** - Ensure initialization order is correct
5. **Plan for failures** - Define cascade failure policies
6. **Use lazy loading** - Initialize only when needed
7. **Monitor dependency health** - Track health of entire chain
8. **Document dependencies** - Make relationships clear
9. **Version dependencies** - Handle dependency version conflicts
10. **Visualize graphs** - Use tools to understand complex dependencies