# Nebula Identity & Multi-Tenancy Roadmap

## 🎯 Vision

Создать production-ready систему управления пользователями, организациями и multi-tenancy для Nebula workflow engine, обеспечивающую:
- Project-based isolation (как в n8n)
- Granular RBAC с custom roles
- Enterprise-ready authentication
- Scalable multi-tenant architecture

---

## 📋 Phase 1: Identity Foundation (2-3 недели)

### Milestone 1.1: Core User Management
**Крейт:** `nebula-user`

**Задачи:**
- [ ] Создать структуру крейта `nebula-user`
- [ ] Определить `User` entity с полями:
  - `id: UserId`
  - `email: String` (unique)
  - `first_name: String`
  - `last_name: String`
  - `password_hash: String`
  - `global_role_id: RoleId`
  - `created_at: DateTime<Utc>`
  - `updated_at: DateTime<Utc>`
  - `disabled: bool`
- [ ] Реализовать `UserRepository` trait (async)
- [ ] Password hashing (argon2 или bcrypt)
- [ ] Basic CRUD операции
- [ ] Unit tests

**Deliverables:**
```rust
// nebula-user/src/lib.rs
pub struct User { /* ... */ }
pub trait UserRepository: Send + Sync {
    async fn create(&self, user: CreateUser) -> Result<User>;
    async fn get_by_id(&self, id: &UserId) -> Result<Option<User>>;
    async fn get_by_email(&self, email: &str) -> Result<Option<User>>;
    async fn update(&self, id: &UserId, data: UpdateUser) -> Result<User>;
    async fn delete(&self, id: &UserId) -> Result<()>;
    async fn list(&self, filters: UserFilters) -> Result<Vec<User>>;
}
```

**Dependencies:**
- `nebula-core` (UserId, error types)
- `thiserror`, `async-trait`, `serde`
- `argon2` or `bcrypt`

---

### Milestone 1.2: Project Management
**Крейт:** `nebula-project`

**Задачи:**
- [ ] Создать структуру крейта `nebula-project`
- [ ] Определить `Project` entity:
  - `id: ProjectId`
  - `name: String`
  - `type: ProjectType` (Personal | Team)
  - `owner_id: UserId`
  - `created_at: DateTime<Utc>`
  - `settings: ProjectSettings` (JSON)
- [ ] Определить `ProjectMember` entity:
  - `project_id: ProjectId`
  - `user_id: UserId`
  - `role_id: RoleId`
  - `joined_at: DateTime<Utc>`
- [ ] Реализовать `ProjectRepository` trait
- [ ] Реализовать `ProjectMemberRepository` trait
- [ ] Personal project auto-creation при создании user
- [ ] Membership management (add/remove/update)
- [ ] Integration tests

**Deliverables:**
```rust
// nebula-project/src/lib.rs
pub struct Project { /* ... */ }
pub struct ProjectMember { /* ... */ }

pub trait ProjectRepository: Send + Sync {
    async fn create(&self, project: CreateProject) -> Result<Project>;
    async fn get_by_id(&self, id: &ProjectId) -> Result<Option<Project>>;
    async fn list_by_user(&self, user_id: &UserId) -> Result<Vec<Project>>;
    async fn update(&self, id: &ProjectId, data: UpdateProject) -> Result<Project>;
    async fn delete(&self, id: &ProjectId) -> Result<()>;
}

pub trait ProjectMemberRepository: Send + Sync {
    async fn add_member(&self, member: ProjectMember) -> Result<()>;
    async fn remove_member(&self, project_id: &ProjectId, user_id: &UserId) -> Result<()>;
    async fn update_role(&self, project_id: &ProjectId, user_id: &UserId, role: RoleId) -> Result<()>;
    async fn list_members(&self, project_id: &ProjectId) -> Result<Vec<ProjectMember>>;
    async fn get_member(&self, project_id: &ProjectId, user_id: &UserId) -> Result<Option<ProjectMember>>;
}
```

**Dependencies:**
- `nebula-core`
- `nebula-user`

---

### Milestone 1.3: Database Storage Implementation
**Крейт:** `nebula-storage` (создать)

**Задачи:**
- [ ] Создать крейт `nebula-storage`
- [ ] Определить базовый `Storage` trait (из документации)
- [ ] PostgreSQL implementation через `sqlx`
- [ ] Database migrations:
  - `users` table
  - `projects` table
  - `project_members` table
  - `roles` table (базовые роли)
- [ ] Имплементация `UserRepository` для PostgreSQL
- [ ] Имплементация `ProjectRepository` для PostgreSQL
- [ ] Имплементация `ProjectMemberRepository` для PostgreSQL
- [ ] Transaction support
- [ ] Connection pooling

**Deliverables:**
```rust
// nebula-storage/src/lib.rs
#[async_trait]
pub trait Storage: Send + Sync {
    type Key;
    type Value;
    type Error;
    
    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>>;
    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<()>;
    async fn delete(&self, key: &Self::Key) -> Result<()>;
    async fn exists(&self, key: &Self::Key) -> Result<bool>;
}

pub struct PostgresUserRepository { /* ... */ }
pub struct PostgresProjectRepository { /* ... */ }
```

**Dependencies:**
- `nebula-core`
- `nebula-user`
- `nebula-project`
- `sqlx` with `postgres` feature
- `tokio-postgres`

---

### Milestone 1.4: Basic API Endpoints
**Крейт:** `nebula-api` (создать)

**Задачи:**
- [ ] Создать крейт `nebula-api`
- [ ] Setup Axum web framework
- [ ] User endpoints:
  - `POST /api/users` - создать пользователя
  - `GET /api/users/:id` - получить пользователя
  - `GET /api/users` - список пользователей (admin only)
  - `PATCH /api/users/:id` - обновить пользователя
  - `DELETE /api/users/:id` - удалить пользователя
- [ ] Project endpoints:
  - `POST /api/projects` - создать проект
  - `GET /api/projects/:id` - получить проект
  - `GET /api/projects` - список проектов пользователя
  - `PATCH /api/projects/:id` - обновить проект
  - `DELETE /api/projects/:id` - удалить проект
- [ ] Project member endpoints:
  - `POST /api/projects/:id/members` - добавить участника
  - `GET /api/projects/:id/members` - список участников
  - `PATCH /api/projects/:id/members/:user_id` - обновить роль
  - `DELETE /api/projects/:id/members/:user_id` - удалить участника
- [ ] Error handling middleware
- [ ] Request validation
- [ ] API tests

**Deliverables:**
```rust
// nebula-api/src/routes/users.rs
async fn create_user(Json(payload): Json<CreateUserRequest>) -> Result<Json<User>>;
async fn get_user(Path(id): Path<UserId>) -> Result<Json<User>>;
```

**Dependencies:**
- `axum`, `tower`, `tower-http`
- `nebula-user`, `nebula-project`, `nebula-storage`

---

## 📋 Phase 2: Authentication & Authorization (2-3 недели)

### Milestone 2.1: Authentication System
**Крейт:** `nebula-auth`

**Задачи:**
- [ ] Создать крейт `nebula-auth`
- [ ] JWT token generation/validation (jsonwebtoken)
- [ ] Session management (Redis backed)
- [ ] Login/logout flow
- [ ] Password reset flow
- [ ] Email verification
- [ ] Refresh token mechanism
- [ ] Rate limiting для auth endpoints
- [ ] Integration с `nebula-api`

**Deliverables:**
```rust
// nebula-auth/src/lib.rs
pub struct AuthService {
    user_repo: Arc<dyn UserRepository>,
    session_store: Arc<dyn SessionStore>,
    jwt_config: JwtConfig,
}

impl AuthService {
    pub async fn login(&self, email: &str, password: &str) -> Result<AuthToken>;
    pub async fn logout(&self, token: &str) -> Result<()>;
    pub async fn validate_token(&self, token: &str) -> Result<Claims>;
    pub async fn refresh_token(&self, refresh: &str) -> Result<AuthToken>;
}
```

**Auth endpoints:**
- `POST /api/auth/login`
- `POST /api/auth/logout`
- `POST /api/auth/refresh`
- `POST /api/auth/reset-password`
- `POST /api/auth/verify-email`

**Dependencies:**
- `jsonwebtoken`, `uuid`
- `redis` (для sessions)
- `nebula-user`, `nebula-storage`

---

### Milestone 2.2: RBAC System
**Крейт:** `nebula-rbac`

**Задачи:**
- [ ] Создать крейт `nebula-rbac`
- [ ] Определить `Role` entity:
  - `id: RoleId`
  - `name: String`
  - `scope: RoleScope` (Global | Project | Credential | Workflow)
  - `builtin: bool`
- [ ] Определить `Permission` enum (Resource + Action)
- [ ] Определить built-in roles:
  - Global: `owner`, `admin`, `member`
  - Project: `admin`, `editor`, `viewer`
  - Resource: `owner`, `editor`, `user`
- [ ] Реализовать `PermissionChecker`:
  - Scope calculation (global + project + resource)
  - Permission evaluation
  - Context-aware checks
- [ ] Custom roles support (Phase 2.3)
- [ ] Permission middleware для API

**Deliverables:**
```rust
// nebula-rbac/src/lib.rs
pub enum Permission {
    WorkflowCreate,
    WorkflowRead,
    WorkflowUpdate,
    WorkflowDelete,
    WorkflowExecute,
    CredentialCreate,
    CredentialRead,
    CredentialUpdate,
    CredentialDelete,
    ProjectManage,
    ProjectMemberAdd,
    ProjectMemberRemove,
}

pub struct PermissionChecker {
    user_repo: Arc<dyn UserRepository>,
    project_repo: Arc<dyn ProjectRepository>,
    role_repo: Arc<dyn RoleRepository>,
}

impl PermissionChecker {
    pub async fn check(&self, user_id: &UserId, permission: Permission, context: &Context) -> Result<bool>;
    pub async fn require(&self, user_id: &UserId, permission: Permission, context: &Context) -> Result<()>;
}
```

**Dependencies:**
- `nebula-core`, `nebula-user`, `nebula-project`

---

### Milestone 2.3: Custom Roles (Enterprise)
**Расширение:** `nebula-rbac`

**Задачи:**
- [ ] `CustomRole` entity с granular permissions
- [ ] Role builder API
- [ ] Permission templates
- [ ] Role inheritance
- [ ] UI для custom role creation (Phase 4)
- [ ] Migration для custom roles table

**Deliverables:**
```rust
pub struct CustomRole {
    pub id: RoleId,
    pub name: String,
    pub project_id: ProjectId,
    pub permissions: Vec<Permission>,
    pub inherits_from: Option<RoleId>,
}

pub struct RoleBuilder {
    name: String,
    permissions: Vec<Permission>,
}

impl RoleBuilder {
    pub fn new(name: impl Into<String>) -> Self;
    pub fn allow(mut self, permission: Permission) -> Self;
    pub fn deny(mut self, permission: Permission) -> Self;
    pub fn inherit(mut self, role_id: RoleId) -> Self;
    pub fn build(self) -> CustomRole;
}
```

---

### Milestone 2.4: Integration с Workflow System

**Задачи:**
- [ ] Расширить `nebula-execution` для project context
- [ ] Расширить `nebula-credential` для project-scoped credentials
- [ ] `SharedWorkflow` entity (workflow_id, project_id, role_id)
- [ ] `SharedCredentials` entity (credential_id, project_id, role_id)
- [ ] Permission checks в workflow execution
- [ ] Permission checks в credential access
- [ ] Audit logging

**Deliverables:**
```rust
// nebula-execution/src/context.rs
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub project_id: ProjectId,  // ← новое
    pub user_id: UserId,        // ← новое
    pub permissions: Arc<PermissionChecker>,  // ← новое
    // ... остальные поля
}
```

---

## 📋 Phase 3: Multi-Tenancy & Isolation (2 недели)

### Milestone 3.1: Tenant Runtime Isolation
**Крейт:** `nebula-tenant`

**Задачи:**
- [ ] Создать крейт `nebula-tenant`
- [ ] Определить `Tenant` entity (alias для Project)
- [ ] `TenantContext` для request scope
- [ ] `TenantQuota` для resource limits:
  - `max_workflows: usize`
  - `max_executions_per_hour: usize`
  - `max_storage_gb: usize`
  - `max_concurrent_executions: usize`
  - `cpu_shares: f32`
  - `memory_limit_mb: usize`
- [ ] Quota enforcement middleware
- [ ] Resource allocator per tenant
- [ ] Tenant isolation в memory manager
- [ ] Tenant isolation в storage

**Deliverables:**
```rust
// nebula-tenant/src/lib.rs
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub user_id: UserId,
    pub quota: Arc<TenantQuota>,
}

pub struct TenantQuotaEnforcer {
    storage: Arc<dyn Storage>,
    metrics: Arc<MetricsCollector>,
}

impl TenantQuotaEnforcer {
    pub async fn check_quota(&self, tenant_id: &TenantId, resource: ResourceType) -> Result<()>;
    pub async fn consume(&self, tenant_id: &TenantId, resource: ResourceType, amount: u64) -> Result<()>;
}
```

**Dependencies:**
- `nebula-core`, `nebula-project`, `nebula-user`
- `nebula-metrics` (для tracking usage)

---

### Milestone 3.2: Data Partitioning Strategy

**Задачи:**
- [ ] Определить partition strategy:
  - Row-Level Security (RLS) в PostgreSQL
  - Table prefix strategy (альтернатива)
- [ ] Имплементация RLS policies
- [ ] Tenant-aware queries
- [ ] Migration scripts для RLS
- [ ] Performance testing

**SQL Example:**
```sql
-- Enable RLS
ALTER TABLE workflows ENABLE ROW LEVEL SECURITY;

-- Policy: users can only see workflows in their projects
CREATE POLICY tenant_isolation ON workflows
    USING (project_id IN (
        SELECT project_id FROM project_members WHERE user_id = current_user_id()
    ));
```

---

### Milestone 3.3: Tenant Middleware для API

**Задачи:**
- [ ] Tenant extraction middleware (из JWT/headers)
- [ ] Автоматическая инъекция `TenantContext`
- [ ] Request filtering по tenant
- [ ] Tenant-scoped caching
- [ ] Cross-tenant protection tests

**Deliverables:**
```rust
// nebula-api/src/middleware/tenant.rs
pub async fn tenant_middleware(
    req: Request,
    next: Next,
) -> Response {
    let tenant_id = extract_tenant_id(&req)?;
    let tenant_context = load_tenant_context(tenant_id).await?;
    
    req.extensions_mut().insert(tenant_context);
    next.call(req).await
}
```

---

## 📋 Phase 4: Enterprise Features (3-4 недели)

### Milestone 4.1: SSO & User Provisioning

**Задачи:**
- [ ] OAuth2 providers integration:
  - Google Workspace
  - Microsoft Azure AD
  - Okta
  - Generic OIDC
- [ ] SAML support
- [ ] SCIM protocol для user provisioning
- [ ] Auto-sync users from IdP
- [ ] Just-in-time (JIT) provisioning
- [ ] Group/role mapping

**Dependencies:**
- `oauth2`, `openidconnect` crates
- `saml-rs` or custom SAML implementation

---

### Milestone 4.2: Advanced RBAC Features

**Задачи:**
- [ ] Attribute-Based Access Control (ABAC)
- [ ] Conditional permissions (time-based, IP-based)
- [ ] Permission delegation
- [ ] Temporary access grants
- [ ] Approval workflows для sensitive actions
- [ ] Audit trail для permission changes

---

### Milestone 4.3: Organization Management

**Крейт:** `nebula-organization` (optional)

**Задачи:**
- [ ] `Organization` entity (выше Project)
- [ ] Organization billing
- [ ] Organization-wide settings
- [ ] Cross-project resource sharing
- [ ] Organization admin role

---

## 📋 Phase 5: Clustering & Scalability (2-3 недели)

### Milestone 5.1: Per-Tenant Workers

**Задачи:**
- [ ] Worker tagging по tenant
- [ ] Routing workflows к dedicated workers
- [ ] Worker pool management
- [ ] Load balancing per tenant
- [ ] Priority queues

---

### Milestone 5.2: Distributed Sessions

**Задачи:**
- [ ] Redis cluster для sessions
- [ ] Session replication
- [ ] Sticky sessions в load balancer
- [ ] Session migration при node failure

---

## 🎯 Success Metrics

**Phase 1:**
- ✅ User CRUD работает
- ✅ Projects с members работают
- ✅ PostgreSQL storage работает
- ✅ API endpoints отвечают корректно

**Phase 2:**
- ✅ JWT auth работает
- ✅ RBAC checks проходят
- ✅ Permission denied для unauthorized actions
- ✅ Integration с workflows работает

**Phase 3:**
- ✅ Tenant isolation работает (нет cross-tenant leaks)
- ✅ Quota enforcement работает
- ✅ Performance не деградирует с ростом tenants

**Phase 4:**
- ✅ SSO login работает
- ✅ Custom roles создаются и применяются
- ✅ SCIM provisioning работает

**Phase 5:**
- ✅ Cluster работает с multiple nodes
- ✅ Tenant-specific workers работают
- ✅ Failover работает

---

## 📦 Deliverables по крейтам

| Крейт | Phase | Статус | Dependencies |
|-------|-------|--------|--------------|
| `nebula-user` | 1.1 | 🔴 Not started | nebula-core |
| `nebula-project` | 1.2 | 🔴 Not started | nebula-core, nebula-user |
| `nebula-storage` | 1.3 | 🔴 Not started | nebula-core, sqlx |
| `nebula-api` | 1.4 | 🔴 Not started | axum, tower |
| `nebula-auth` | 2.1 | 🔴 Not started | jsonwebtoken, redis |
| `nebula-rbac` | 2.2 | 🔴 Not started | nebula-user, nebula-project |
| `nebula-tenant` | 3.1 | 🔴 Not started | nebula-project, nebula-metrics |
| `nebula-organization` | 4.3 | 🔴 Optional | nebula-project |

---

## 🚀 Recommended Start

**Начать с Phase 1.1-1.2:**
1. Создать `nebula-user` с базовым User entity
2. Создать `nebula-project` с Project и ProjectMember
3. Написать unit tests
4. Создать простой in-memory repository для тестов

**Не делать сразу:**
- ❌ Полную auth систему (можно заглушить)
- ❌ SSO (Enterprise feature)
- ❌ Custom roles (можно hardcode базовые)
- ❌ Clustering

---

## 📅 Timeline Overview

```
Phase 1: Identity Foundation        ████████░░░░░░░░░░░░░░░░ (2-3 weeks)
Phase 2: Auth & Authorization       ░░░░░░░░████████░░░░░░░░ (2-3 weeks)
Phase 3: Multi-Tenancy & Isolation  ░░░░░░░░░░░░░░░░████░░░░ (2 weeks)
Phase 4: Enterprise Features        ░░░░░░░░░░░░░░░░░░░░████ (3-4 weeks)
Phase 5: Clustering & Scalability   ░░░░░░░░░░░░░░░░░░░░░░░░ (2-3 weeks)

Total: ~11-15 weeks
```

---

## 🔗 Architecture Integration

```
┌─────────────────────────────────────────────────────────┐
│                 Presentation Layer                      │
│       (nebula-ui, nebula-api, nebula-cli)              │
├─────────────────────────────────────────────────────────┤
│            Multi-Tenancy & Identity Layer               │
│    (nebula-auth, nebula-rbac, nebula-tenant,           │
│     nebula-user, nebula-project, nebula-organization)  │
├─────────────────────────────────────────────────────────┤
│                 Business Logic Layer                    │
│         (nebula-resource, nebula-registry)              │
├─────────────────────────────────────────────────────────┤
│                   Execution Layer                       │
│      (nebula-engine, nebula-runtime, nebula-worker)     │
├─────────────────────────────────────────────────────────┤
│                     Node Layer                          │
│  (nebula-node, nebula-action, nebula-parameter,         │
│              nebula-credential)                         │
├─────────────────────────────────────────────────────────┤
│                     Core Layer                          │
│  (nebula-core, nebula-value, nebula-expression,         │
│   nebula-memory, nebula-eventbus)                       │
├─────────────────────────────────────────────────────────┤
│              Cross-Cutting Concerns Layer               │
│  (nebula-config, nebula-log, nebula-metrics,            │
│   nebula-resilience, nebula-validator)                  │
├─────────────────────────────────────────────────────────┤
│                Infrastructure Layer                     │
│         (nebula-storage, nebula-binary)                 │
└─────────────────────────────────────────────────────────┘
```

---

## 📝 Notes

- Roadmap может корректироваться based on feedback и приоритеты
- Phase 4-5 могут быть отложены для MVP
- Каждый milestone должен иметь полное test coverage
- Документация должна обновляться вместе с кодом
- Code review обязателен для всех PR
