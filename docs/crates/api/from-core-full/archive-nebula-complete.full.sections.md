#### 13. nebula-api (Week 10-11)
- [ ] 13.1 **REST API**
  - [ ] 13.1.1 Setup Axum framework
  - [ ] 13.1.2 Create workflow endpoints
  - [ ] 13.1.3 Create execution endpoints
  - [ ] 13.1.4 Create node endpoints
  - [ ] 13.1.5 Add authentication
  - [ ] 13.1.6 Add rate limiting

- [ ] 13.2 **GraphQL** — отложен; API только REST + WebSocket

- [ ] 13.3 **WebSocket Support**
  - [ ] 13.3.1 Implement WebSocket handler
  - [ ] 13.3.2 Add real-time updates
  - [ ] 13.3.3 Add execution streaming
  - [ ] 13.3.4 Add log streaming
  - [ ] 13.3.5 Add metrics streaming
  - [ ] 13.3.6 Add connection management

- [ ] 13.4 **API Documentation**
  - [ ] 13.4.1 Add OpenAPI spec
  - [ ] 13.4.2 Add OpenAPI/WebSocket docs
  - [ ] 13.4.3 Add example requests
  - [ ] 13.4.4 Add error codes
  - [ ] 13.4.5 Add rate limit docs
  - [ ] 13.4.6 Add webhook docs

#### 14. Standard Nodes (Week 11-12)
- [ ] 14.1 **HTTP Nodes**
  - [ ] 14.1.1 Create HTTP Request node
  - [ ] 14.1.2 Create HTTP Response node
  - [ ] 14.1.3 Create Webhook trigger
  - [ ] 14.1.4 Add authentication support
  - [ ] 14.1.5 Add proxy support
  - [ ] 14.1.6 Add retry configuration

- [ ] 14.2 **Data Transform Nodes**
  - [ ] 14.2.1 Create JSON Transform node
  - [ ] 14.2.2 Create CSV Parser node
  - [ ] 14.2.3 Create Data Mapper node
  - [ ] 14.2.4 Create Filter node
  - [ ] 14.2.5 Create Aggregation node
  - [ ] 14.2.6 Create Sort node

- [ ] 14.3 **Database Nodes**
  - [ ] 14.3.1 Create PostgreSQL node
  - [ ] 14.3.2 Create MySQL node
  - [ ] 14.3.3 Create MongoDB node
  - [ ] 14.3.4 Add query builder
  - [ ] 14.3.5 Add transaction support
  - [ ] 14.3.6 Add connection pooling

- [ ] 14.4 **Utility Nodes**
  - [ ] 14.4.1 Create Logger node
  - [ ] 14.4.2 Create Delay node
  - [ ] 14.4.3 Create Conditional node
  - [ ] 14.4.4 Create Loop node
  - [ ] 14.4.5 Create Error Handler node
  - [ ] 14.4.6 Create Notification node

### Phase 5: Production Features (Weeks 13-16)

#### 15. Performance Optimization (Week 13)
- [ ] 15.1 **Profiling**
  - [ ] 15.1.1 Add performance benchmarks
  - [ ] 15.1.2 Identify bottlenecks
  - [ ] 15.1.3 Add flame graphs
  - [ ] 15.1.4 Memory profiling
  - [ ] 15.1.5 CPU profiling
  - [ ] 15.1.6 I/O profiling

- [ ] 15.2 **Optimization**
  - [ ] 15.2.1 Optimize serialization
  - [ ] 15.2.2 Add zero-copy where possible
  - [ ] 15.2.3 Optimize allocations
  - [ ] 15.2.4 Add SIMD optimizations
  - [ ] 15.2.5 Optimize database queries
  - [ ] 15.2.6 Add caching strategies

#### 16. Monitoring & Observability (Week 14)
- [ ] 16.1 **Metrics**
  - [ ] 16.1.1 Integrate Prometheus
  - [ ] 16.1.2 Add custom metrics
  - [ ] 16.1.3 Add dashboards
  - [ ] 16.1.4 Add alerts
  - [ ] 16.1.5 Add SLI/SLO tracking
  - [ ] 16.1.6 Add capacity planning

- [ ] 16.2 **Tracing**
  - [ ] 16.2.1 Integrate OpenTelemetry
  - [ ] 16.2.2 Add distributed tracing
  - [ ] 16.2.3 Add trace sampling
  - [ ] 16.2.4 Add context propagation
  - [ ] 16.2.5 Add trace visualization
  - [ ] 16.2.6 Add performance analysis

#### 17. Security (Week 15)
- [ ] 17.1 **Authentication & Authorization**
  - [ ] 17.1.1 Add JWT support
  - [ ] 17.1.2 Add OAuth2 integration
  - [ ] 17.1.3 Add RBAC system
  - [ ] 17.1.4 Add API key management
  - [ ] 17.1.5 Add session management
  - [ ] 17.1.6 Add MFA support

- [ ] 17.2 **Security Hardening**
  - [ ] 17.2.1 Add input validation
  - [ ] 17.2.2 Add SQL injection prevention
  - [ ] 17.2.3 Add XSS prevention
  - [ ] 17.2.4 Add rate limiting
  - [ ] 17.2.5 Add encryption at rest
  - [ ] 17.2.6 Add audit logging

#### 18. Documentation & Testing (Week 16)
- [ ] 18.1 **Documentation**
  - [ ] 18.1.1 Complete API documentation
  - [ ] 18.1.2 Write architecture guide
  - [ ] 18.1.3 Create user manual
  - [ ] 18.1.4 Add deployment guide
  - [ ] 18.1.5 Create troubleshooting guide
  - [ ] 18.1.6 Add migration guide

- [ ] 18.2 **Testing**
  - [ ] 18.2.1 Achieve 80% test coverage
  - [ ] 18.2.2 Add integration tests
  - [ ] 18.2.3 Add load tests
  - [ ] 18.2.4 Add chaos testing
  - [ ] 18.2.5 Add security tests
  - [ ] 18.2.6 Add regression tests

---

# 📁 Crate Documentation


---

## nebula-api

### Purpose
API layer: **REST + WebSocket** (GraphQL не планируется в текущей фазе).

### Responsibilities
- REST endpoints
- WebSocket streaming (real-time, execution logs)
- Authentication

### Architecture
```rust
pub struct ApiServer {
    rest: RestApi,
    websocket: WebSocketHandler,
    auth: AuthManager,
}
```

---
