# Parameter Prototypes — API Validation

> **Purpose:** Validate nebula-parameter v2 HLD against real-world schemas.
> Each prototype is a complete builder call: ParameterCollection construction,
> value examples, validation expectations.
> Comments mark friction points, missing API, and "this works well" confirmations.

---

## 1. Telegram Bot — resource → operation pattern

The most common workflow-engine pattern. Tests: conditional visibility,
multiple operations sharing chat_id, Mode for reply markup, file upload,
mixed required/optional per operation.

### Schema

```rust
use nebula_parameter::prelude::*;
use serde_json::json;

fn telegram_bot_schema() -> ParameterCollection {
    ParameterCollection::new()
        // ── Resource / Operation selects ────────────────────────────
        .add(Parameter::select("resource")
            .label("Resource")
            .option("message", "Message")
            .option("chat", "Chat")
            .option("callback", "Callback Query")
            .default(json!("message"))
            .required())
        // ✅ WORKS WELL: .option(value, label) is clean shorthand.
        .add(Parameter::select("operation")
            .label("Operation")
            .option("sendMessage", "Send Message")
            .option("sendPhoto", "Send Photo")
            .option("sendDocument", "Send Document")
            .option("sendLocation", "Send Location")
            .option("sendContact", "Send Contact")
            .option("editMessageText", "Edit Message Text")
            .default(json!("sendMessage"))
            .searchable()
            .required())

        // ── Shared field ────────────────────────────────────────────
        .add(Parameter::string("chat_id")
            .label("Chat ID")
            .description("Unique identifier for the target chat or @username")
            .required())
        // ✅ chat_id is always visible + required — no conditions needed.
        // In n8n this would need displayOptions on every operation. Our default is simpler.

        // ── sendMessage ─────────────────────────────────────────────
        .add(Parameter::string("text")
            .label("Text")
            .description("Text of the message (1–4096 characters)")
            .multiline()
            .active_when(Condition::eq("operation", json!("sendMessage")))
            .with_rule(Rule::MaxLength { max: 4096, message: None }))
        // ✅ WORKS WELL: .multiline() on string — no separate "textarea" type needed.

        .add(Parameter::select("parse_mode")
            .label("Parse Mode")
            .option("HTML", "HTML")
            .option("Markdown", "Markdown")
            .option("MarkdownV2", "MarkdownV2")
            .default(json!("HTML"))
            .visible_when(Condition::any(vec![
                Condition::eq("operation", json!("sendMessage")),
                Condition::eq("operation", json!("editMessageText")),
            ])))
        // ✅ WORKS WELL: Condition::any for sharing field across operations.

        .add(Parameter::boolean("disable_notification")
            .label("Disable Notification")
            .description("Sends the message silently"))

        // ── sendPhoto ───────────────────────────────────────────────
        .add(Parameter::file("photo")
            .label("Photo")
            .accept("image/*")
            .active_when(Condition::eq("operation", json!("sendPhoto"))))
        // ✅ .accept() for MIME filter — maps to <input accept="image/*">

        // ── sendLocation ────────────────────────────────────────────
        .add(Parameter::number("latitude")
            .label("Latitude")
            .min(-90).max(90)
            .active_when(Condition::eq("operation", json!("sendLocation"))))
        .add(Parameter::number("longitude")
            .label("Longitude")
            .min(-180).max(180)
            .active_when(Condition::eq("operation", json!("sendLocation"))))
        // ⚠️ FRICTION: .min() / .max() take what type?
        // HLD says min: Option<serde_json::Number> on Number variant.
        // But builder .min(-90) needs to accept i32/f64/etc.
        // Need: impl From<i32> for serde_json::Number in builder.
        // Or: .min(json!(-90)) — ugly.
        // DECISION: Builder .min() / .max() / .step() accept impl Into<serde_json::Number>.
        // Provide From<i32>, From<f64>, From<u32> etc.

        // ── sendContact ─────────────────────────────────────────────
        .add(Parameter::string("phone_number")
            .label("Phone Number")
            .input_type("tel")
            .active_when(Condition::eq("operation", json!("sendContact"))))
        .add(Parameter::string("first_name")
            .label("First Name")
            .active_when(Condition::eq("operation", json!("sendContact"))))
        .add(Parameter::string("last_name")
            .label("Last Name")
            .visible_when(Condition::eq("operation", json!("sendContact"))))
        // ✅ .input_type("tel") — maps directly to <input type="tel">

        // ── Reply markup (Mode) ─────────────────────────────────────
        .add(Parameter::mode("reply_markup")
            .label("Reply Markup")
            .variant("none", "None",
                Parameter::hidden("_placeholder"))
            .variant("inline", "Inline Keyboard",
                // ⚠️ FRICTION: keyboard JSON should be Code { language: "json" },
                // but Mode variant content is a single Parameter.
                // Option A: Parameter::code("inline_keyboard_json").language("json")
                // Option B: Parameter::string("...").multiline() — loses highlighting.
                // DECISION: Use Code type. Builder needs .language() method.
                Parameter::code("inline_keyboard_json")
                    .label("Inline Keyboard JSON")
                    .language("json"))
            .variant("reply", "Reply Keyboard",
                Parameter::code("reply_keyboard_json")
                    .label("Reply Keyboard JSON")
                    .language("json"))
            .default_variant("none"))
        // ✅ WORKS WELL: Mode is natural for keyboard selection.
}
```

### Expected values

```json
{
  "resource": "message",
  "operation": "sendMessage",
  "chat_id": "123456789",
  "text": "Hello from Nebula!",
  "parse_mode": "HTML",
  "disable_notification": false,
  "reply_markup": { "mode": "none" }
}
```

### Validation notes

- ✅ Resource/operation pattern works with pure selects + conditions. No special mechanism.
- ✅ Shared fields (chat_id) just exist without conditions — always visible.
- ✅ Mode for reply markup — clean discriminated union.
- ✅ **Resolved:** `.active_when()` shorthand replaces paired `.visible_when()` + `.required_when()`.
  Clean: `.active_when(Condition::eq("operation", json!("sendPhoto")))` — one line.
- ⚠️ **Friction:** `.min()` / `.max()` type ergonomics need From impls for numeric types.

---

## 2. HTTP Request — complex node with auth, headers, body

The "kitchen sink" node. Tests: nested objects in Mode variants, List of
key-value pairs, Code editor for body, conditional body visibility,
deeply nested auth flows.

### Schema

```rust
fn http_request_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::string("url")
            .label("URL")
            .placeholder("https://api.example.com/endpoint")
            .input_type("url")
            .required()
            .with_rule(Rule::Pattern {
                pattern: r"^https?://".to_owned(),
                message: Some("Must start with http:// or https://".to_owned()),
            }))
        .add(Parameter::select("method")
            .label("Method")
            .option("GET", "GET")
            .option("POST", "POST")
            .option("PUT", "PUT")
            .option("PATCH", "PATCH")
            .option("DELETE", "DELETE")
            .option("HEAD", "HEAD")
            .option("OPTIONS", "OPTIONS")
            .default(json!("GET"))
            .required())

        // ── Authentication ──────────────────────────────────────────
        .add(Parameter::mode("authentication")
            .label("Authentication")
            .variant("none", "None",
                Parameter::hidden("_auth_none"))
            .variant("basic", "Basic Auth",
                Parameter::object("basic_auth")
                    .add(Parameter::string("username").label("Username").required())
                    .add(Parameter::string("password").label("Password")
                        .input_type("password").secret().required()))
            .variant("bearer", "Bearer Token",
                Parameter::string("token").label("Token").secret().required())
            .variant("api_key", "API Key",
                Parameter::object("api_key_config")
                    .add(Parameter::string("key_name")
                        .label("Header Name")
                        .default(json!("X-API-Key"))
                        .required())
                    .add(Parameter::string("key_value")
                        .label("API Key").secret().required()))
            .default_variant("none"))
        // ✅ WORKS WELL: Mode with Object variant content — deeply nested, reads well.

        // ── Headers ─────────────────────────────────────────────────
        .add(Parameter::list("headers")
            .label("Headers")
            .item(Parameter::object("header")
                .add(Parameter::string("name").label("Name").required())
                .add(Parameter::string("value").label("Value").required()))
            .sortable())
        // ✅ WORKS WELL: List of Objects — the universal key-value pattern.

        // ── Query parameters ────────────────────────────────────────
        .add(Parameter::list("query_params")
            .label("Query Parameters")
            .item(Parameter::object("param")
                .add(Parameter::string("name").label("Name").required())
                .add(Parameter::string("value").label("Value").required()))
            .sortable())
        // ⚠️ FRICTION: Headers and query_params have identical structure.
        // No way to reuse the item template. Must copy-paste the object definition.
        // Not a schema-level problem — Rust functions solve this:
        //   fn key_value_item(id: &str) -> Parameter { Parameter::object(id).add(...).add(...) }
        // But HLD should mention this pattern in docs/examples.

        // ── Body ────────────────────────────────────────────────────
        .add(Parameter::mode("body")
            .label("Body")
            .visible_when(Condition::ne("method", json!("GET")))
            .variant("none", "None",
                Parameter::hidden("_body_none"))
            .variant("json", "JSON",
                Parameter::code("json_body")
                    .label("JSON Body")
                    .language("json"))
            .variant("form", "Form Data",
                Parameter::list("form_fields")
                    .item(Parameter::object("field")
                        .add(Parameter::string("key").label("Key").required())
                        .add(Parameter::string("value").label("Value").required()))
                    .sortable())
            .variant("raw", "Raw",
                Parameter::string("raw_body")
                    .label("Raw Body")
                    .multiline())
            .variant("binary", "Binary / File",
                Parameter::file("binary_body")
                    .label("File"))
            .default_variant("none"))
        // ✅ WORKS WELL: Mode with List inside a variant — works.
        // ✅ Mode visible_when — the entire body section hides for GET.

        // ── Response ────────────────────────────────────────────────
        .add(Parameter::select("response_format")
            .label("Response Format")
            .option("auto", "Auto-detect")
            .option("json", "JSON")
            .option("text", "Text")
            .option("binary", "Binary")
            .default(json!("auto")))

        // ── Advanced ────────────────────────────────────────────────
        .add(Parameter::object("advanced")
            .label("Advanced Settings")
            .collapsed()
            .add(Parameter::integer("timeout_ms")
                .label("Timeout (ms)")
                .default(json!(30000))
                .min(0).max(300_000))
            .add(Parameter::boolean("follow_redirects")
                .label("Follow Redirects")
                .default(json!(true)))
            .add(Parameter::integer("max_redirects")
                .label("Max Redirects")
                .default(json!(10))
                .min(0).max(50)
                .visible_when(Condition::eq("follow_redirects", json!(true))))
            .add(Parameter::boolean("ignore_ssl_errors")
                .label("Ignore SSL Errors")
                .default(json!(false)))
            .add(Parameter::string("proxy_url")
                .label("Proxy URL")
                .input_type("url")))
        // ✅ WORKS WELL: collapsed Object = "Advanced" section. No separate Group type needed.
        // ✅ RESOLVED: Scope resolution rules in HLD — conditions resolve
        // relative to parent scope. "follow_redirects" here is a sibling
        // within "advanced" object. To reference root, use ParameterPath::root("method").
        // 95% of cases are sibling references (just &str), $root is the escape hatch.
}
```

### Expected values

```json
{
  "url": "https://api.example.com/users",
  "method": "POST",
  "authentication": {
    "mode": "bearer",
    "value": { "token": "sk-abc123" }
  },
  "headers": [
    { "name": "Content-Type", "value": "application/json" },
    { "name": "Accept", "value": "application/json" }
  ],
  "query_params": [],
  "body": {
    "mode": "json",
    "value": { "json_body": "{ \"name\": \"Alice\" }" }
  },
  "response_format": "auto",
  "advanced": {
    "timeout_ms": 30000,
    "follow_redirects": true,
    "max_redirects": 10,
    "ignore_ssl_errors": false
  }
}
```

### Validation notes

- ✅ Deep nesting (Mode → Object → fields) composes naturally.
- ✅ List inside Mode variant works (form data fields).
- ✅ Collapsed Object for advanced settings — clean.
- ✅ **Resolved: Condition scope.** Scope resolution rules added to HLD — conditions resolve relative to parent scope. `"follow_redirects"` inside `advanced` Object = `advanced.follow_redirects`. Use `ParameterPath::root("x")` for cross-scope.
- ⚠️ **Reusable templates.** Key-value list pattern appears 3x (headers, query_params, form_fields). Rust functions handle this, but docs should show the pattern.

---

## 3. Google Sheets — dynamic loading, depends_on chains

Tests: chained dynamic selects (spreadsheet → sheet → column),
OptionLoader with credential, DynamicFields for row data.

### Schema

```rust
fn google_sheets_append_row() -> ParameterCollection {
    ParameterCollection::new()
        // ── Spreadsheet (dynamic, loaded from Drive API) ────────────
        .add(Parameter::select("spreadsheet_id")
            .label("Spreadsheet")
            .searchable()
            .loader(|ctx: LoaderContext| async move {
                let token = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No credential"))?;
                let sheets = google_drive::list_spreadsheets(token).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                Ok(sheets.into_iter()
                    .map(|s| SelectOption::new(json!(s.id), &s.name))
                    .collect())
            })
            .required())
        // ✅ WORKS WELL: .loader() auto-sets dynamic=true. Clean.

        // ── Sheet tab (depends on spreadsheet) ──────────────────────
        .add(Parameter::select("sheet_name")
            .label("Sheet")
            .depends_on(&["spreadsheet_id"])
            .loader(|ctx: LoaderContext| async move {
                let spreadsheet_id = ctx.values.get_string("spreadsheet_id")
                    .ok_or(LoaderError::new("Select a spreadsheet first"))?;
                let token = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No credential"))?;
                let tabs = google_sheets::list_sheets(token, spreadsheet_id).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                Ok(tabs.into_iter()
                    .map(|t| SelectOption::new(json!(t.title), &t.title))
                    .collect())
            })
            .required())
        // ✅ WORKS WELL: depends_on triggers reload when spreadsheet changes.
        // ✅ LoaderError message "Select a spreadsheet first" — good UX.

        // ── Row data (dynamic fields from sheet columns) ────────────
        .add(Parameter::dynamic("row_data")
            .label("Row Data")
            .depends_on(&["spreadsheet_id", "sheet_name"])
            .loader(|ctx: LoaderContext| async move {
                let spreadsheet_id = ctx.values.get_string("spreadsheet_id")
                    .ok_or(LoaderError::new("Select a spreadsheet first"))?;
                let sheet_name = ctx.values.get_string("sheet_name")
                    .ok_or(LoaderError::new("Select a sheet first"))?;
                let token = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No credential"))?;
                let columns = google_sheets::get_columns(token, spreadsheet_id, sheet_name).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                Ok(columns.into_iter()
                    .map(|col| Parameter::string(&col.header)
                        .label(&col.header)
                        .description(format!("Column {}", col.letter)))
                    .collect())
            }))
        // ✅ WORKS WELL: Dynamic fields — columns become form fields.
        // RecordLoader returns Vec<Parameter> — engine renders them inline.
        // ✅ RESOLVED: depends_on semantics = "re-trigger when any changes."
        // Loader receives all current values via ctx.values and decides
        // itself whether it has enough context. If not, returns LoaderError
        // with helpful message ("Select a spreadsheet first").

        // ── Options ─────────────────────────────────────────────────
        .add(Parameter::select("value_input_option")
            .label("Value Input Option")
            .option("RAW", "Raw (values stored as-is)")
            .option("USER_ENTERED", "User Entered (parsed as if typed)")
            .default(json!("USER_ENTERED")))
        .add(Parameter::select("insert_data_option")
            .label("Insert Data Option")
            .option("INSERT_ROWS", "Insert Rows")
            .option("OVERWRITE", "Overwrite")
            .default(json!("INSERT_ROWS")))
}
```

### Expected values

```json
{
  "spreadsheet_id": "1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgVE2upms",
  "sheet_name": "Sheet1",
  "row_data": {
    "Name": "Alice",
    "Email": "alice@example.com",
    "Score": "95"
  },
  "value_input_option": "USER_ENTERED",
  "insert_data_option": "INSERT_ROWS"
}
```

### Validation notes

- ✅ Chained dynamic selects with depends_on — natural.
- ✅ DynamicFields for sheet columns — correct pattern.
- ✅ LoaderError with `.retryable()` — good for transient failures.
- ✅ **Resolved: depends_on** = trigger on any change, loader decides readiness via ctx.values.
- ✅ **Resolved: Dynamic values** nested under parent ID as object: `"row_data": { "Name": "Alice" }`. No collision with static fields.

---

## 4. Postgres Credential — simple credential form with secret fields

Tests: all fields required, password masked, SSL mode select,
port with integer constraint.

### Schema

```rust
fn postgres_credential_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::string("host")
            .label("Host")
            .placeholder("localhost")
            .input_type("hostname")   // ⚠️ Is "hostname" a valid HTML input type? No.
                                      // It's just "text". input_type is for HTML types only.
                                      // DECISION: Don't use input_type for non-HTML types.
                                      // Just use .label() and .placeholder() for context.
            .required())
        .add(Parameter::integer("port")
            .label("Port")
            .default(json!(5432))
            .min(1).max(65535)
            .required())
        .add(Parameter::string("database")
            .label("Database")
            .placeholder("postgres")
            .required())
        .add(Parameter::string("username")
            .label("Username")
            .required())
        .add(Parameter::string("password")
            .label("Password")
            .input_type("password")
            .secret()
            .required())
        // ⚠️ QUESTION: .secret() and .input_type("password") — redundant?
        // .secret() = exclude from logs/debug. .input_type("password") = mask in UI.
        // They COULD be independent: .secret() without password input (e.g. API key
        // shown in plaintext but excluded from logs). But 99% of cases they go together.
        // DECISION: Keep both. .secret() is backend concern (logging/storage).
        // .input_type("password") is frontend concern (masking). Independent.

        .add(Parameter::select("ssl_mode")
            .label("SSL Mode")
            .option("disable", "Disable")
            .option("prefer", "Prefer")
            .option("require", "Require")
            .option("verify-ca", "Verify CA")
            .option("verify-full", "Verify Full")
            .default(json!("prefer")))
        .add(Parameter::string("ssl_ca_cert")
            .label("CA Certificate")
            .multiline()
            .visible_when(Condition::any(vec![
                Condition::eq("ssl_mode", json!("verify-ca")),
                Condition::eq("ssl_mode", json!("verify-full")),
            ])))
        // ✅ WORKS WELL: Condition for CA cert — only when verify mode.

        // ── Connection string preview ───────────────────────────────
        .add(Parameter::computed("connection_preview")
            .label("Connection String")
            .returns_string()
            .expression("postgres://{{ username }}@{{ host }}:{{ port }}/{{ database }}"))
        // ✅ WORKS WELL: Computed preview — user sees what the connection will look like.
        // ⚠️ NOTE: password intentionally excluded from preview (secret).
}
```

### Expected values

```json
{
  "host": "db.example.com",
  "port": 5432,
  "database": "myapp",
  "username": "admin",
  "password": "supersecret",
  "ssl_mode": "require"
}
```

### Validation notes

- ✅ Simple required fields — clean.
- ✅ Computed connection preview — good UX for credentials.
- ✅ SSL cert conditional visibility — natural.
- ⚠️ **input_type misuse.** `"hostname"` is not valid HTML input type. Document that input_type is strictly HTML `<input type="...">` values only.
- ⚠️ **.secret() vs .input_type("password")** — document that they're independent concerns.

---

## 5. Postgres Resource Config — operational settings with computed fields

Tests: all optional with defaults, computed connection string, min/max on integers.

### Schema

```rust
fn postgres_resource_config() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::integer("connect_timeout_ms")
            .label("Connect Timeout (ms)")
            .default(json!(5000))
            .min(0).max(60_000))
        .add(Parameter::integer("statement_timeout_ms")
            .label("Statement Timeout (ms)")
            .default(json!(30_000))
            .min(0))
        // ⚠️ QUESTION: .min(0) without .max() — is this allowed?
        // Number variant has min/max as Option<Number>. Yes, one without other is fine.
        // ✅ CONFIRMED: asymmetric bounds work.

        .add(Parameter::integer("pool_size")
            .label("Pool Size")
            .default(json!(10))
            .min(1).max(100))
        .add(Parameter::string("application_name")
            .label("Application Name")
            .default(json!("nebula")))
        .add(Parameter::string("search_path")
            .label("Search Path")
            .placeholder("public"))
        .add(Parameter::select("recycle_method")
            .label("Recycle Method")
            .option("full", "Full (DISCARD ALL always)")
            .option("smart", "Smart (DISCARD ALL only if needed)")
            .default(json!("smart")))
        // ✅ WORKS WELL: all fields optional with sensible defaults. 
        // normalize() backfills, user only overrides what they care about.
}
```

### Validation notes

- ✅ All optional with defaults — cleanest schema pattern.
- ✅ No required fields — normalize() does all the work.

---

## 6. If/Switch — control flow nodes with Mode for condition type

Tests: Mode with Object variant (compare sub-fields), expression field.

### Schema

```rust
fn if_action_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::mode("condition")
            .label("Condition")
            .required()
            .variant("expression", "Expression",
                Parameter::string("expr")
                    .label("Expression")
                    .placeholder("{{ $input.value > 0 }}")
                    .required())
            .variant("compare", "Compare Values",
                Parameter::object("compare")
                    .add(Parameter::string("left").label("Left Value").required())
                    .add(Parameter::select("operator")
                        .label("Operator")
                        .option("eq", "equals (=)")
                        .option("ne", "not equals (≠)")
                        .option("gt", "greater than (>)")
                        .option("lt", "less than (<)")
                        .option("gte", "≥")
                        .option("lte", "≤")
                        .option("contains", "contains")
                        .option("matches", "matches regex")
                        .default(json!("eq"))
                        .required())
                    .add(Parameter::string("right").label("Right Value").required()))
            .default_variant("expression"))
        // ✅ WORKS WELL: Mode perfectly models "expression OR structured compare".
        // Object inside Mode variant — multi-field variant content.
}

fn switch_action_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::string("value")
            .label("Value to Switch On")
            .description("Expression or field reference")
            .required())
        .add(Parameter::list("cases")
            .label("Cases")
            .item(Parameter::object("case")
                .add(Parameter::string("match_value")
                    .label("Matches")
                    .placeholder("exact value or expression")
                    .required())
                .add(Parameter::string("output")
                    .label("Output Branch Name")
                    .required()
                    .with_rule(Rule::Pattern {
                        pattern: r"^[a-zA-Z_][a-zA-Z0-9_]*$".to_owned(),
                        message: Some("must be a valid identifier".to_owned()),
                    })))
            .min_items(1))
        .add(Parameter::string("fallback_output")
            .label("Fallback Branch")
            .default(json!("default"))
            .with_rule(Rule::Pattern {
                pattern: r"^[a-zA-Z_][a-zA-Z0-9_]*$".to_owned(),
                message: Some("must be a valid identifier".to_owned()),
            }))
        // ⚠️ FRICTION: Same Pattern rule applied to two fields. No way to define
        // a named rule and reference it. Must copy the Rule literal each time.
        // Rust solves this: fn identifier_rule() -> Rule { ... }
        // But schema JSON has no "named rules" — acceptable.
}

fn for_each_action_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::string("items")
            .label("Items")
            .description("Expression resolving to an array")
            .placeholder("{{ $input.rows }}")
            .required())
        .add(Parameter::string("item_variable")
            .label("Item Variable Name")
            .default(json!("item"))
            .with_rule(Rule::Pattern {
                pattern: r"^[a-zA-Z_][a-zA-Z0-9_]*$".to_owned(),
                message: Some("must be a valid identifier".to_owned()),
            }))
        .add(Parameter::integer("batch_size")
            .label("Batch Size")
            .description("Number of items to process concurrently (1 = sequential)")
            .default(json!(1))
            .min(1).max(100))
}

fn wait_action_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::mode("wait_mode")
            .label("Wait Until")
            .required()
            .variant("duration", "Fixed Duration",
                Parameter::object("duration")
                    .add(Parameter::integer("amount")
                        .label("Amount")
                        .default(json!(1))
                        .min(1)
                        .required())
                    .add(Parameter::select("unit")
                        .label("Unit")
                        .option("milliseconds", "Milliseconds")
                        .option("seconds", "Seconds")
                        .option("minutes", "Minutes")
                        .option("hours", "Hours")
                        .option("days", "Days")
                        .default(json!("seconds"))))
            .variant("timestamp", "Until Timestamp",
                Parameter::string("timestamp")
                    .label("Timestamp")
                    .description("ISO 8601 datetime or expression")
                    // ⚠️ QUESTION: Should this be DateTime type instead of String?
                    // DateTime type gives date picker, but user may want expression.
                    // String with .expression(true) allows both.
                    // DECISION: Use DateTime type. If user needs expression,
                    // they toggle to expression mode (frontend feature).
                    // Actually — we have .expression flag on Parameter struct.
                    // Let's use DateTime:
                    )
            .default_variant("duration"))
        // ⚠️ RETHINK: Replacing above String with DateTime.
        // But then .description() and .placeholder() on DateTime are useful.
        // Let's leave as String for now — timestamps as expressions are common.
}
```

### Validation notes

- ✅ Mode for condition type — natural discriminated union.
- ✅ Object inside Mode variant — multi-field structured conditions.
- ✅ List with min_items(1) — at least one case in switch.
- ⚠️ **Rule reuse.** Same pattern rule copied across fields. Rust functions solve it, but worth documenting the pattern.
- ⚠️ **DateTime vs String for timestamps.** When user needs expression, String is simpler. When user needs date picker, DateTime is better. Expression flag (`expression: true` on Parameter) may bridge this.

---

## 7. E-commerce Order — computed fields, complex validation

Tests: Computed type for derived values, cross-field dependencies,
List with unique constraint.

### Schema

```rust
fn ecommerce_order_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::string("customer_email")
            .label("Customer Email")
            .input_type("email")
            .required()
            .with_rule(Rule::Pattern {
                pattern: r"^[^@]+@[^@]+\.[^@]+$".to_owned(),
                message: Some("must be a valid email address".to_owned()),
            }))

        .add(Parameter::list("items")
            .label("Order Items")
            .item(Parameter::object("item")
                .add(Parameter::string("product_id")
                    .label("Product ID")
                    .required())
                .add(Parameter::string("product_name")
                    .label("Product Name")
                    .required())
                .add(Parameter::number("unit_price")
                    .label("Unit Price")
                    .min(0)
                    .required())
                .add(Parameter::integer("quantity")
                    .label("Quantity")
                    .min(1).max(999)
                    .default(json!(1))
                    .required())
                .add(Parameter::computed("line_total")
                    .label("Line Total")
                    .returns_number()
                    .expression("{{ unit_price * quantity }}")))
            .min_items(1)
            .sortable())
        // ✅ RESOLVED: Expression scope follows same rules as Condition scope —
        // resolves relative to parent. "unit_price" = THIS item's unit_price.
        // To reference root, use "$root.field" in expression: "{{ $root.tax_rate }}".

        .add(Parameter::string("coupon_code")
            .label("Coupon Code")
            .placeholder("SAVE20"))

        .add(Parameter::select("shipping_method")
            .label("Shipping Method")
            .option("standard", "Standard (5-7 days)")
            .option("express", "Express (2-3 days)")
            .option("overnight", "Overnight")
            .default(json!("standard"))
            .required())

        .add(Parameter::string("notes")
            .label("Order Notes")
            .multiline())

        // ⚠️ MISSING: Computed "order_total" that sums all line_totals.
        // Expression: "{{ items.map(i => i.line_total).sum() }}" — but this
        // requires array operations in expression engine. Not a parameter
        // crate problem, but shows expression engine must support collection ops.
}
```

### Validation notes

- ✅ List with computed inside items — powerful pattern.
- ✅ input_type("email") with Pattern rule — belt and suspenders.
- ✅ **Resolved: Expression scope.** Same as Condition scope — relative to parent. `{{ unit_price * quantity }}` inside list item = item-scoped. Use `{{ $root.tax_rate }}` for cross-scope.
- ⚠️ **Expression engine capability.** Order-total across list items requires collection operations. Not parameter crate's problem, but validate that expression syntax can handle it.

---

## 8. Filter Builder — the Filter parameter type

Tests: Filter type with operator restrictions, nested groups.

### Schema

```rust
fn email_filter_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::select("mailbox")
            .label("Mailbox")
            .option("INBOX", "Inbox")
            .option("SENT", "Sent")
            .option("DRAFTS", "Drafts")
            .option("TRASH", "Trash")
            .default(json!("INBOX"))
            .required())
        .add(Parameter::filter("conditions")
            .label("Filter Conditions")
            .operators(vec![
                FilterOp::Eq,
                FilterOp::Ne,
                FilterOp::Contains,
                FilterOp::IsSet,
                FilterOp::IsEmpty,
            ])
            .allow_groups(true)
            .max_depth(3))
        // ✅ WORKS WELL: Filter type with restricted operators.
        // Frontend renders visual condition builder.

        .add(Parameter::integer("max_results")
            .label("Max Results")
            .default(json!(50))
            .min(1).max(500))
}
```

### Expected Filter value

```json
{
  "mailbox": "INBOX",
  "conditions": {
    "kind": "group",
    "combinator": "and",
    "children": [
      {
        "kind": "rule",
        "field": "subject",
        "op": "contains",
        "value": "invoice"
      },
      {
        "kind": "rule",
        "field": "from",
        "op": "contains",
        "value": "@company.com"
      }
    ]
  },
  "max_results": 50
}
```

### Validation notes

- ✅ Filter type with operator whitelist — clean.
- ⚠️ **QUESTION:** Filter builders need to know available fields (subject, from, to, date, etc.). Where does this come from? Static list? Dynamic? The Filter variant has no `fields` config.
  **DECISION NEEDED:** Filter might need a `columns` or `fields` parameter that tells the builder which fields are filterable. Or this is frontend-supplied context, not schema.

---

## Cross-cutting validation summary

### What works well across all prototypes

| Aspect | Verdict |
|--------|---------|
| `Parameter::type("id").label("L").required()` fluent builder | ✅ Clean, consistent, readable |
| Select with `.option(value, label)` | ✅ Most natural shorthand |
| Mode for discriminated unions | ✅ Auth, body format, condition type, wait mode — all fit |
| List of Objects for key-value patterns | ✅ Headers, query params, form fields, order items |
| Condition::eq / Condition::any for visibility | ✅ Covers resource→operation pattern well |
| `.loader()` auto-setting `dynamic=true` | ✅ No redundant flag |
| LoaderError with context messages | ✅ "Select a spreadsheet first" — good UX |
| Collapsed Object for "Advanced" sections | ✅ No separate Group type needed |
| Computed as separate type | ✅ Connection preview, line totals |
| `.input_type("email"/"tel"/"url"/"password")` | ✅ Direct HTML mapping, simple |

### Friction points identified

| # | Issue | Affected | Severity | Status |
|---|-------|----------|----------|--------|
| 1 | `.visible_when()` + `.required_when()` always paired | Telegram, HTTP | Medium | ✅ **Resolved:** `.active_when()` shorthand added |
| 2 | `.min()` / `.max()` type ergonomics | Telegram, Postgres | Low | ✅ **Resolved:** `impl Into<serde_json::Number>` in HLD |
| 3 | Condition scope in nested Objects | HTTP (advanced), E-commerce | **Critical** | ✅ **Resolved:** ParameterPath + scope resolution rules |
| 4 | Expression scope in List items | E-commerce | **Critical** | ✅ **Resolved:** same as #3 — relative to parent, `$root` escape |
| 5 | Reusable key-value item template | HTTP | Low | ✅ **Resolved:** documented in DX Guidelines |
| 6 | Rule reuse across fields | Switch | Low | ✅ **Resolved:** documented in DX Guidelines |
| 7 | Filter field definitions | Email filter | Medium | Open — static list for v1, dynamic later |
| 8 | `.secret()` vs `.input_type("password")` independence | Postgres cred | Low | ✅ **Resolved:** documented in DX Guidelines |
| 9 | `input_type` misuse for non-HTML types | Postgres cred | Low | ✅ **Resolved:** documented in DX Guidelines |
| 10 | depends_on semantics ("all set" vs "any changed") | Google Sheets | Medium | ✅ **Resolved:** trigger on any change, defined in HLD |
| 11 | Dynamic field value namespace | Google Sheets | Medium | ✅ **Resolved:** nested under parent ID, defined in HLD |

### Architecture decisions — all resolved in HLD

| # | Decision | Resolution | Status |
|---|----------|------------|--------|
| D1 | Condition field resolution scope | Relative to parent + `ParameterPath::root()` escape | ✅ In HLD |
| D2 | Expression scope in list items | Same as D1 — relative to parent | ✅ In HLD |
| D3 | `.active_when()` shorthand | Added to Parameter impl | ✅ In HLD |
| D4 | depends_on semantics | Trigger on any change, loader decides readiness | ✅ In HLD |
| D5 | Dynamic field value location | Nested under parent ID as object | ✅ In HLD |
| D6 | Filter field definitions | Static list in v1, dynamic loader later | Open |
| D7 | LoaderContext metadata | Added `metadata: Option<Value>` to LoaderContext | ✅ In HLD |
| D8 | Dedicated type vs String+input_type | Prefer dedicated type, documented in DX Guidelines | ✅ In HLD |
| D9 | Constructors for all types | All 17 constructors listed in HLD | ✅ In HLD |

### API methods discovered — all added to HLD

| Method | On | Status |
|--------|-----|--------|
| `.active_when(condition)` | Parameter | ✅ Added |
| `.language(lang)` | Parameter (Code type) | ✅ Added |
| `.operators(ops)` | Parameter (Filter type) | ✅ Added |
| `.allow_groups(bool)` | Parameter (Filter type) | ✅ Added |
| `.max_depth(n)` | Parameter (Filter type) | ✅ Added |
| `.min(n)` / `.max(n)` | Parameter (Number type) | ✅ `impl Into<serde_json::Number>` |
| `Parameter::code(id)` | Constructor | ✅ Added |
| `Parameter::file(id)` | Constructor | ✅ Added |
| `Parameter::filter(id)` | Constructor | ✅ Added |
| `Parameter::date(id)` | Constructor | ✅ Added |
| `Parameter::datetime(id)` | Constructor | ✅ Added |
| `Parameter::time(id)` | Constructor | ✅ Added |
| `Parameter::color(id)` | Constructor | ✅ Added |
| `.retryable()` | LoaderError | ✅ Added |
| `LoaderContext.metadata` | LoaderContext | ✅ Added |

---
---

# Part 2 — Additional Prototypes

---

## 9. AI / LLM Node — slider, large text, dynamic model list

Tests: input_type("range") for temperature, Code for system prompt,
dynamic model list from API, large token limits, number step.

### Schema

```rust
fn ai_completion_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::select("provider")
            .label("Provider")
            .option("openai", "OpenAI")
            .option("anthropic", "Anthropic")
            .option("google", "Google Gemini")
            .option("ollama", "Ollama (Local)")
            .default(json!("openai"))
            .required())

        // ── Model (dynamic, depends on provider) ────────────────────
        .add(Parameter::select("model")
            .label("Model")
            .depends_on(&["provider"])
            .searchable()
            .loader(|ctx: LoaderContext| async move {
                let provider = ctx.values.get_string("provider")
                    .ok_or(LoaderError::new("Select a provider first"))?;
                let cred = ctx.credential.as_ref();
                let models = match provider {
                    "openai" => fetch_openai_models(cred).await?,
                    "anthropic" => fetch_anthropic_models(cred).await?,
                    "google" => fetch_google_models(cred).await?,
                    "ollama" => fetch_ollama_models(cred).await?,
                    _ => vec![],
                };
                Ok(models.into_iter()
                    .map(|m| {
                        let mut opt = SelectOption::new(json!(m.id), &m.name);
                        opt.description = Some(format!("{}K context", m.context_window / 1000));
                        opt
                    })
                    .collect())
            })
            // ⚠️ FRICTION: Loader closure has match on provider string.
            // This is action-level logic leaking into schema definition.
            // Alternative: register separate loaders per provider?
            // But then we'd need conditional loaders. Current approach is fine —
            // schema author owns the loader logic.
            .required())

        .add(Parameter::string("system_prompt")
            .label("System Prompt")
            .multiline()
            .placeholder("You are a helpful assistant...")
            .default(json!("You are a helpful assistant.")))
        // ⚠️ QUESTION: Should this be Code { language: "markdown" }?
        // System prompts aren't really code. Multiline string is correct.

        .add(Parameter::string("user_message")
            .label("User Message")
            .multiline()
            .required())

        // ── Temperature ─────────────────────────────────────────────
        .add(Parameter::number("temperature")
            .label("Temperature")
            .min(0.0).max(2.0).step(0.1)
            .default(json!(0.7))
            .input_type("range"))
        // ✅ WORKS WELL: input_type("range") + min/max/step → slider in frontend.
        // ⚠️ FRICTION: .min(0.0) — does builder accept f64?
        // Same issue as #2 from part 1. Need impl Into<serde_json::Number> for f64.

        .add(Parameter::integer("max_tokens")
            .label("Max Tokens")
            .default(json!(1024))
            .min(1).max(128_000)
            .description("Maximum number of tokens to generate"))

        // ── Advanced ────────────────────────────────────────────────
        .add(Parameter::object("advanced")
            .label("Advanced")
            .collapsed()
            .add(Parameter::number("top_p")
                .label("Top P")
                .min(0.0).max(1.0).step(0.05)
                .default(json!(1.0)))
            .add(Parameter::number("frequency_penalty")
                .label("Frequency Penalty")
                .min(-2.0).max(2.0).step(0.1)
                .default(json!(0.0)))
            .add(Parameter::number("presence_penalty")
                .label("Presence Penalty")
                .min(-2.0).max(2.0).step(0.1)
                .default(json!(0.0)))
            .add(Parameter::list("stop_sequences")
                .label("Stop Sequences")
                .item(Parameter::string("stop"))
                .max_items(4)
                .unique())
            // ✅ WORKS WELL: List of plain strings — not everything is key-value.
            .add(Parameter::code("response_format")
                .label("Response Format (JSON Schema)")
                .language("json")))
}
```

### Expected values

```json
{
  "provider": "openai",
  "model": "gpt-4o",
  "system_prompt": "You are a helpful assistant.",
  "user_message": "Explain quantum computing in simple terms.",
  "temperature": 0.7,
  "max_tokens": 1024,
  "advanced": {
    "top_p": 1.0,
    "frequency_penalty": 0.0,
    "presence_penalty": 0.0,
    "stop_sequences": ["\n\n"],
    "response_format": ""
  }
}
```

### Validation notes

- ✅ Slider via `input_type("range")` + min/max/step — clean mapping.
- ✅ Dynamic model list with depends_on provider — natural chain.
- ✅ List of plain strings (stop sequences) — simpler than List of Objects.
- ✅ SelectOption with `.description` for context window info — nice UX.
- ⚠️ `.min(0.0)` / `.step(0.1)` — f64 ergonomics for builder.

---

## 10. Slack Send Message — dynamic channels, mentions, rich text blocks

Tests: dynamic select with static fallback, multiple select (mentions),
Condition with multiple values, optional attachments list.

### Schema

```rust
fn slack_send_message_schema() -> ParameterCollection {
    ParameterCollection::new()
        // ── Channel (dynamic with static fallback) ──────────────────
        .add(Parameter::select("channel")
            .label("Channel")
            .option("#general", "#general")
            .option("#random", "#random")
            // Static options as fallback until loader responds
            .searchable()
            .loader(|ctx: LoaderContext| async move {
                let token = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No Slack token"))?;
                let channels = slack::list_channels(token).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                let filter = ctx.filter.as_deref().unwrap_or("").to_lowercase();
                Ok(channels.into_iter()
                    .filter(|c| filter.is_empty() || c.name.to_lowercase().contains(&filter))
                    .map(|c| SelectOption::new(json!(c.id), format!("#{}", c.name)))
                    .collect())
            })
            .required())
        // ✅ WORKS WELL: Static options as fallback + dynamic loader.
        // Frontend shows #general/#random immediately, then replaces with full list.
        // .searchable() + filter in loader — server-side filtering.

        // ── Message type ────────────────────────────────────────────
        .add(Parameter::mode("message_type")
            .label("Message Type")
            .variant("simple", "Simple Text",
                Parameter::string("text")
                    .label("Message")
                    .multiline()
                    .required())
            .variant("blocks", "Block Kit (JSON)",
                Parameter::code("blocks_json")
                    .label("Blocks JSON")
                    .language("json")
                    .required())
            // ⚠️ FRICTION: .required() inside Mode variant content —
            // is this "required when this variant is active" or "always required"?
            // Per Mode contract: only selected variant is validated.
            // So .required() inside variant = required when variant is selected.
            // ✅ CONFIRMED: this is correct behavior per HLD.
            .default_variant("simple"))

        // ── Mentions ────────────────────────────────────────────────
        .add(Parameter::select("mentions")
            .label("Mention Users")
            .multiple()
            .searchable()
            .loader(|ctx: LoaderContext| async move {
                let token = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No Slack token"))?;
                let users = slack::list_users(token).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                Ok(users.into_iter()
                    .map(|u| SelectOption::new(json!(u.id), format!("{} (@{})", u.name, u.handle)))
                    .collect())
            }))
        // ✅ WORKS WELL: .multiple() + .loader() — multi-select with dynamic users.

        // ── Optional fields ─────────────────────────────────────────
        .add(Parameter::string("thread_ts")
            .label("Thread Timestamp")
            .description("Reply in thread (provide parent message ts)")
            .placeholder("1234567890.123456"))

        .add(Parameter::boolean("unfurl_links")
            .label("Unfurl Links")
            .default(json!(true)))

        .add(Parameter::boolean("unfurl_media")
            .label("Unfurl Media")
            .default(json!(true)))

        // ── Attachments ─────────────────────────────────────────────
        .add(Parameter::list("attachments")
            .label("Attachments")
            .item(Parameter::object("attachment")
                .add(Parameter::string("title").label("Title"))
                .add(Parameter::string("text").label("Text"))
                .add(Parameter::string("color")
                    .label("Color")
                    .input_type("color")
                    // ⚠️ QUESTION: Should this be Color type or String with input_type("color")?
                    // Color type exists in ParameterType. But Color is a unit variant —
                    // no extra config. String with input_type("color") does the same thing
                    // from frontend perspective.
                    // DECISION: Use Color type when color IS the value.
                    // Use String + input_type("color") when it's a field within an object
                    // and you want to keep it as string. Actually — Color type also produces
                    // a string value. So they're equivalent. Use Color type for clarity:
                    )
                .add(Parameter::string("image_url").label("Image URL").input_type("url")))
            .max_items(20))
        // ⚠️ RETHINK: attachment.color should be Parameter::color("color").label("Color")
        // instead of string + input_type. Both produce string value, but Color type is
        // semantically clearer. Let's note this as a pattern guideline.
}
```

### Validation notes

- ✅ Static fallback + dynamic loader — solved UX for slow APIs.
- ✅ Multiple select with dynamic loader — multi-user mention.
- ✅ Mode for simple text vs Block Kit — natural.
- ✅ .required() inside Mode variant — means "required when active." Correct.
- ⚠️ **Pattern guideline:** Prefer `Parameter::color("x")` over `Parameter::string("x").input_type("color")`. Both work, but type is semantically stronger.
- ⚠️ **New question:** Color vs String+input_type — when both produce same value, which wins?

---

## 11. Cron Schedule Trigger — cron syntax, date-time, recurring patterns

Tests: Code type for cron, DateTime type, Mode for schedule type,
integer with step for intervals.

### Schema

```rust
fn schedule_trigger_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::mode("schedule_type")
            .label("Schedule Type")
            .required()
            .variant("interval", "Fixed Interval",
                Parameter::object("interval_config")
                    .add(Parameter::integer("amount")
                        .label("Every")
                        .default(json!(15))
                        .min(1)
                        .required())
                    .add(Parameter::select("unit")
                        .label("Unit")
                        .option("seconds", "Seconds")
                        .option("minutes", "Minutes")
                        .option("hours", "Hours")
                        .option("days", "Days")
                        .default(json!("minutes"))
                        .required()))
            .variant("cron", "Cron Expression",
                Parameter::string("cron_expression")
                    .label("Cron Expression")
                    .placeholder("0 */5 * * * *")
                    .required()
                    .with_rule(Rule::Pattern {
                        pattern: r"^(\S+\s+){4,5}\S+$".to_owned(),
                        message: Some("Must be a valid cron expression (5-6 fields)".to_owned()),
                    }))
                // ⚠️ QUESTION: Should cron be Code { language: "cron" } for highlighting?
                // Most cron expressions are short (one line). Code type implies editor.
                // String with pattern validation is better here.
                // DECISION: String + pattern rule. Code is for multi-line editing.
            .variant("specific_time", "Specific Time",
                Parameter::object("specific_config")
                    .add(Parameter::time("time")
                        .label("Time of Day")
                        .required())
                    .add(Parameter::select("days_of_week")
                        .label("Days")
                        .multiple()
                        .option("mon", "Monday")
                        .option("tue", "Tuesday")
                        .option("wed", "Wednesday")
                        .option("thu", "Thursday")
                        .option("fri", "Friday")
                        .option("sat", "Saturday")
                        .option("sun", "Sunday")
                        .default(json!(["mon", "tue", "wed", "thu", "fri"]))))
                // ✅ WORKS WELL: Time type + multi-select for days. Clean.
                // ✅ Multi-select default as JSON array — works with serde.
            .variant("once", "One-time",
                Parameter::datetime("run_at")
                    .label("Run At")
                    .required())
                // ✅ WORKS WELL: DateTime type for one-time schedule.
            .default_variant("interval"))

        .add(Parameter::select("timezone")
            .label("Timezone")
            .searchable()
            .allow_custom()
            .option("UTC", "UTC")
            .option("America/New_York", "Eastern Time")
            .option("America/Chicago", "Central Time")
            .option("America/Denver", "Mountain Time")
            .option("America/Los_Angeles", "Pacific Time")
            .option("Europe/London", "London")
            .option("Europe/Berlin", "Berlin")
            .option("Asia/Tokyo", "Tokyo")
            .default(json!("UTC")))
        // ✅ WORKS WELL: .allow_custom() for timezones not in common list.
        // User can type "Asia/Kolkata" even though it's not in options.

        .add(Parameter::computed("next_run_preview")
            .label("Next Run")
            .returns_string()
            .expression("{{ schedule_next_run(schedule_type) }}"))
        // ⚠️ QUESTION: Computed expression calls a function — does expression
        // engine support function calls? This is runtime's problem, not parameter crate.
        // But worth noting: computed expressions may need more than field references.
}
```

### Validation notes

- ✅ Mode perfectly models interval vs cron vs specific_time vs one-time.
- ✅ Time type for time-of-day — correct use.
- ✅ DateTime type for one-time schedule — correct use.
- ✅ Multi-select with array default for days of week — works.
- ✅ .allow_custom() for extensible timezone list.
- ⚠️ **Cron as String vs Code** — String is better for short expressions.
- ⚠️ **Computed with function calls** — expression engine capability question.

---

## 12. Data Mapper — source→target field mapping with dynamic sources

Tests: Dynamic on both sides (source and target), List of mapping pairs,
Mode for transformation type per mapping.

### Schema

```rust
fn data_mapper_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::list("mappings")
            .label("Field Mappings")
            .item(Parameter::object("mapping")
                // ⚠️ FRICTION: source_field should be a dynamic select loaded from
                // input data schema. But it's inside a List item — how does the loader
                // know which list index it's in? Does it matter?
                // Actually: loader gets ALL current form values, it just needs to load
                // the source schema once. All list items share the same options.
                // ✅ This works — loader doesn't need per-item context.
                .add(Parameter::select("source_field")
                    .label("Source Field")
                    .searchable()
                    .loader(|ctx: LoaderContext| async move {
                        // ✅ RESOLVED: Use ctx.metadata for engine-injected context,
                        // not ctx.values (which is user data only).
                        let meta = ctx.metadata.as_ref()
                            .ok_or(LoaderError::new("No input data available. Run the previous node first."))?;
                        let schema = meta.get("input_schema")
                            .ok_or(LoaderError::new("No input schema in metadata"))?;
                        let fields: Vec<String> = serde_json::from_value(schema.clone())
                            .map_err(|_| LoaderError::new("Invalid input schema"))?;
                        Ok(fields.into_iter()
                            .map(|f| SelectOption::new(json!(f), &f))
                            .collect())
                    })
                    .required())
                .add(Parameter::select("target_field")
                    .label("Target Field")
                    .searchable()
                    .allow_custom()  // user can type new field names
                    .loader(|ctx: LoaderContext| async move {
                        let meta = ctx.metadata.as_ref()
                            .ok_or(LoaderError::new("No target schema configured"))?;
                        let schema = meta.get("output_schema")
                            .ok_or(LoaderError::new("No output schema in metadata"))?;
                        let fields: Vec<String> = serde_json::from_value(schema.clone())
                            .map_err(|_| LoaderError::new("Invalid target schema"))?;
                        Ok(fields.into_iter()
                            .map(|f| SelectOption::new(json!(f), &f))
                            .collect())
                    })
                    .required())
                .add(Parameter::mode("transform")
                    .label("Transform")
                    .variant("none", "Direct Copy",
                        Parameter::hidden("_direct"))
                    .variant("expression", "Expression",
                        Parameter::string("expr")
                            .label("Expression")
                            .placeholder("{{ value.trim().toLowerCase() }}")
                            .required())
                    .variant("type_cast", "Type Cast",
                        Parameter::select("cast_to")
                            .label("Cast To")
                            .option("string", "String")
                            .option("number", "Number")
                            .option("boolean", "Boolean")
                            .option("date", "Date")
                            .required())
                    .default_variant("none")))
            .min_items(1)
            .sortable())
        // ✅ WORKS WELL: Complex list items with dynamic selects + Mode inside.
        // This is the most deeply nested schema so far:
        // Collection → List → Object → Mode → Select/String
        // 5 levels of nesting. API still reads well.

        .add(Parameter::boolean("drop_unmapped")
            .label("Drop Unmapped Fields")
            .description("Remove fields not present in mappings")
            .default(json!(false)))

        .add(Parameter::select("on_error")
            .label("On Mapping Error")
            .option("skip", "Skip Field")
            .option("null", "Set to Null")
            .option("fail", "Fail Execution")
            .default(json!("skip")))
}
```

### Expected values

```json
{
  "mappings": [
    {
      "source_field": "full_name",
      "target_field": "name",
      "transform": { "mode": "none" }
    },
    {
      "source_field": "created_at",
      "target_field": "signup_date",
      "transform": {
        "mode": "type_cast",
        "value": { "cast_to": "date" }
      }
    },
    {
      "source_field": "email",
      "target_field": "contact_email",
      "transform": {
        "mode": "expression",
        "value": { "expr": "{{ value.trim().toLowerCase() }}" }
      }
    }
  ],
  "drop_unmapped": false,
  "on_error": "skip"
}
```

### Validation notes

- ✅ 5-level nesting — still readable builder API.
- ✅ Dynamic select inside List item — loader loads once, shared across items.
- ✅ Mode inside List item Object — per-row transform type.
- ✅ `.allow_custom()` on target — user can create new field names.
- ✅ **Resolved: LoaderContext.metadata** — engine-injected context (upstream schemas, etc.) separate from user `values`. No more `_input_schema` hacks in values.

---

## 13. Email Send — multiple recipients, CC/BCC, attachments

Tests: List of emails with unique constraint, multiple lists,
multiline HTML body, file attachments.

### Schema

```rust
fn email_send_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::list("to")
            .label("To")
            .item(Parameter::string("email")
                .input_type("email")
                .with_rule(Rule::Pattern {
                    pattern: r"^[^@]+@[^@]+\.[^@]+$".to_owned(),
                    message: Some("must be a valid email address".to_owned()),
                }))
            .min_items(1)
            .unique())
        // ✅ WORKS WELL: List of emails with unique + validation.
        // Simple items (just strings, no object wrapper).

        .add(Parameter::list("cc")
            .label("CC")
            .item(Parameter::string("email")
                .input_type("email"))
            .unique())

        .add(Parameter::list("bcc")
            .label("BCC")
            .item(Parameter::string("email")
                .input_type("email"))
            .unique())
        // ⚠️ FRICTION: Same email item definition 3x (to, cc, bcc).
        // Rust function: fn email_item() -> Parameter { Parameter::string("email").input_type("email")... }
        // Same pattern as key-value from prototype #2.

        .add(Parameter::string("subject")
            .label("Subject")
            .required()
            .with_rule(Rule::MaxLength { max: 998, message: None }))

        .add(Parameter::mode("body_type")
            .label("Body")
            .required()
            .variant("text", "Plain Text",
                Parameter::string("text_body")
                    .label("Body")
                    .multiline()
                    .required())
            .variant("html", "HTML",
                Parameter::code("html_body")
                    .label("HTML Body")
                    .language("html")
                    .required())
            .default_variant("text"))
        // ✅ WORKS WELL: Mode for text vs HTML body.
        // Code type with language("html") — syntax highlighting.

        .add(Parameter::list("attachments")
            .label("Attachments")
            .item(Parameter::file("attachment")
                .max_size(25 * 1024 * 1024))  // 25MB
            .max_items(10))
        // ✅ WORKS WELL: List of files with max_size per file and max_items.

        .add(Parameter::object("options")
            .label("Options")
            .collapsed()
            .add(Parameter::string("reply_to")
                .label("Reply-To")
                .input_type("email"))
            .add(Parameter::select("priority")
                .label("Priority")
                .option("high", "High")
                .option("normal", "Normal")
                .option("low", "Low")
                .default(json!("normal")))
            .add(Parameter::boolean("read_receipt")
                .label("Request Read Receipt")
                .default(json!(false))))
}
```

### Validation notes

- ✅ List of simple strings (emails) with unique — no wrapping object needed.
- ✅ List of files — attachments with constraints.
- ✅ Mode for text vs HTML body — Code with language for HTML.
- ⚠️ **Reusable item pattern** — same as headers/query params friction. Document it.

---

## 14. Stripe Payment — currency handling, nested address, computed amounts

Tests: select affecting other field display (currency symbol in computed),
deeply nested address object, computed with formatting.

### Schema

```rust
fn stripe_create_payment_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::integer("amount")
            .label("Amount (in cents)")
            .description("Amount in smallest currency unit (e.g. cents for USD)")
            .required()
            .min(1))
        // ⚠️ FRICTION: "Amount in cents" is confusing UX. Ideally user enters
        // dollars and we compute cents. But computed is read-only.
        // Alternative: user enters decimal amount, action multiplies by 100.
        // Parameter crate can't do "transform on submit" — that's action logic.
        // DECISION: This is fine. Document the pattern: schema describes the API shape.

        .add(Parameter::select("currency")
            .label("Currency")
            .searchable()
            .option("usd", "USD — US Dollar")
            .option("eur", "EUR — Euro")
            .option("gbp", "GBP — British Pound")
            .option("jpy", "JPY — Japanese Yen")
            .option("cad", "CAD — Canadian Dollar")
            .option("aud", "AUD — Australian Dollar")
            .allow_custom()  // 135+ currencies in Stripe
            .default(json!("usd"))
            .required())

        .add(Parameter::computed("amount_display")
            .label("Display Amount")
            .returns_string()
            .expression("{{ format_currency(amount, currency) }}"))
        // ⚠️ Computed with function call — same as schedule prototype.
        // Expression engine needs format_currency(). Action authors may need
        // to register custom expression functions.

        .add(Parameter::string("description")
            .label("Description")
            .placeholder("Payment for order #1234"))

        .add(Parameter::string("customer_email")
            .label("Customer Email")
            .input_type("email")
            .required())

        // ── Shipping address ────────────────────────────────────────
        .add(Parameter::object("shipping")
            .label("Shipping Address")
            .add(Parameter::string("name").label("Full Name").required())
            .add(Parameter::string("line1").label("Address Line 1").required())
            .add(Parameter::string("line2").label("Address Line 2"))
            .add(Parameter::string("city").label("City").required())
            .add(Parameter::string("state").label("State / Region"))
            .add(Parameter::string("postal_code").label("Postal Code").required())
            .add(Parameter::select("country")
                .label("Country")
                .searchable()
                .option("US", "United States")
                .option("CA", "Canada")
                .option("GB", "United Kingdom")
                .option("DE", "Germany")
                .option("FR", "France")
                .option("JP", "Japan")
                .option("AU", "Australia")
                .allow_custom()
                .required()))
        // ✅ WORKS WELL: Nested object for address — standard pattern.
        // Many fields, all flat within object. No deeper nesting needed.

        // ── Metadata ────────────────────────────────────────────────
        .add(Parameter::list("metadata")
            .label("Metadata")
            .description("Key-value pairs attached to the payment")
            .item(Parameter::object("kv")
                .add(Parameter::string("key").label("Key").required())
                .add(Parameter::string("value").label("Value").required()))
            .max_items(50))
}
```

### Validation notes

- ✅ Nested address object — many flat fields, clean.
- ✅ Currency select with .allow_custom() — covers 135+ currencies.
- ✅ Computed display amount — good UX (shows "$12.34" instead of "1234").
- ⚠️ **"Amount in cents" UX** — schema describes API shape, not user-friendly shape. Transform is action logic.
- ⚠️ **Computed with custom functions** — expression engine needs function registry.

---

## 15. Database Query — SQL editor, parameterized queries, dynamic result mapping

Tests: Code type for SQL, List for query parameters with type select,
expression in parameter values.

### Schema

```rust
fn database_query_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::select("operation")
            .label("Operation")
            .option("query", "Execute Query")
            .option("insert", "Insert Row")
            .option("update", "Update Rows")
            .option("delete", "Delete Rows")
            .default(json!("query"))
            .required())

        // ── Table (dynamic from database) ───────────────────────────
        .add(Parameter::select("table")
            .label("Table")
            .searchable()
            .depends_on(&["operation"])
            .loader(|ctx: LoaderContext| async move {
                let cred = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No database credential"))?;
                let tables = db::list_tables(cred).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                Ok(tables.into_iter()
                    .map(|t| SelectOption::new(json!(t.name), &t.name))
                    .collect())
            })
            .active_when(Condition::ne("operation", json!("query"))))
        // ✅ WORKS WELL: Table visible+required for insert/update/delete, hidden for raw query.

        // ── Raw SQL (query mode) ────────────────────────────────────
        .add(Parameter::code("sql")
            .label("SQL Query")
            .language("sql")
            .active_when(Condition::eq("operation", json!("query"))))
        // ✅ WORKS WELL: Code type with language("sql") — syntax highlighting.
        // ✅ RESOLVED: Parameter::code(id) constructor added to HLD.

        // ── Query parameters ────────────────────────────────────────
        .add(Parameter::list("parameters")
            .label("Query Parameters")
            .description("Bind variables ($1, $2, ...) to prevent SQL injection")
            .item(Parameter::object("param")
                .add(Parameter::string("value")
                    .label("Value")
                    .expression(true)  // ⚠️ QUESTION: How do we allow expression here?
                    // .expression(true) = this field accepts {{ $input.user_id }} syntax.
                    // Frontend shows expression toggle button.
                    // When user enters expression, value becomes { "$expr": "{{ $input.user_id }}" }
                    // ✅ This is the correct use of the .expression flag.
                    .required())
                .add(Parameter::select("type")
                    .label("Type")
                    .option("string", "String")
                    .option("integer", "Integer")
                    .option("float", "Float")
                    .option("boolean", "Boolean")
                    .option("null", "NULL")
                    .default(json!("string"))))
            .visible_when(Condition::eq("operation", json!("query"))))

        // ── Insert/Update row data (dynamic from table columns) ─────
        .add(Parameter::dynamic("row_data")
            .label("Row Data")
            .depends_on(&["table"])
            .visible_when(Condition::any(vec![
                Condition::eq("operation", json!("insert")),
                Condition::eq("operation", json!("update")),
            ]))
            .loader(|ctx: LoaderContext| async move {
                let table = ctx.values.get_string("table")
                    .ok_or(LoaderError::new("Select a table first"))?;
                let cred = ctx.credential.as_ref()
                    .ok_or(LoaderError::new("No database credential"))?;
                let columns = db::describe_table(cred, table).await
                    .map_err(|e| LoaderError::new(e.to_string()).retryable())?;
                Ok(columns.into_iter()
                    .filter(|c| !c.is_auto_increment)  // skip auto-increment columns
                    .map(|col| {
                        let param = match col.data_type.as_str() {
                            "integer" | "bigint" => Parameter::integer(&col.name),
                            "boolean" => Parameter::boolean(&col.name),
                            "date" => Parameter::date(&col.name),
                            "timestamp" => Parameter::datetime(&col.name),
                            _ => Parameter::string(&col.name),
                        };
                        param.label(&col.name)
                            .description(format!("{} ({})", col.data_type, if col.nullable { "nullable" } else { "NOT NULL" }))
// ✅ RESOLVED: Use conditional builder based on column metadata:
                        let param = if !col.nullable { param.required() } else { param };
                        param
                    })
                    .collect())
            }))
        // ✅ WORKS WELL: Dynamic fields from table columns, type-aware.
        // RecordLoader maps DB column types to Parameter types.
        // ⚠️ CORRECTION above: Don't use required_when for static facts.
        // Use conditional builder: if !nullable { param.required() } else { param }

        // ── Where clause (update/delete) ────────────────────────────
        .add(Parameter::string("where_clause")
            .label("WHERE Clause")
            .placeholder("id = $1")
            .visible_when(Condition::any(vec![
                Condition::eq("operation", json!("update")),
                Condition::eq("operation", json!("delete")),
            ]))
            .required_when(Condition::eq("operation", json!("delete"))))
        // ✅ WORKS WELL: visible for update+delete, required only for delete.
        // Shows that visible_when and required_when can differ — important case.
}
```

### Validation notes

- ✅ Code type for SQL — natural.
- ✅ Dynamic fields map DB column types to Parameter types — powerful.
- ✅ `.expression(true)` on query parameter value — correct use of expression flag.
- ✅ visible_when ≠ required_when — WHERE clause visible for update+delete, required only for delete.
- ⚠️ **Dynamic loader builds typed parameters** — integer/boolean/date based on column type. This confirms `RecordLoader` returning `Vec<Parameter>` is correct.
- ✅ **Resolved:** `Parameter::date(id)`, `Parameter::datetime(id)` constructors added to HLD.

---

## 16. OAuth2 Credential — multi-step auth, URL fields, scope builder

Tests: multiple URL fields, scope as tag-like multi-input,
conditional fields based on grant type.

### Schema

```rust
fn oauth2_credential_schema() -> ParameterCollection {
    ParameterCollection::new()
        .add(Parameter::select("grant_type")
            .label("Grant Type")
            .option("authorization_code", "Authorization Code")
            .option("client_credentials", "Client Credentials")
            .option("pkce", "Authorization Code with PKCE")
            .default(json!("authorization_code"))
            .required())

        .add(Parameter::string("client_id")
            .label("Client ID")
            .required())

        .add(Parameter::string("client_secret")
            .label("Client Secret")
            .secret()
            .input_type("password")
            .required_when(Condition::ne("grant_type", json!("pkce"))))
        // ✅ WORKS WELL: client_secret not required for PKCE flow.

        .add(Parameter::string("authorization_url")
            .label("Authorization URL")
            .input_type("url")
            .placeholder("https://accounts.google.com/o/oauth2/v2/auth")
            .required_when(Condition::any(vec![
                Condition::eq("grant_type", json!("authorization_code")),
                Condition::eq("grant_type", json!("pkce")),
            ]))
            .visible_when(Condition::ne("grant_type", json!("client_credentials"))))
        // ✅ visible_when ≠ required_when again — visible for 2 types, required for 2 types,
        // but expressed differently (ne vs any+eq). Both work.

        .add(Parameter::string("token_url")
            .label("Token URL")
            .input_type("url")
            .placeholder("https://oauth2.googleapis.com/token")
            .required())

        .add(Parameter::string("scope")
            .label("Scope")
            .placeholder("openid email profile")
            .description("Space-separated list of scopes"))
        // ⚠️ FRICTION: Scope is space-separated string. In many UIs this is
        // rendered as tag input (chips). Our options:
        // (a) String field — user types space-separated. Simple but ugly.
        // (b) List of strings — better UX but changes data shape.
        // (c) String field with input_type("tags") — non-standard HTML type.
        // DECISION: Use (a) String for now. Scope format varies (some APIs use
        // comma-separated, some use arrays). String is most flexible.
        // Future: input_type can support custom types that frontend maps to widgets.

        .add(Parameter::string("redirect_uri")
            .label("Redirect URI")
            .input_type("url")
            .default(json!("https://app.nebula.io/oauth/callback"))
            .visible_when(Condition::ne("grant_type", json!("client_credentials"))))

        .add(Parameter::object("advanced")
            .label("Advanced")
            .collapsed()
            .add(Parameter::select("token_placement")
                .label("Token Placement")
                .option("header", "Authorization Header")
                .option("query", "Query Parameter")
                .option("body", "Request Body")
                .default(json!("header")))
            .add(Parameter::string("audience")
                .label("Audience")
                .placeholder("https://api.example.com"))
            .add(Parameter::string("custom_params")
                .label("Additional Parameters")
                .multiline()
                .description("key=value pairs, one per line")))
        // ⚠️ FRICTION: "Additional Parameters" as multiline string is weak.
        // Should be List of key-value pairs. But it's in advanced/collapsed,
        // and custom OAuth params are rare. String is OK for v1.
}
```

### Validation notes

- ✅ Grant type conditions — client_secret not required for PKCE.
- ✅ visible_when ≠ required_when — authorization_url hidden for client_credentials but required for code flows.
- ⚠️ **Scope UX** — space-separated string is pragmatic, tag input would be nicer. Future input_type extension.
- ⚠️ **Custom params** — multiline string vs list of key-value. String for v1, list later.

---

## Updated cross-cutting summary (all 16 prototypes)

### New friction points (additions to Part 1)

| # | Issue | Affected | Severity | Status |
|---|-------|----------|----------|--------|
| 12 | f64 ergonomics for `.min()` / `.max()` / `.step()` | AI node | Low | ✅ **Resolved:** `impl Into<serde_json::Number>` in HLD |
| 13 | Color type vs String+input_type("color") guidance | Slack | Low | ✅ **Resolved:** documented in DX Guidelines |
| 14 | LoaderContext needs metadata beyond user values | Data mapper | Medium | ✅ **Resolved:** `metadata` field added to LoaderContext |
| 15 | Missing constructors | DB query | Low | ✅ **Resolved:** all 17 constructors in HLD |
| 16 | Scope as tags / chips input | OAuth2 | Low | Open — String for v1 |
| 17 | `.expression(true)` flag usage pattern | DB query | Low | Open — document pattern |
| 18 | Computed expressions may need function calls | Schedule, Stripe | Medium | Open — runtime concern |

### New architecture decisions

| # | Decision | Recommendation |
|---|----------|----------------|
| D7 | LoaderContext metadata | Add `metadata: Option<serde_json::Value>` for engine-injected context (upstream schemas, etc.) |
| D8 | Dedicated type vs String+input_type | When both produce same value, prefer dedicated ParameterType for semantic clarity |
| D9 | Constructors for all types | Every ParameterType variant gets a `Parameter::typename(id)` constructor |

### Complete constructor inventory needed

```rust
// Existing in HLD
Parameter::string(id)
Parameter::number(id)
Parameter::integer(id)   // number with integer=true
Parameter::boolean(id)
Parameter::select(id)
Parameter::object(id)
Parameter::list(id)
Parameter::mode(id)
Parameter::computed(id)
Parameter::dynamic(id)
Parameter::hidden(id)

// Missing — add these
Parameter::code(id)
Parameter::date(id)
Parameter::datetime(id)
Parameter::time(id)
Parameter::color(id)
Parameter::file(id)
Parameter::filter(id)
```

