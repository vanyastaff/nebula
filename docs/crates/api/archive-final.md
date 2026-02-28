# Archived From "docs/archive/final.md"

### nebula-api
**Назначение:** REST + WebSocket API для управления workflows (GraphQL не в текущем плане).

```rust
// REST endpoints
pub fn configure_routes(cfg: &mut ServiceConfig) {
    cfg
        // Workflows
        .route("/workflows", post(create_workflow))
        .route("/workflows/{id}", get(get_workflow))
        .route("/workflows/{id}/execute", post(execute_workflow))
        
        // Executions
        .route("/executions", get(list_executions))
        .route("/executions/{id}", get(get_execution))
        .route("/executions/{id}/cancel", post(cancel_execution))
        
        // Actions
        .route("/actions", get(list_actions))
        .route("/actions/search", get(search_actions));
}

// (GraphQL отложен; при необходимости — позже)
// #[derive(GraphQLObject)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub nodes: Vec<Node>,
    pub executions: Vec<Execution>,
}

// GraphQL Query/Mutation отложены; API — REST + WebSocket
// impl Query {
//     async fn workflow(&self, id: String) -> Result<Workflow> { ... }
//     async fn search_actions(&self, query: String) -> Vec<Action> { ... }
// }
```

---

