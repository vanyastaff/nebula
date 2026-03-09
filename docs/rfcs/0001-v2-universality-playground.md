# RFC 0001 V2 Universality Playground

Status: Draft
Purpose: Verify universality of Parameter API v2 with Rust-grade DX: typed, explicit, and safe.

This revision removes stringly configuration from the playground and proposes a cleaner model.

## 1. Rust-First Design Rules

1. No control metadata in free-form strings.
2. No sentinel field names (for example `_notice_*`).
3. UI-only elements are not represented as fake value fields.
4. Dynamic behavior is declared in typed contracts, not in ad-hoc hints.
5. Invalid states should be unrepresentable by enums/structs.
6. Container semantics are modeled explicitly, not collapsed into generic object blobs.

## 2. Core Model Split (clean and explicit)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub nodes: Vec<SchemaNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum SchemaNode {
    Field(FieldDef),
    Ui(UiNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub id: FieldId,
    pub value_spec: ValueSpec,
    pub constraints: Constraints,
    pub default: Option<serde_json::Value>,
    pub security: Option<SecurityPolicy>,
    pub presentation: Option<FieldPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldPresentation {
    pub label: Option<String>,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub group: Option<String>,
    pub editor: Option<EditorConfig>,
    pub visibility: VisibilityPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiNode {
    Notice(NoticeNode),
    Callout(CalloutNode),
    ActionButton(ActionButtonNode),
    CurlImport(CurlImportNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalloutNode {
    pub id: String,
    pub title: String,
    pub body: String,
    pub visible_if: Vec<ExpressionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurlImportNode {
    pub id: String,
    pub label: String,
    pub target_fields: Vec<FieldId>,
    pub enabled_if: Vec<ExpressionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoticeNode {
    pub id: String,
    pub text: String,
    pub severity: NoticeSeverity,
    pub visible_if: Vec<ExpressionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NoticeSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionButtonNode {
    pub id: String,
    pub label: String,
    pub action: UiAction,
    pub enabled_if: Vec<ExpressionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UiAction {
    RefreshSchema,
    TestConnection,
    Custom { key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "editor", rename_all = "snake_case")]
pub enum EditorConfig {
    PlainText,
    JavaScript,
    Sql { dialect: SqlDialect },
    Html,
    Css,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SqlDialect {
    PostgreSql,
    MySql,
    Sqlite,
    MsSql,
    Oracle,
    Standard,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum VisibilityPolicy {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub kind: SecretKind,
    pub write_only: bool,
    pub persist: PersistPolicy,
    pub redact_in: RedactionTargets,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SecretKind {
    Secret,
    Password,
    ApiKey,
    BearerToken,
    Credential,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PersistPolicy {
    Persist,
    SkipSave,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionTargets {
    pub logs: bool,
    pub diagnostics: bool,
    pub api_responses: bool,
    pub events: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "container", rename_all = "snake_case")]
pub enum ContainerSpec {
    Object(ObjectSpec),
    List(ListSpec),
    Mode(ModeSpec),
    Matrix(MatrixSpec),
    Reference(ReferenceSpec),
    Routing(RoutingSpec),
    Expirable(ExpirableSpec),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSpec {
    pub fields: Vec<FieldDef>,
    pub additional: Option<AdditionalPropertiesSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdditionalPropertiesSpec {
    pub value: Box<ValueSpec>,
    pub key_pattern: Option<String>,
    pub min_properties: Option<u32>,
    pub max_properties: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSpec {
    pub item: Box<ValueSpec>,
    pub min_items: Option<u32>,
    pub max_items: Option<u32>,
    pub unique: bool,
    pub ordering: ListOrdering,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ListOrdering {
    None,
    Sortable,
    Ranked { direction: RankDirection, show_numbers: bool },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RankDirection {
    HighestFirst,
    LowestFirst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeSpec {
    pub variants: Vec<ModeVariantSpec>,
    pub default_variant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeVariantSpec {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub value: ValueSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixSpec {
    pub rows: Vec<MatrixRowSpec>,
    pub columns: Vec<MatrixColumnSpec>,
    pub cell_type: MatrixCellType,
    pub all_rows_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixRowSpec {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixColumnSpec {
    pub value: String,
    pub label: String,
    pub weight: Option<i32>,
    pub exclusive: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MatrixCellType {
    Radio,
    Checkbox,
    Dropdown,
    Text,
    Rating,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceSpec {
    pub target: FieldId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingSpec {
    pub child: Option<Box<ValueSpec>>,
    pub connection_label: Option<String>,
    pub connection_required: bool,
    pub max_connections: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpirableSpec {
    pub child: Option<Box<ValueSpec>>,
    pub ttl_seconds: u64,
    pub auto_refresh: bool,
    pub auto_clear_expired: bool,
    pub warning_threshold_seconds: Option<u64>,
}
```

## 2.1 Container Ideas Adopted from paramdef

1. `Object + additional properties` is a first-class shape for headers/env maps, with typed key pattern and bounds.
2. `List` gets explicit ordering semantics (`None | Sortable | Ranked`) so ranking is not hidden in UI flags.
3. `Mode` is a discriminated union with stable variant keys and optional default variant.
4. `Matrix` is a dedicated container, not emulated with nested list/object hacks.
5. `Reference` keeps schema reuse explicit and avoids copy-paste structural drift.
6. `Routing` and `Expirable` stay as wrappers with typed options; runtime can choose whether missing `child` is valid.

Build-time validation rules for container specs:
- object: reject duplicate field ids; validate `min_properties <= max_properties`.
- list: require item spec; validate `min_items <= max_items`.
- mode: require at least one variant; reject duplicate variant keys; `default_variant` must exist.
- matrix: require non-empty rows and columns; reject duplicate row keys and duplicate column values.
- reference: target must resolve during schema compilation.
- expirable: `warning_threshold_seconds < ttl_seconds`.

## 2.2 Secret, Select, and Layout Ideas Adopted from paramdef

1. `secret` is treated as a semantic subtype and policy bundle, not as a standalone data shape.
2. Sensitive defaults should be forbidden by schema compile checks unless explicitly opted in for tests.
3. `Select` keeps two independent axes: selection mode (`single|multiple`) and source (`static|dynamic`).
4. `SelectOption` supports optional `description`, `icon`, `group` as typed fields.
5. Group/layout hierarchy is explicit:
   - `Group` may contain fields, containers, UI nodes, and `Panel`.
   - `Panel` may contain fields/containers/UI nodes, but not `Group` or nested `Panel`.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "layout", rename_all = "snake_case")]
pub enum LayoutNode {
    Group(GroupNode),
    Panel(PanelNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupNode {
    pub id: String,
    pub title: Option<String>,
    pub layout: GroupLayout,
    pub children: Vec<LayoutChild>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelNode {
    pub id: String,
    pub title: Option<String>,
    pub display: PanelDisplay,
    pub collapsed: bool,
    pub children: Vec<PanelChild>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum GroupLayout {
    Vertical,
    Horizontal,
    Grid,
    Tabs,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PanelDisplay {
    Section,
    Collapsible,
    Tab,
    Card,
    Inline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "child", rename_all = "snake_case")]
pub enum LayoutChild {
    Field(FieldId),
    Container(FieldId),
    Ui(UiNode),
    Panel(PanelNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "child", rename_all = "snake_case")]
pub enum PanelChild {
    Field(FieldId),
    Container(FieldId),
    Ui(UiNode),
}
```

Build-time validation rules for secret/select/layout:
- secret: fields with `SecurityPolicy` must set `redact_in.logs = true` and `redact_in.diagnostics = true`.
- secret: `default` is disallowed for secret-like kinds (`Password`, `ApiKey`, `BearerToken`) in production schemas.
- select: `default_single` and `default_multiple` are mutually exclusive.
- select: for static source, defaults must exist in option set.
- layout: `PanelChild` cannot include `Panel`/`Group`; `LayoutChild` allows `Panel` only under `Group`.
- layout: cycles in layout tree are rejected at compile time.

## 3. Mapping: NodePropertyTypes -> v2 (typed)

| NodePropertyType | v2 representation | Safety note |
|---|---|---|
| boolean | `Field(ValueSpec::Boolean)` | direct |
| button | `Ui(UiNode::ActionButton)` | no fake field value |
| collection | `Field(ValueSpec::Object)` | typed nested fields |
| color | `Field(ValueSpec::Text + subtype color_hex)` | validated pattern |
| dateTime | `Field(ValueSpec::Text + subtype datetime_iso8601)` | validated format |
| fixedCollection | `Field(ValueSpec::List<Object>)` | typed repeated objects |
| hidden | `Field.presentation.visibility = Hidden` | explicit policy |
| icon | `Field(ValueSpec::Object/IconValue)` | no free-form blob |
| json | `Field(ValueSpec::Object)` | safer than raw string |
| callout | `Ui(UiNode::Callout)` | UI-only node |
| notice | `Ui(UiNode::Notice)` | UI-only node |
| multiOptions | `Field(ValueSpec::Select { multiple: true })` | direct |
| number | `Field(ValueSpec::Number + NumberKind)` | int/decimal preserved |
| options | `Field(ValueSpec::Select { multiple: false })` | direct |
| string | `Field(ValueSpec::Text)` | direct |
| credentialsSelect | `Field(Text/Select) + SecurityPolicy` | restricted expressions |
| resourceLocator | `Field(Object/ResourceLocatorValue)` | tagged enum modes |
| curlImport | `Ui(UiNode::CurlImport)` | typed action target |
| resourceMapper | `Field(ValueSpec::Object)` | typed mapping payload |
| filter | `Field(ValueSpec::Object)` | typed filter DSL |
| assignmentCollection | `Field(ValueSpec::List<Object>)` | typed rows |
| credentials | `Field(ValueSpec::Object) + SecurityPolicy` | redaction first |
| workflowSelector | `Field(Text or Dynamic Select)` | typed source contract |

## 4. Dynamic Source Contract (typed, async, deterministic)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum OptionSource {
    Static { options: Vec<SelectOption> },
    Dynamic(DynamicSource),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicSource {
    pub provider_key: String,
    pub strategy: OptionLoadStrategy,
    pub depends_on: Vec<ValuePath>,
    pub cache: CachePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum OptionLoadStrategy {
    FullList,
    ListSearch { filter_required: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePolicy {
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionPage {
    pub options: Vec<SelectOption>,
    pub pagination_token: Option<String>,
    pub has_more: bool,
}

#[async_trait::async_trait]
pub trait OptionProvider: Send + Sync {
    fn key(&self) -> &str;

    async fn resolve(
        &self,
        request: &OptionRequest,
        query: Option<&OptionQuery>,
    ) -> Result<OptionPage, OptionProviderError>;
}
```

Required behavior:
- `depends_on` change invalidates only dependent caches.
- Hidden/unresolved dependency produces empty options for dependents.
- Dependency cycles fail schema compilation.

## 5. Resource Locator (no invalid states)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ResourceLocatorValue {
    Id {
        value: String,
        cached_result_name: Option<String>,
    },
    Url {
        value: String,
        cached_result_name: Option<String>,
        cached_result_url: Option<String>,
    },
    List {
        value: String,
        cached_result_name: Option<String>,
        cached_result_url: Option<String>,
    },
    Custom {
        id: String,
        value: String,
    },
}
```

## 6. Example: Clean v2 schema (cross-type)

```rust
Schema {
    nodes: vec![
        SchemaNode::Field(
            FieldDef::text("endpoint")
                .label("Endpoint")
                .required(true)
                .build()
        ),
        SchemaNode::Field(
            FieldDef::number("timeout_ms")
                .label("Timeout (ms)")
                .integer(IntBits::U32)
                .default_json(30_000)
                .min_json(100)
                .max_json(120_000)
                .build()
        ),
        SchemaNode::Field(
            FieldDef::boolean("retry_enabled")
                .label("Enable Retry")
                .default_json(false)
                .build()
        ),
        SchemaNode::Field(
            FieldDef::text("transform_code")
                .label("Transform Code")
                .editor(EditorConfig::JavaScript)
                .default_json("return $json;")
                .build()
        ),
        SchemaNode::Field(
            FieldDef::select("method")
                .label("Method")
                .multiple(false)
                .options_static(vec![
                    SelectOption::kv("GET", "GET"),
                    SelectOption::kv("POST", "POST"),
                    SelectOption::kv("PUT", "PUT"),
                    SelectOption::kv("DELETE", "DELETE"),
                ])
                .build()
        ),
        SchemaNode::Field(
            FieldDef::select("environment")
                .label("Environment")
                .multiple(false)
                .options_dynamic(DynamicSource {
                    provider_key: "org.environments".into(),
                    strategy: OptionLoadStrategy::ListSearch { filter_required: false },
                    depends_on: vec![ValuePath::parse("project")?],
                    cache: CachePolicy { ttl_ms: Some(30_000) },
                })
                .build()
        ),
        SchemaNode::Field(
            FieldDef::object("project")
                .label("Project")
                .resource_locator()
                .build()
        ),
        SchemaNode::Ui(
            UiNode::Notice(NoticeNode {
                id: "rate_limit_notice".into(),
                text: "API may return 429. Enable retry for stability.".into(),
                severity: NoticeSeverity::Warning,
                visible_if: vec![],
            })
        ),
        SchemaNode::Ui(
            UiNode::ActionButton(ActionButtonNode {
                id: "refresh_schema".into(),
                label: "Refresh Schema".into(),
                action: UiAction::RefreshSchema,
                enabled_if: vec![],
            })
        ),
    ]
}
```

## 7. Safety Invariants (must hold)

1. UI-only nodes never appear in runtime value payload.
2. Security policies are attached to value fields only.
3. All dynamic option dependencies are acyclic.
4. Error ordering is deterministic.
5. Secret-bearing field errors are redacted in all channels.
6. Unsupported UI controls fail fast at schema compile time.
7. Container build errors are deterministic and include precise field paths.
8. Secret fields are always write-only on transport boundaries.
9. Layout hierarchy violations are compile-time schema errors.

## 8. DX Acceptance Gates (Rust-grade)

1. Authoring: no ad-hoc `hint` strings required for core controls.
2. Refactorability: symbol rename works across schema API without string parsing.
3. Safety: impossible to encode invalid resource-locator mode combinations.
4. Plugin ergonomics: one async provider trait for both full list and search.
5. Diagnostics: compile-time schema errors are explicit and path-aware.

## 9. Plan Delta from previous playground

1. Replaced `UiHints.control = "..."` with typed `UiNode` enum.
2. Removed sentinel pseudo-fields for notices/buttons.
3. Replaced editor hint strings with `EditorConfig` enum.
4. Made dynamic options fully typed (`DynamicSource`, `OptionLoadStrategy`, `CachePolicy`).
5. Elevated safety invariants and acceptance gates to first-class plan artifacts.

## 10. Developer API Usage (how teams will actually use it)

This section shows practical authoring and runtime flow, not only type definitions.

### 10.1 Define a schema with conditional display and secret policy

```rust
let schema = Schema::builder("http.request")
    .field(
        FieldDef::select("auth_mode")
            .label("Auth Mode")
            .options_static(vec![
                SelectOption::kv("none", "None"),
                SelectOption::kv("bearer", "Bearer Token"),
            ])
            .default_json("none")
            .required(true)
            .build()
    )
    .field(
        FieldDef::text("bearer_token")
            .label("Bearer Token")
            .security(SecurityPolicy {
                kind: SecretKind::BearerToken,
                write_only: true,
                persist: PersistPolicy::SkipSave,
                redact_in: RedactionTargets {
                    logs: true,
                    diagnostics: true,
                    api_responses: true,
                    events: true,
                },
            })
            .visible_if(ExpressionRule::eq(
                ValuePath::parse("auth_mode")?,
                serde_json::json!("bearer"),
            ))
            .required_if(ExpressionRule::eq(
                ValuePath::parse("auth_mode")?,
                serde_json::json!("bearer"),
            ))
            .build()
    )
    .ui(UiNode::Notice(NoticeNode {
        id: "auth_notice".into(),
        text: "Token field appears only for Bearer mode".into(),
        severity: NoticeSeverity::Info,
        visible_if: vec![],
    }))
    .build()?;
```

### 10.2 Compile schema once (static guarantees)

```rust
let compiled = SchemaCompiler::new()
    .with_expression_policy(ExpressionPolicy::Restricted)
    .compile(schema)?;

// Compile step verifies:
// - field paths in expressions exist
// - dependency graph has no cycles
// - default values match declared types
// - secret policy invariants hold
```

### 10.3 Create form runtime and apply user input

```rust
let mut runtime = FormRuntime::new(compiled, RuntimeConfig {
    validate_when_hidden: ValidateWhenHidden::IfHasValue,
})?;

runtime.set_value("auth_mode", serde_json::json!("bearer"))?;

let ui_state = runtime.ui_state();
assert!(ui_state.field("bearer_token").visible);
```

### 10.4 Run validation and return deterministic diagnostics

```rust
let report = runtime.validate()?;

for err in report.errors() {
    println!("{} {} {}", err.path(), err.code(), err.message());
}

// Secret errors are redacted automatically.
// Example message: "invalid secret format" (never contains token value).
```

### 10.5 Provide dynamic options with one async provider interface

```rust
pub struct EnvironmentsProvider;

#[async_trait::async_trait]
impl OptionProvider for EnvironmentsProvider {
    fn key(&self) -> &str {
        "org.environments"
    }

    async fn resolve(
        &self,
        request: &OptionRequest,
        query: Option<&OptionQuery>,
    ) -> Result<OptionPage, OptionProviderError> {
        let project = request.value_at("project");
        let search = query.and_then(|q| q.search_text());

        let options = fetch_env_options(project, search).await?;
        Ok(OptionPage {
            options,
            pagination_token: None,
            has_more: false,
        })
    }
}
```

### 10.6 Typical integration sequence in app code

1. Register option providers in runtime registry.
2. Compile schemas at startup or plugin load.
3. Create `FormRuntime` per editing session.
4. On each user change: `set_value` -> recompute visibility for dependents only.
5. On submit: `validate` -> `redact` -> persist allowed fields.

This gives developers one consistent API surface for display logic, dynamic options, and validation.
