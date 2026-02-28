---

# nebula-api

## Purpose

`nebula-api` provides the external API layer for Nebula: **REST + WebSocket** (GraphQL не планируется в текущей фазе).

## Responsibilities

- REST API endpoints
- WebSocket real-time communication
- Authentication and authorization
- Rate limiting
- API documentation

## Architecture

### Core Components

```rust
pub struct ApiServer {
    // HTTP server
    server: Server,
    
    // API implementations (REST + WebSocket only)
    rest_api: Arc<RestApi>,
    websocket_handler: Arc<WebSocketHandler>,
    
    // Shared services
    auth_service: Arc<AuthService>,
    rate_limiter: Arc<RateLimiter>,
    
    // Backend services
    engine: Arc<WorkflowEngine>,
    storage: Arc<dyn StorageBackend>,
    
    // Metrics
    metrics: Arc<ApiMetrics>,
}

pub struct ApiConfig {
    pub host: String,
    pub port: u16,
    pub tls_config: Option<TlsConfig>,
    pub cors_config: CorsConfig,
    pub auth_config: AuthConfig,
    pub rate_limit_config: RateLimitConfig,
}
```

### REST API

```rust
pub struct RestApi {
    engine: Arc<WorkflowEngine>,
    storage: Arc<dyn StorageBackend>,
    validator: Arc<RequestValidator>,
}

impl RestApi {
    pub fn routes(&self) -> Router {
        Router::new()
            // Workflow endpoints
            .route("/api/v1/workflows", post(create_workflow))
            .route("/api/v1/workflows", get(list_workflows))
            .route("/api/v1/workflows/:id", get(get_workflow))
            .route("/api/v1/workflows/:id", put(update_workflow))
            .route("/api/v1/workflows/:id", delete(delete_workflow))
            .route("/api/v1/workflows/:id/versions", get(list_versions))
            .route("/api/v1/workflows/:id/activate", post(activate_workflow))
            .route("/api/v1/workflows/:id/deactivate", post(deactivate_workflow))
            
            // Execution endpoints
            .route("/api/v1/workflows/:id/execute", post(execute_workflow))
            .route("/api/v1/executions", get(list_executions))
            .route("/api/v1/executions/:id", get(get_execution))
            .route("/api/v1/executions/:id/cancel", post(cancel_execution))
            .route("/api/v1/executions/:id/logs", get(get_execution_logs))
            .route("/api/v1/executions/:id/nodes/:node_id/output", get(get_node_output))
            
            // Node endpoints
            .route("/api/v1/nodes", get(list_nodes))
            .route("/api/v1/nodes/:id", get(get_node))
            .route("/api/v1/nodes/:id/documentation", get(get_node_docs))
            
            // Resource endpoints
            .route("/api/v1/resources", get(list_resources))
            .route("/api/v1/resources/:id/health", get(check_resource_health))
            
            // System endpoints
            .route("/api/v1/health", get(health_check))
            .route("/api/v1/metrics", get(get_metrics))
            
            // Apply middleware
            .layer(AuthLayer::new(self.auth_service.clone()))
            .layer(RateLimitLayer::new(self.rate_limiter.clone()))
            .layer(TraceLayer::new_for_http())
            .layer(CompressionLayer::new())
            .layer(CorsLayer::new(self.cors_config.clone()))
    }
}

// Workflow handlers
async fn create_workflow(
    State(api): State<Arc<RestApi>>,
    Json(request): Json<CreateWorkflowRequest>,
) -> Result<Json<CreateWorkflowResponse>, ApiError> {
    // Validate request
    api.validator.validate(&request)?;
    
    // Create workflow
    let workflow = Workflow::from_request(request)?;
    api.storage.save_workflow(&workflow).await?;
    
    // Deploy if requested
    if request.deploy {
        api.engine.deploy_workflow(workflow.clone()).await?;
    }
    
    Ok(Json(CreateWorkflowResponse {
        id: workflow.id,
        version: workflow.version,
        status: workflow.status,
    }))
}

async fn execute_workflow(
    State(api): State<Arc<RestApi>>,
    Path(workflow_id): Path<String>,
    Json(request): Json<ExecuteWorkflowRequest>,
) -> Result<Json<ExecuteWorkflowResponse>, ApiError> {
    let workflow_id = WorkflowId::from_str(&workflow_id)?;
    
    // Create execution request
    let execution_request = ExecutionRequest {
        workflow_id,
        input: request.input,
        trigger: TriggerInfo::Manual {
            user: request.user,
        },
        parent_execution: request.parent_execution,
    };
    
    // Execute
    let handle = api.engine.create_execution(execution_request).await?;
    
    Ok(Json(ExecuteWorkflowResponse {
        execution_id: handle.execution_id,
        status: ExecutionStatus::Created,
    }))
}
```

### GraphQL — отложен

API только REST + WebSocket. GraphQL при необходимости можно добавить позже.

<details>
<summary>Возможная будущая структура GraphQL (не в текущем плане)</summary>

```rust
pub struct GraphqlApi {
    schema: Schema<Query, Mutation, Subscription>,
}

#[derive(Default)]
pub struct Query;

#[Object]
impl Query {
    async fn workflow(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Workflow>> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        let workflow_id = WorkflowId::from_str(&id)?;
        
        match storage.load_workflow(&workflow_id).await {
            Ok(workflow) => Ok(Some(workflow)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    async fn workflows(
        &self,
        ctx: &Context<'_>,
        filter: Option<WorkflowFilterInput>,
        first: Option<i32>,
        after: Option<String>,
    ) -> Result<Connection<Workflow>> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        
        let filter = filter.map(Into::into).unwrap_or_default();
        let workflows = storage.list_workflows(filter).await?;
        
        // Create connection
        let connection = Connection::new(
            workflows,
            first.unwrap_or(20) as usize,
            after,
        );
        
        Ok(connection)
    }
    
    async fn execution(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Execution>> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        let execution_id = ExecutionId::from_str(&id)?;
        
        match storage.load_execution(&execution_id).await {
            Ok(execution) => Ok(Some(execution)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    async fn node(&self, ctx: &Context<'_>, id: String) -> Result<Option<NodeInfo>> {
        let registry = ctx.data::<Arc<NodeRegistry>>()?;
        
        match registry.get_node_metadata(&id).await {
            Some(metadata) => Ok(Some(NodeInfo::from(metadata))),
            None => Ok(None),
        }
    }
}

#[derive(Default)]
pub struct Mutation;

#[Object]
impl Mutation {
    async fn create_workflow(
        &self,
        ctx: &Context<'_>,
        input: CreateWorkflowInput,
    ) -> Result<CreateWorkflowPayload> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        let engine = ctx.data::<Arc<WorkflowEngine>>()?;
        
        let workflow = Workflow::from_input(input)?;
        storage.save_workflow(&workflow).await?;
        
        if input.deploy {
            engine.deploy_workflow(workflow.clone()).await?;
        }
        
        Ok(CreateWorkflowPayload {
            workflow,
            success: true,
        })
    }
    
    async fn execute_workflow(
        &self,
        ctx: &Context<'_>,
        workflow_id: ID,
        input: Option<Json>,
    ) -> Result<ExecuteWorkflowPayload> {
        let engine = ctx.data::<Arc<WorkflowEngine>>()?;
        
        let execution_request = ExecutionRequest {
            workflow_id: WorkflowId::from_str(&workflow_id)?,
            input: input.unwrap_or(json!({})),
            trigger: TriggerInfo::Manual {
                user: ctx.data::<User>()?.clone(),
            },
            parent_execution: None,
        };
        
        let handle = engine.create_execution(execution_request).await?;
        
        Ok(ExecuteWorkflowPayload {
            execution_id: handle.execution_id,
            status: ExecutionStatus::Created,
        })
    }
}

#[derive(Default)]
pub struct Subscription;

#[Subscription]
impl Subscription {
    async fn execution_updates(
        &self,
        ctx: &Context<'_>,
        execution_id: ID,
    ) -> impl Stream<Item = ExecutionUpdate> {
        let event_bus = ctx.data::<Arc<dyn EventBus>>()?;
        let execution_id = ExecutionId::from_str(&execution_id)?;
        
        let stream = event_bus
            .subscribe(&format!("execution.{}", execution_id))
            .await?
            .filter_map(move |event| {
                match event {
                    Event::ExecutionUpdate(update) if update.execution_id == execution_id => {
                        Some(update)
                    }
                    _ => None,
                }
            });
            
        Ok(stream)
    }
    
    async fn workflow_logs(
        &self,
        ctx: &Context<'_>,
        workflow_id: ID,
    ) -> impl Stream<Item = LogEntry> {
        let event_bus = ctx.data::<Arc<dyn EventBus>>()?;
        let workflow_id = WorkflowId::from_str(&workflow_id)?;
        
        let stream = event_bus
            .subscribe(&format!("logs.workflow.{}", workflow_id))
            .await?
            .filter_map(move |event| {
                match event {
                    Event::LogEntry(entry) if entry.workflow_id == Some(workflow_id) => {
                        Some(entry)
                    }
                    _ => None,
                }
            });
            
        Ok(stream)
    }
}
```

</details>

### WebSocket Handler

```rust
pub struct WebSocketHandler {
    sessions: Arc<DashMap<SessionId, WebSocketSession>>,
    event_bus: Arc<dyn EventBus>,
    auth_service: Arc<AuthService>,
}

pub struct WebSocketSession {
    id: SessionId,
    user: User,
    subscriptions: Vec<Subscription>,
    sender: mpsc::UnboundedSender<Message>,
}

impl WebSocketHandler {
    pub async fn handle_connection(
        &self,
        ws: WebSocket,
        user: User,
    ) -> Result<(), Error> {
        let session_id = SessionId::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let (ws_sender, mut ws_receiver) = ws.split();
        
        // Create session
        let session = WebSocketSession {
            id: session_id.clone(),
            user,
            subscriptions: Vec::new(),
            sender: tx,
        };
        
        self.sessions.insert(session_id.clone(), session);
        
        // Spawn sender task
        let sender_task = tokio::spawn(
            rx.forward(ws_sender).map(|_| ())
        );
        
        // Handle incoming messages
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    self.handle_message(&session_id, text).await?;
                }
                Ok(Message::Binary(bin)) => {
                    self.handle_binary(&session_id, bin).await?;
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        
        // Cleanup
        self.cleanup_session(&session_id).await?;
        sender_task.abort();
        
        Ok(())
    }
    
    async fn handle_message(
        &self,
        session_id: &SessionId,
        text: String,
    ) -> Result<(), Error> {
        let message: WsMessage = serde_json::from_str(&text)?;
        
        match message {
            WsMessage::Subscribe { channel } => {
                self.handle_subscribe(session_id, channel).await?;
            }
            WsMessage::Unsubscribe { channel } => {
                self.handle_unsubscribe(session_id, channel).await?;
            }
            WsMessage::Execute { workflow_id, input } => {
                self.handle_execute(session_id, workflow_id, input).await?;
            }
            WsMessage::Ping => {
                self.send_to_session(session_id, WsMessage::Pong).await?;
            }
        }
        
        Ok(())
    }
}
```

### Authentication

```rust
pub struct AuthService {
    jwt_validator: Arc<JwtValidator>,
    api_key_store: Arc<ApiKeyStore>,
    oauth_provider: Arc<OAuthProvider>,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, request: &Request) -> Result<AuthContext, Error>;
}

pub struct AuthContext {
    pub user: User,
    pub method: AuthMethod,
    pub permissions: Vec<Permission>,
}

pub enum AuthMethod {
    Jwt,
    ApiKey,
    OAuth,
    Basic,
}

impl AuthService {
    pub async fn authenticate(
        &self,
        request: &Request<Body>,
    ) -> Result<AuthContext, Error> {
        // Try JWT first
        if let Some(token) = extract_bearer_token(request) {
            if let Ok(claims) = self.jwt_validator.validate(token).await {
                return Ok(AuthContext {
                    user: User::from_claims(claims)?,
                    method: AuthMethod::Jwt,
                    permissions: claims.permissions,
                });
            }
        }
        
        // Try API key
        if let Some(api_key) = extract_api_key(request) {
            if let Some(key_info) = self.api_key_store.get_key(api_key).await? {
                return Ok(AuthContext {
                    user: key_info.user,
                    method: AuthMethod::ApiKey,
                    permissions: key_info.permissions,
                });
            }
        }
        
        // Try OAuth
        if let Some(oauth_token) = extract_oauth_token(request) {
            if let Ok(user_info) = self.oauth_provider.get_user_info(oauth_token).await {
                return Ok(AuthContext {
                    user: User::from_oauth(user_info)?,
                    method: AuthMethod::OAuth,
                    permissions: vec![Permission::Read],
                });
            }
        }
        
        Err(Error::Unauthorized)
    }
}
```

### Rate Limiting

```rust
pub struct RateLimiter {
    store: Arc<RateLimitStore>,
    config: RateLimitConfig,
}

pub struct RateLimitConfig {
    pub default_limit: u32,
    pub window: Duration,
    pub burst_size: u32,
    pub custom_limits: HashMap<String, RateLimit>,
}

pub struct RateLimit {
    pub requests: u32,
    pub window: Duration,
    pub burst: u32,
}

impl RateLimiter {
    pub async fn check_rate_limit(
        &self,
        key: &str,
        cost: u32,
    ) -> Result<RateLimitStatus, Error> {
        let limit = self.get_limit_for_key(key);
        let window_start = Utc::now() - limit.window;
        
        // Get current usage
        let usage = self.store
            .get_usage(key, window_start)
            .await?;
            
        if usage + cost > limit.requests {
            return Ok(RateLimitStatus::Exceeded {
                limit: limit.requests,
                remaining: 0,
                reset_at: window_start + limit.window,
            });
        }
        
        // Record usage
        self.store.record_usage(key, cost).await?;
        
        Ok(RateLimitStatus::Allowed {
            limit: limit.requests,
            remaining: limit.requests - usage - cost,
            reset_at: window_start + limit.window,
        })
    }
}
```

### API Documentation

```rust
pub struct ApiDocumentation {
    openapi: OpenApi,
    examples: HashMap<String, Example>,
}

impl ApiDocumentation {
    pub fn generate() -> Self {
        let mut openapi = OpenApi {
            openapi: "3.0.0".to_string(),
            info: Info {
                title: "Nebula Workflow API".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some("Workflow automation engine API".to_string()),
                ..Default::default()
            },
            servers: vec![Server {
                url: "/api/v1".to_string(),
                description: Some("API v1".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        
        // Add paths
        openapi.paths.insert(
            "/workflows".to_string(),
            PathItem {
                get: Some(Operation {
                    summary: Some("List workflows".to_string()),
                    operation_id: Some("listWorkflows".to_string()),
                    parameters: vec![
                        Parameter::Query {
                            name: "limit".to_string(),
                            required: false,
                            schema: Schema::Integer {
                                default: Some(20),
                                minimum: Some(1),
                                maximum: Some(100),
                            },
                        },
                    ],
                    responses: Responses {
                        responses: btreemap! {
                            "200".to_string() => Response {
                                description: "List of workflows".to_string(),
