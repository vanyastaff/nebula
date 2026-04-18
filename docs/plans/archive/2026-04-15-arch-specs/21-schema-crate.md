# 21 — `nebula-schema` crate

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md), [`../2026-04-15-architecture-review-qa.md`](../2026-04-15-architecture-review-qa.md)
> **Replaces:** `crates/parameter/` (nebula-parameter) — to be deleted after migration
> **Depends on:** `nebula-core`, `nebula-error`, `nebula-validator`, `nebula-expression` (through interfaces)
> **Consumers:** `nebula-action`, `nebula-credential`, `nebula-derive` (macros), `nebula-runtime`, `apps/*`
> **Related specs:** 12 (expression language), 19 (error taxonomy), 20 (testing)
> **Related prototype:** `C:\Users\vanya\Downloads\parameter-prototypes.md` — 16 real-world schemas validated against the proposed API

## 1. Problem

The existing `nebula-parameter` crate (~8800 lines) provides a schema DSL for
all configurable surfaces of a workflow: action inputs, credentials, resource
configs, trigger configs. It works, but has accumulated friction:

1. **Naming is cosmetically wrong.** `ParameterCollection` is generic; `Parameter`
   conflates the top-level concept with its per-field instance; `ParameterType`
   is a flatten enum that forces all type-specific data into inline enum
   variants.

2. **Compile-time type safety is incomplete.** Typed builders
   (`StringBuilder`, `NumberBuilder`) give safety at *builder time*, but the
   `Parameter` struct itself accepts any `ParameterType` variant paired with
   any field values. Direct construction / deserialization / macro expansion
   can produce structurally invalid `Parameter { kind: String{..}, min: Some(5) }`.
   The invariance between `kind` and the rest of the struct is not expressed
   in the type system — only enforced by convention.

3. **Concern mixing.** `min: Option<Number>` / `max: Option<Number>` /
   `min_length` / `max_length` live on the enum variant *and* are expressible
   as `Rule::Min` / `Rule::MaxLength` in `nebula-validator`. Two sources of
   truth for the same validation → drift risk, duplicate enforcement paths.

4. **`Condition` duplicates `nebula-validator::Rule`.** Both are predicates
   over JSON values with logical combinators. `Condition` targets sibling
   fields for visibility gating; `Rule` targets the current value for
   validation. Two separate type hierarchies for what is structurally the
   same concept.

5. **`secret: bool` on every field type.** Only strings meaningfully carry
   sensitive material; `secret: true` on `SelectField` / `NumberField` /
   `ObjectField` is nonsense. The flag invites misuse.

6. **`expression: bool` is a schema-level opt-in for expression mode.** But
   in practice any input field can be an expression (n8n model — frontend
   shows a toggle button). The schema doesn't need to track this.

7. **Deprecated-but-active types** (`Date` / `DateTime` / `Time` / `Color` /
   `Hidden`) are `#[deprecated]` in the enum yet actively used in
   prototypes and real schemas. The deprecation marker is wrong.

8. **`Filter` is non-primitive.** It is composable out of `List<Object<Select + Select + String>>`;
   keeping it as its own primitive adds a type without adding expressive power.

This spec proposes a **new crate `nebula-schema`** that replaces
`nebula-parameter` with:

- Pattern 4 architecture — enum wrapper + per-type structs via macro
- Clean naming (`Schema`, `Field`, `FieldKey`, `FieldValue`, ...)
- Full compile-time type safety (structurally impossible invalid fields)
- Single source of truth for validation (`nebula_validator::Rule`)
- Unified predicate enum for both validation and visibility gating
- Dedicated `SecretField` type (instead of `secret: bool` flag)
- Typed widget hints per field type

## 2. Decision

### 2.1 New crate

Create `crates/schema/` (crate name `nebula-schema`) as a **parallel** crate
alongside `nebula-parameter`. Migrate callsites incrementally through a
PR sequence (§ 11). Once migration is complete, delete `nebula-parameter`.

### 2.2 Architecture — Pattern 4

```
┌──────────────────────────────────────────────────────────────┐
│ Schema                                                       │
│   fields: Vec<Field>                                         │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│ Field  (enum wrapper, #[serde(tag = "type")])                │
│                                                              │
│   String(StringField) | Secret(SecretField) | Number(..) |   │
│   Boolean(..) | Select(..) | Object(..) | List(..) |         │
│   Mode(..) | Code(..) | Date(..) | DateTime(..) | Time(..) | │
│   Color(..) | File(..) | Hidden(..) | Computed(..) |         │
│   Dynamic(..) | Notice(..)                                   │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│ StringField / NumberField / ... (18 per-type structs)        │
│                                                              │
│   key: FieldKey                                              │
│   label, description, placeholder, hint, default             │
│   visible: VisibilityMode                                    │
│   required: RequiredMode                                     │
│   rules: Vec<Rule>                                           │
│   transformers: Vec<Transformer>                             │
│   group: Option<String>                                      │
│   + type-specific fields (multiline/widget/min/max/...)      │
└──────────────────────────────────────────────────────────────┘
```

**Key properties:**

1. **Enum wrapper** `Field` provides homogeneous storage in `Schema::fields`
   and tagged-JSON serialization.
2. **Per-type structs** carry only their relevant fields — impossible to
   construct a `StringField { min: 5 }` at compile time.
3. **Shared fields** are injected into every struct via a declarative
   `define_field!` macro, so adding a new shared field edits one location.
4. **Type-specific methods** live in each struct's own `impl` block, giving
   compile-time rejection of cross-type misuse (`StringField.min(5)` → error).
5. **Serde** uses `#[serde(tag = "type", rename_all = "snake_case")]` on the
   `Field` enum and `#[serde(flatten)]` inside per-type structs for clean
   uniform JSON output.

### 2.3 Renames

| Old (`nebula-parameter`) | New (`nebula-schema`) |
|---|---|
| `ParameterCollection` | `Schema` |
| `Parameter` (struct) | `Field` (enum wrapper) |
| `ParameterType::String { .. }` | `StringField` struct |
| `ParameterType::Number { .. }` | `NumberField` struct |
| *(all 17 variants)* | *(separate structs)* |
| `ParameterValue` | `FieldValue` |
| `ParameterValues` | `FieldValues` |
| `ParameterPath` | `FieldPath` |
| `ParameterError` | `SchemaError` |
| `Condition` | `nebula_validator::Rule` (reuse) |
| `HasParameters` trait | `HasInputSchema` / `HasOutputSchema` (split via derive macros) |
| `nebula-parameter-macros` | `nebula-schema-macros` |
| `ParameterId` (string id) | `FieldKey` (newtype wrapper) |

### 2.4 Final 18 primitive types

1. **StringField** — plain text input
2. **SecretField** — masked sensitive string (new, replaces `secret: bool`)
3. **NumberField** — numeric (integer/float via flag)
4. **BooleanField** — boolean toggle
5. **SelectField** — single/multi select from options
6. **ObjectField** — nested struct (with display mode)
7. **ListField** — ordered collection of `Field` items
8. **ModeField** — discriminated union (select + payload)
9. **CodeField** — source code with language
10. **DateField** — date picker
11. **DateTimeField** — datetime picker
12. **TimeField** — time picker
13. **ColorField** — color picker
14. **FileField** — file upload
15. **HiddenField** — stored but not rendered
16. **ComputedField** — read-only derived
17. **DynamicField** — runtime-resolved schema via loader
18. **NoticeField** — display-only banner (info/warn/danger/success)

**Removed:** `FilterField` (composable from `List<Object<Select+Select+String>>` by users who need it).

### 2.5 Widget hints — per-type enums

Widget hints are **typed per-field** enums (not `Option<String>`). 7 enums for
types where visual variation matters. Types with a single consistent UX
(Date/DateTime/Time/Color/File/Hidden/Computed/Dynamic/Notice/Mode) have no
widget field.

### 2.6 Validation — single source via `nebula-validator`

All validation predicates are `nebula_validator::Rule`. A field's `rules:
Vec<Rule>` is applied at validation time. The same `Rule` enum powers
visibility gating (`visible_when`) and required gating (`required_when`),
leveraging the existing cross-field support in `nebula-validator`.

### 2.7 Integration with `ActionMetadata`

```rust
pub struct ActionMetadata {
    pub key: ActionKey,
    pub name: String,
    pub description: String,
    pub version: Version,
    pub input: Schema,
    pub output: OutputSchema,
    // ... other fields (see existing nebula-action crate)
}

pub enum OutputSchema {
    Typed(schemars::Schema),
    Opaque,
}
```

- **Input** — rich UI schema via `#[derive(Input)]` → `nebula-schema::Schema`
- **Output** — structural-only schema via `#[derive(Output)]` → `schemars::Schema`

Downstream expression references use the output schema for autocomplete and
compile-time field reference validation.

## 3. Data model

### 3.1 `FieldKey` newtype

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldKey(String);

impl FieldKey {
    /// Validates: non-empty, ASCII, `[a-zA-Z_][a-zA-Z0-9_]*`, max 64 chars.
    pub fn new(s: impl Into<String>) -> Result<Self, SchemaError> {
        let s = s.into();
        if s.is_empty() {
            return Err(SchemaError::InvalidKey("key cannot be empty".into()));
        }
        if s.len() > 64 {
            return Err(SchemaError::InvalidKey("key max 64 chars".into()));
        }
        if !s.chars().next().unwrap().is_ascii_alphabetic() && !s.starts_with('_') {
            return Err(SchemaError::InvalidKey(
                "key must start with letter or underscore".into(),
            ));
        }
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(SchemaError::InvalidKey(
                "key must be ASCII alphanumeric or underscore".into(),
            ));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Convenient `.into()` from static strings — panics if invalid. Use
/// `FieldKey::new()` for fallible construction from dynamic strings.
impl From<&'static str> for FieldKey {
    fn from(s: &'static str) -> Self {
        Self::new(s).expect("invalid static FieldKey")
    }
}
```

**Rationale**: prevents silent typos, enforces a stable identifier grammar
across all surfaces (UI, CLI, YAML, derive macros, lint diagnostics).

### 3.2 `VisibilityMode` and `RequiredMode`

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisibilityMode {
    #[default]
    Always,
    When(Rule),   // where `Rule` is `nebula_validator::Rule`
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequiredMode {
    #[default]
    Never,
    Always,
    When(Rule),
}
```

**Why single enums instead of `visible: bool + visible_when: Option<Rule>`:**

- Eliminates ambiguity when both are set.
- Type system makes state space explicit: default is "always visible,
  never required" via `Default` impl.
- Builder methods stay concise (`.required()`, `.required_when(rule)`,
  `.visible_when(rule)`, `.active_when(rule)`).

**Note:** `VisibilityMode::Never` is deliberately absent. Use `HiddenField`
for fields that should never render — it's a separate type, not a mode.

### 3.3 Widget enums (7)

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StringWidget {
    #[default]
    Plain,
    Multiline,
    Email,
    Url,
    Password,      // UI-masked but NOT a secret (consider SecretField instead)
    Phone,
    Ip,
    Regex,
    Markdown,
    Cron,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretWidget {
    #[default]
    Plain,         // single-line masked input
    Multiline,     // textarea masked (PEM keys, JSON service accounts)
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberWidget {
    #[default]
    Plain,
    Slider,
    Stepper,
    Percent,
    Currency,
    Duration,
    Bytes,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanWidget {
    #[default]
    Toggle,
    Checkbox,
    Radio,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectWidget {
    #[default]
    Dropdown,
    Radio,
    Checkboxes,
    Combobox,
    Tags,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectWidget {
    #[default]
    Inline,         // all fields visible
    Collapsed,      // collapse/expand section
    PickFields,     // "Add field" dropdown
    Sections,       // grouped "Add field" dropdown
    Tabs,           // tabs per field group
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListWidget {
    #[default]
    Plain,
    Sortable,       // drag-drop reordering
    Tags,           // tag chips (List<String>)
    KeyValue,       // key-value pairs (List<Object{key,value}>)
    Accordion,      // expandable items
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeWidget {
    #[default]
    Monaco,
    Simple,
}
```

**Types without widget enum** (consistent default rendering):

- `DateField`, `DateTimeField`, `TimeField` — calendar + input
- `ColorField` — color picker
- `FileField` — file picker with drag-drop
- `ModeField` — discriminator + payload (always)
- `HiddenField` — not rendered
- `ComputedField` — read-only display
- `DynamicField` — loader-driven
- `NoticeField` — rendering by severity (Info/Warn/Danger/Success)

### 3.4 The `define_field!` macro

Shared fields for every per-type struct are injected via a declarative macro:

```rust
macro_rules! define_field {
    ($name:ident { $($field:ident: $type:ty = $default:expr),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            // ── Shared fields (11) ──────────────────────────────────────
            pub key: FieldKey,

            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub label: Option<String>,

            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub description: Option<String>,

            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub placeholder: Option<String>,

            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub hint: Option<String>,

            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub default: Option<serde_json::Value>,

            #[serde(default, skip_serializing_if = "VisibilityMode::is_default")]
            pub visible: VisibilityMode,

            #[serde(default, skip_serializing_if = "RequiredMode::is_default")]
            pub required: RequiredMode,

            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub group: Option<String>,

            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub rules: Vec<Rule>,

            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub transformers: Vec<Transformer>,

            // ── Type-specific fields ────────────────────────────────────
            $(pub $field: $type,)*
        }

        impl $name {
            pub fn new(key: impl Into<FieldKey>) -> Self {
                Self {
                    key: key.into(),
                    label: None,
                    description: None,
                    placeholder: None,
                    hint: None,
                    default: None,
                    visible: VisibilityMode::default(),
                    required: RequiredMode::default(),
                    group: None,
                    rules: Vec::new(),
                    transformers: Vec::new(),
                    $($field: $default,)*
                }
            }

            // ── Shared builder methods ──────────────────────────────────

            pub fn label(mut self, s: impl Into<String>) -> Self {
                self.label = Some(s.into());
                self
            }

            pub fn description(mut self, s: impl Into<String>) -> Self {
                self.description = Some(s.into());
                self
            }

            pub fn placeholder(mut self, s: impl Into<String>) -> Self {
                self.placeholder = Some(s.into());
                self
            }

            pub fn hint(mut self, s: impl Into<String>) -> Self {
                self.hint = Some(s.into());
                self
            }

            pub fn default(mut self, v: serde_json::Value) -> Self {
                self.default = Some(v);
                self
            }

            pub fn required(mut self) -> Self {
                self.required = RequiredMode::Always;
                self
            }

            pub fn required_when(mut self, rule: Rule) -> Self {
                self.required = RequiredMode::When(rule);
                self
            }

            pub fn visible_when(mut self, rule: Rule) -> Self {
                self.visible = VisibilityMode::When(rule);
                self
            }

            /// Shorthand — visible AND required under same rule.
            pub fn active_when(mut self, rule: Rule) -> Self {
                self.visible = VisibilityMode::When(rule.clone());
                self.required = RequiredMode::When(rule);
                self
            }

            pub fn group(mut self, name: impl Into<String>) -> Self {
                self.group = Some(name.into());
                self
            }

            pub fn with_rule(mut self, rule: Rule) -> Self {
                self.rules.push(rule);
                self
            }

            pub fn with_transformer(mut self, t: Transformer) -> Self {
                self.transformers.push(t);
                self
            }
        }
    };
}
```

### 3.5 Per-type struct definitions

```rust
// ── 1. String ───────────────────────────────────────────────────────────
define_field!(StringField {
    widget: StringWidget = StringWidget::Plain,
});

impl StringField {
    pub fn widget(mut self, w: StringWidget) -> Self { self.widget = w; self }

    // Convenience shortcuts
    pub fn multiline(mut self) -> Self { self.widget = StringWidget::Multiline; self }
    pub fn email(mut self) -> Self { self.widget = StringWidget::Email; self }
    pub fn url(mut self) -> Self { self.widget = StringWidget::Url; self }
    pub fn phone(mut self) -> Self { self.widget = StringWidget::Phone; self }
    pub fn cron(mut self) -> Self { self.widget = StringWidget::Cron; self }

    // Type-safe validation entry points
    pub fn min_length(mut self, n: u32) -> Self {
        self.rules.push(Rule::MinLength { min: n, message: None });
        self
    }
    pub fn max_length(mut self, n: u32) -> Self {
        self.rules.push(Rule::MaxLength { max: n, message: None });
        self
    }
    pub fn pattern(mut self, p: impl Into<String>) -> Self {
        self.rules.push(Rule::Pattern { pattern: p.into(), message: None });
        self
    }
}

// ── 2. Secret ───────────────────────────────────────────────────────────
define_field!(SecretField {
    widget: SecretWidget = SecretWidget::Plain,
    reveal_last: Option<u8> = None,
});

impl SecretField {
    pub fn widget(mut self, w: SecretWidget) -> Self { self.widget = w; self }
    pub fn multiline(mut self) -> Self { self.widget = SecretWidget::Multiline; self }

    /// Show the last N characters of the stored value for identification
    /// (e.g., `•••••••••ef3a`). `None` = fully masked.
    pub fn reveal_last(mut self, n: u8) -> Self {
        self.reveal_last = Some(n);
        self
    }

    pub fn min_length(mut self, n: u32) -> Self {
        self.rules.push(Rule::MinLength { min: n, message: None });
        self
    }
    pub fn max_length(mut self, n: u32) -> Self {
        self.rules.push(Rule::MaxLength { max: n, message: None });
        self
    }
    pub fn pattern(mut self, p: impl Into<String>) -> Self {
        self.rules.push(Rule::Pattern { pattern: p.into(), message: None });
        self
    }
}

// ── 3. Number ───────────────────────────────────────────────────────────
define_field!(NumberField {
    integer: bool = false,
    widget: NumberWidget = NumberWidget::Plain,
    step: Option<serde_json::Number> = None,
});

impl NumberField {
    pub fn widget(mut self, w: NumberWidget) -> Self { self.widget = w; self }
    pub fn integer(mut self) -> Self { self.integer = true; self }

    pub fn step(mut self, s: impl Into<serde_json::Number>) -> Self {
        self.step = Some(s.into());
        self
    }

    pub fn min(mut self, n: impl Into<serde_json::Number>) -> Self {
        self.rules.push(Rule::Min { min: n.into(), message: None });
        self
    }
    pub fn max(mut self, n: impl Into<serde_json::Number>) -> Self {
        self.rules.push(Rule::Max { max: n.into(), message: None });
        self
    }
}

// ── 4. Boolean ──────────────────────────────────────────────────────────
define_field!(BooleanField {
    widget: BooleanWidget = BooleanWidget::Toggle,
});

impl BooleanField {
    pub fn widget(mut self, w: BooleanWidget) -> Self { self.widget = w; self }
}

// ── 5. Select ───────────────────────────────────────────────────────────
define_field!(SelectField {
    options: Vec<SelectOption> = Vec::new(),
    dynamic: bool = false,
    depends_on: Vec<FieldPath> = Vec::new(),
    multiple: bool = false,
    allow_custom: bool = false,
    searchable: bool = false,
    widget: SelectWidget = SelectWidget::Dropdown,
    #[serde(skip)]
    loader: Option<OptionLoader> = None,
});

impl SelectField {
    pub fn widget(mut self, w: SelectWidget) -> Self { self.widget = w; self }

    pub fn option(mut self, value: impl Into<serde_json::Value>, label: impl Into<String>) -> Self {
        self.options.push(SelectOption::new(value.into(), label));
        self
    }

    pub fn option_with(mut self, opt: SelectOption) -> Self {
        self.options.push(opt);
        self
    }

    pub fn multiple(mut self) -> Self { self.multiple = true; self }
    pub fn allow_custom(mut self) -> Self { self.allow_custom = true; self }
    pub fn searchable(mut self) -> Self { self.searchable = true; self }

    pub fn depends_on(mut self, paths: &[impl AsRef<str>]) -> Self {
        self.depends_on = paths.iter().map(|p| FieldPath::parse(p.as_ref())).collect();
        self
    }

    pub fn loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<LoaderResult<SelectOption>, LoaderError>> + Send + 'static,
    {
        self.dynamic = true;
        self.loader = Some(OptionLoader::new(f));
        self
    }
}

// ── 6. Object ───────────────────────────────────────────────────────────
define_field!(ObjectField {
    parameters: Vec<Field> = Vec::new(),
    widget: ObjectWidget = ObjectWidget::Inline,
});

impl ObjectField {
    pub fn widget(mut self, w: ObjectWidget) -> Self { self.widget = w; self }
    pub fn collapsed(mut self) -> Self { self.widget = ObjectWidget::Collapsed; self }
    pub fn pick_fields(mut self) -> Self { self.widget = ObjectWidget::PickFields; self }
    pub fn sections(mut self) -> Self { self.widget = ObjectWidget::Sections; self }
    pub fn tabs(mut self) -> Self { self.widget = ObjectWidget::Tabs; self }

    pub fn add(mut self, field: impl Into<Field>) -> Self {
        self.parameters.push(field.into());
        self
    }
}

// ── 7. List ─────────────────────────────────────────────────────────────
define_field!(ListField {
    item: Option<Box<Field>> = None,
    min_items: Option<u32> = None,
    max_items: Option<u32> = None,
    unique: bool = false,
    widget: ListWidget = ListWidget::Plain,
});

impl ListField {
    pub fn widget(mut self, w: ListWidget) -> Self { self.widget = w; self }
    pub fn sortable(mut self) -> Self { self.widget = ListWidget::Sortable; self }

    pub fn item(mut self, field: impl Into<Field>) -> Self {
        self.item = Some(Box::new(field.into()));
        self
    }

    pub fn min_items(mut self, n: u32) -> Self { self.min_items = Some(n); self }
    pub fn max_items(mut self, n: u32) -> Self { self.max_items = Some(n); self }
    pub fn unique(mut self) -> Self { self.unique = true; self }
}

// ── 8. Mode ─────────────────────────────────────────────────────────────
define_field!(ModeField {
    variants: Vec<ModeVariant> = Vec::new(),
    default_variant: Option<String> = None,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeVariant {
    pub key: String,
    pub label: String,
    pub field: Box<Field>,
}

impl ModeField {
    pub fn variant(
        mut self,
        key: impl Into<String>,
        label: impl Into<String>,
        field: impl Into<Field>,
    ) -> Self {
        self.variants.push(ModeVariant {
            key: key.into(),
            label: label.into(),
            field: Box::new(field.into()),
        });
        self
    }

    pub fn default_variant(mut self, key: impl Into<String>) -> Self {
        self.default_variant = Some(key.into());
        self
    }
}

// ── 9. Code ─────────────────────────────────────────────────────────────
define_field!(CodeField {
    language: String = String::from("plaintext"),
    widget: CodeWidget = CodeWidget::Monaco,
});

impl CodeField {
    pub fn widget(mut self, w: CodeWidget) -> Self { self.widget = w; self }
    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.language = lang.into();
        self
    }
}

// ── 10-12. Date / DateTime / Time ───────────────────────────────────────
define_field!(DateField {});
define_field!(DateTimeField {});
define_field!(TimeField {});

// ── 13. Color ───────────────────────────────────────────────────────────
define_field!(ColorField {});

// ── 14. File ────────────────────────────────────────────────────────────
define_field!(FileField {
    accept: Option<String> = None,
    max_size: Option<u64> = None,
    multiple: bool = false,
});

impl FileField {
    pub fn accept(mut self, mime: impl Into<String>) -> Self {
        self.accept = Some(mime.into());
        self
    }
    pub fn max_size(mut self, bytes: u64) -> Self { self.max_size = Some(bytes); self }
    pub fn multiple(mut self) -> Self { self.multiple = true; self }
}

// ── 15. Hidden ──────────────────────────────────────────────────────────
define_field!(HiddenField {});

// ── 16. Computed ────────────────────────────────────────────────────────
define_field!(ComputedField {
    expression: String = String::new(),
    returns: ComputedReturn = ComputedReturn::String,
});

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputedReturn { String, Number, Boolean }

impl ComputedField {
    pub fn expression(mut self, expr: impl Into<String>) -> Self {
        self.expression = expr.into();
        self
    }
    pub fn returns_string(mut self) -> Self { self.returns = ComputedReturn::String; self }
    pub fn returns_number(mut self) -> Self { self.returns = ComputedReturn::Number; self }
    pub fn returns_boolean(mut self) -> Self { self.returns = ComputedReturn::Boolean; self }
}

// ── 17. Dynamic ─────────────────────────────────────────────────────────
define_field!(DynamicField {
    depends_on: Vec<FieldPath> = Vec::new(),
    #[serde(skip)]
    loader: Option<RecordLoader> = None,
});

impl DynamicField {
    pub fn depends_on(mut self, paths: &[impl AsRef<str>]) -> Self {
        self.depends_on = paths.iter().map(|p| FieldPath::parse(p.as_ref())).collect();
        self
    }

    pub fn loader<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<LoaderResult<Field>, LoaderError>> + Send + 'static,
    {
        self.loader = Some(RecordLoader::new(f));
        self
    }
}

// ── 18. Notice ──────────────────────────────────────────────────────────
define_field!(NoticeField {
    severity: NoticeSeverity = NoticeSeverity::Info,
});

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    #[default]
    Info,
    Warning,
    Danger,
    Success,
}

impl NoticeField {
    pub fn severity(mut self, s: NoticeSeverity) -> Self { self.severity = s; self }
    pub fn info(mut self) -> Self { self.severity = NoticeSeverity::Info; self }
    pub fn warning(mut self) -> Self { self.severity = NoticeSeverity::Warning; self }
    pub fn danger(mut self) -> Self { self.severity = NoticeSeverity::Danger; self }
    pub fn success(mut self) -> Self { self.severity = NoticeSeverity::Success; self }
}
```

### 3.6 `Field` enum wrapper

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Field {
    String(StringField),
    Secret(SecretField),
    Number(NumberField),
    Boolean(BooleanField),
    Select(SelectField),
    Object(ObjectField),
    List(ListField),
    Mode(ModeField),
    Code(CodeField),
    Date(DateField),
    DateTime(DateTimeField),
    Time(TimeField),
    Color(ColorField),
    File(FileField),
    Hidden(HiddenField),
    Computed(ComputedField),
    Dynamic(DynamicField),
    Notice(NoticeField),
}

impl Field {
    // Entry points — return typed structs, not Field.
    // Users chain builder methods then let Into<Field> auto-convert at Schema::add.
    pub fn string(key: impl Into<FieldKey>) -> StringField { StringField::new(key) }
    pub fn secret(key: impl Into<FieldKey>) -> SecretField { SecretField::new(key) }
    pub fn number(key: impl Into<FieldKey>) -> NumberField { NumberField::new(key) }
    pub fn integer(key: impl Into<FieldKey>) -> NumberField { NumberField::new(key).integer() }
    pub fn boolean(key: impl Into<FieldKey>) -> BooleanField { BooleanField::new(key) }
    pub fn select(key: impl Into<FieldKey>) -> SelectField { SelectField::new(key) }
    pub fn object(key: impl Into<FieldKey>) -> ObjectField { ObjectField::new(key) }
    pub fn list(key: impl Into<FieldKey>) -> ListField { ListField::new(key) }
    pub fn mode(key: impl Into<FieldKey>) -> ModeField { ModeField::new(key) }
    pub fn code(key: impl Into<FieldKey>) -> CodeField { CodeField::new(key) }
    pub fn date(key: impl Into<FieldKey>) -> DateField { DateField::new(key) }
    pub fn datetime(key: impl Into<FieldKey>) -> DateTimeField { DateTimeField::new(key) }
    pub fn time(key: impl Into<FieldKey>) -> TimeField { TimeField::new(key) }
    pub fn color(key: impl Into<FieldKey>) -> ColorField { ColorField::new(key) }
    pub fn file(key: impl Into<FieldKey>) -> FileField { FileField::new(key) }
    pub fn hidden(key: impl Into<FieldKey>) -> HiddenField { HiddenField::new(key) }
    pub fn computed(key: impl Into<FieldKey>) -> ComputedField { ComputedField::new(key) }
    pub fn dynamic(key: impl Into<FieldKey>) -> DynamicField { DynamicField::new(key) }
    pub fn notice(key: impl Into<FieldKey>) -> NoticeField { NoticeField::new(key) }

    /// Shared accessor — all variants expose key.
    pub fn key(&self) -> &FieldKey {
        match self {
            Self::String(f) => &f.key,
            Self::Secret(f) => &f.key,
            Self::Number(f) => &f.key,
            Self::Boolean(f) => &f.key,
            Self::Select(f) => &f.key,
            Self::Object(f) => &f.key,
            Self::List(f) => &f.key,
            Self::Mode(f) => &f.key,
            Self::Code(f) => &f.key,
            Self::Date(f) => &f.key,
            Self::DateTime(f) => &f.key,
            Self::Time(f) => &f.key,
            Self::Color(f) => &f.key,
            Self::File(f) => &f.key,
            Self::Hidden(f) => &f.key,
            Self::Computed(f) => &f.key,
            Self::Dynamic(f) => &f.key,
            Self::Notice(f) => &f.key,
        }
    }

    // Similar accessors generated via macro for label, required, visible, rules, etc.
}

// Auto-conversion for ergonomic Schema::add(...)
impl From<StringField> for Field { fn from(f: StringField) -> Self { Self::String(f) } }
impl From<SecretField> for Field { fn from(f: SecretField) -> Self { Self::Secret(f) } }
// ... for each struct (generated by helper macro)
```

### 3.7 `Schema` top-level

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub fields: Vec<Field>,
}

impl Schema {
    pub fn new() -> Self { Self::default() }

    pub fn add(mut self, field: impl Into<Field>) -> Self {
        self.fields.push(field.into());
        self
    }

    pub fn len(&self) -> usize { self.fields.len() }
    pub fn is_empty(&self) -> bool { self.fields.is_empty() }

    pub fn find(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key().as_str() == key)
    }

    /// Validate runtime values against this schema.
    pub fn validate(&self, values: &FieldValues) -> ValidationReport {
        // Delegates to nebula-validator for rule evaluation.
        // See § 4.2 for flow details.
    }

    /// Normalize runtime values — backfill defaults, resolve computed fields.
    pub fn normalize(&self, values: &FieldValues) -> FieldValues {
        // See § 4.3.
    }
}
```

### 3.8 `FieldValue` (runtime wire format)

```rust
/// Reserved object key for expression-backed values.
pub const EXPRESSION_KEY: &str = "$expr";

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Literal(serde_json::Value),
    Expression(String),
    Mode { mode: String, value: Option<serde_json::Value> },
}

impl FieldValue {
    /// Parse from JSON runtime wire.
    ///
    /// Detection precedence:
    /// 1. Object with single `$expr` string key → `Expression`
    /// 2. Object with `mode` string key → `Mode`
    /// 3. String containing `{{ ... }}` pattern → `Expression`
    /// 4. Otherwise → `Literal`
    pub fn from_json(v: &serde_json::Value) -> Self {
        if let Some(obj) = v.as_object() {
            if obj.len() == 1 {
                if let Some(expr) = obj.get(EXPRESSION_KEY).and_then(|x| x.as_str()) {
                    return Self::Expression(expr.to_owned());
                }
            }
            if let Some(mode) = obj.get("mode").and_then(|x| x.as_str()) {
                return Self::Mode {
                    mode: mode.to_owned(),
                    value: obj.get("value").cloned(),
                };
            }
        }
        if let Some(s) = v.as_str() {
            if Self::contains_expression_marker(s) {
                return Self::Expression(s.to_owned());
            }
        }
        Self::Literal(v.clone())
    }

    fn contains_expression_marker(s: &str) -> bool {
        // Simple lexer: look for `{{` with matching `}}`.
        // Escape: `{{{{ literal }}}}` → literal `{{ literal }}`.
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '{' {
                if chars.next() == Some('{') {
                    if chars.peek() != Some(&'{') { return true; }
                    // Escaped `{{{{` — skip.
                    chars.next();
                }
            }
        }
        false
    }

    pub fn into_json(self) -> serde_json::Value {
        match self {
            Self::Literal(v) => v,
            Self::Expression(s) => {
                // Canonical wire format when explicit wrapper needed.
                // Inline `{{ }}` is preferred for user input; `{$expr: ...}`
                // is the escape hatch for strings that would be ambiguous.
                serde_json::json!({ EXPRESSION_KEY: s })
            }
            Self::Mode { mode, value } => {
                let mut o = serde_json::Map::new();
                o.insert("mode".to_owned(), serde_json::Value::String(mode));
                if let Some(v) = value { o.insert("value".to_owned(), v); }
                serde_json::Value::Object(o)
            }
        }
    }
}
```

### 3.9 `FieldValues` (runtime map)

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldValues(HashMap<String, serde_json::Value>);

impl FieldValues {
    pub fn new() -> Self { Self::default() }

    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.0.insert(key.into(), value);
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.0.get(key)
    }

    pub fn get_typed(&self, key: &str) -> Option<FieldValue> {
        self.0.get(key).map(FieldValue::from_json)
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.as_str())
    }

    pub fn get_number<T: Numeric>(&self, key: &str) -> Option<T> {
        self.0.get(key).and_then(T::from_json)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(|v| v.as_bool())
    }
}
```

### 3.10 `FieldPath`

```rust
/// Typed reference to a field, supports root anchoring and nested paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldPath(String);

impl FieldPath {
    /// Local path — resolved relative to parent Object/List scope.
    pub fn local(s: impl Into<String>) -> Self { Self(s.into()) }

    /// Absolute path — resolved from root of schema. Prefix: `$root.`
    pub fn root(s: impl Into<String>) -> Self {
        Self(format!("$root.{}", s.into()))
    }

    pub fn parse(s: &str) -> Self { Self(s.to_owned()) }

    pub fn is_root(&self) -> bool { self.0.starts_with("$root.") }

    pub fn as_str(&self) -> &str { &self.0 }
}
```

### 3.11 `SchemaError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("invalid field key: {0}")]
    InvalidKey(String),

    #[error("duplicate field key: {0}")]
    DuplicateKey(String),

    #[error("field not found: {0}")]
    FieldNotFound(String),

    #[error("type mismatch at {path}: expected {expected}, got {actual}")]
    TypeMismatch { path: String, expected: String, actual: String },

    #[error("rule does not apply to field type: {rule} on {field_type}")]
    RuleTypeMismatch { rule: String, field_type: String },

    #[error("validation failed: {0}")]
    Validation(#[from] nebula_validator::ValidatorError),

    #[error("loader error: {0}")]
    Loader(#[from] LoaderError),
}
```

### 3.12 `Transformer`

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transformer {
    Trim,
    Lowercase,
    Uppercase,
    ExtractRegex { pattern: String, group: u32 },
    Replace { from: String, to: String },
    // ... extensible
}
```

### 3.13 `ActionMetadata::output`

```rust
// In nebula-action crate
pub struct ActionMetadata {
    pub key: ActionKey,
    pub name: String,
    pub description: String,
    pub version: Version,

    /// Rich UI schema for action input parameters.
    pub input: nebula_schema::Schema,

    /// Structural schema for action output — used by downstream expression
    /// references for autocomplete and type-checking.
    pub output: OutputSchema,

    // ... other existing fields
}

pub enum OutputSchema {
    /// Known shape via schemars::JsonSchema derive.
    Typed(schemars::Schema),

    /// Unknown shape — action returns arbitrary JSON. Downstream treats
    /// output as `any` with no autocomplete.
    Opaque,
}
```

### 3.14 Derive macros

Two focused derive macros in `nebula-schema-macros`:

```rust
// Input schema — rich UI via nebula-schema
#[derive(Input)]
struct SendMessageInput {
    #[field(label = "Chat ID", required)]
    chat_id: String,

    #[field(label = "Message", widget = "multiline")]
    text: String,

    #[field(label = "Timeout (ms)", min = 100, max = 60_000, default = 5000)]
    timeout_ms: u32,

    #[field(label = "API Key")]
    api_key: Secret,                  // nebula_schema::Secret newtype → SecretField

    #[field(label = "Authentication")]
    auth: AuthMode,                   // enum → ModeField

    #[field(
        label = "OAuth Scopes",
        visible_when = "auth == 'oauth2'"
    )]
    oauth_scopes: Vec<String>,
}

// Output schema — schemars for structural shape
#[derive(Output)]
struct SendMessageOutput {
    message_id: String,
    sent_at: chrono::DateTime<chrono::Utc>,
    channel: ChannelInfo,
}
```

**Generated code:**

```rust
impl HasInputSchema for SendMessageInput {
    fn input_schema() -> Schema {
        Schema::new()
            .add(Field::string("chat_id").label("Chat ID").required())
            .add(Field::string("text").label("Message").multiline())
            .add(Field::integer("timeout_ms").label("Timeout (ms)")
                 .min(100).max(60_000).default(json!(5000)))
            .add(Field::secret("api_key").label("API Key"))
            .add(/* Mode field generated from AuthMode enum */)
            .add(Field::list("oauth_scopes").label("OAuth Scopes")
                 .item(Field::string("scope").build())
                 .visible_when(Rule::FieldEq {
                     field: "auth".into(),
                     value: json!("oauth2"),
                 }))
    }
}

impl HasOutputSchema for SendMessageOutput {
    fn output_schema() -> OutputSchema {
        OutputSchema::Typed(schemars::schema_for!(Self))
    }
}
```

## 4. Flows

### 4.1 Schema construction

```rust
use nebula_schema::{Schema, Field, SelectOption, Rule};
use nebula_validator::Rule as ValidatorRule;  // alias for clarity
use serde_json::json;

let schema = Schema::new()
    .add(Field::select("resource")
        .label("Resource")
        .option("message", "Message")
        .option("chat", "Chat")
        .default(json!("message"))
        .required())

    .add(Field::select("operation")
        .label("Operation")
        .option("sendMessage", "Send Message")
        .option("sendPhoto", "Send Photo")
        .default(json!("sendMessage"))
        .searchable()
        .required())

    .add(Field::string("chat_id")
        .label("Chat ID")
        .required())

    .add(Field::string("text")
        .label("Text")
        .multiline()
        .max_length(4096)
        .active_when(ValidatorRule::FieldEq {
            field: "operation".into(),
            value: json!("sendMessage"),
        }))

    .add(Field::secret("api_key")
        .label("API Key")
        .required()
        .with_rule(ValidatorRule::Pattern {
            pattern: r"^sk-[a-zA-Z0-9]{32}$".into(),
            message: Some("invalid key format".into()),
        })
        .reveal_last(4));
```

### 4.2 Validation flow

```
User input → FieldValues → Schema::validate(values) →
    for each field in schema.fields:
        if is_visible(field, values):
            if is_required(field, values) and values.get(field.key()).is_none():
                report.error(field.key(), "required field missing")
                continue
            for rule in field.rules():
                nebula_validator::evaluate(rule, values.get(field.key()), &context)
                    .map_err(|e| report.error(...))
        → ValidationReport
```

**Execution modes** (from `nebula-validator::ExecutionMode`):

- **`StaticOnly`** — schema-time validation, skips rules that require runtime
  expression resolution. Used when saving a workflow draft.
- **`Deferred`** — runtime validation of previously-deferred rules, after
  expression resolution.
- **`Full`** — both at once, used at execution time.

### 4.3 Normalization

```
Schema::normalize(values) →
    let normalized = FieldValues::new()
    for field in schema.fields:
        if values.contains(field.key()):
            normalized.set(field.key(), values.get(field.key()))
        else if let Some(default) = field.default():
            normalized.set(field.key(), default)
    → normalized
```

Unlike old `nebula-parameter`, normalization ignores any Object display mode —
defaults are always backfilled (DisplayMode is UI-only, see §6.2).

### 4.4 Condition evaluation (visibility / required)

```rust
pub fn is_visible(field: &Field, values: &FieldValues) -> bool {
    match field.visible() {
        VisibilityMode::Always => true,
        VisibilityMode::When(rule) => {
            nebula_validator::evaluate_context(rule, values)
                .unwrap_or(true)  // default to visible on eval failure
        }
    }
}

pub fn is_required(field: &Field, values: &FieldValues) -> bool {
    match field.required() {
        RequiredMode::Never => false,
        RequiredMode::Always => true,
        RequiredMode::When(rule) => {
            nebula_validator::evaluate_context(rule, values)
                .unwrap_or(false)  // default to optional on eval failure
        }
    }
}
```

### 4.5 Serde round-trip

**Serialization** of `Schema::new().add(Field::string("name").required().max_length(50))`:

```json
{
  "fields": [
    {
      "type": "string",
      "key": "name",
      "required": { "kind": "always" },
      "rules": [
        { "kind": "max_length", "max": 50 }
      ],
      "widget": "plain"
    }
  ]
}
```

Deserialization reverses this via `#[serde(tag = "type")]` on the `Field`
enum wrapper. Type mismatches produce `SchemaError::TypeMismatch`.

### 4.6 Wire format for `FieldValue`

YAML workflow parameter values use smart detection per § 3.8:

```yaml
parameters:
  # literal number
  retries: 3

  # literal string
  method: "GET"

  # expression (inline {{ }})
  timeout_ms: "{{ $config.default_timeout }}"
  url: "{{ $node.config.output.base_url }}/users/{{ $input.user_id }}"

  # mode value
  auth:
    mode: "bearer"
    value: "{{ $credentials.github_token }}"

  # expression escape (rare — literal string containing {{)
  weird_string:
    $expr: "literal string that has {{ inside }}"
```

## 5. Edge cases

### 5.1 Recursive structures

`ListField::item: Box<Field>` allows `List<List<...>>` or `List<Object<List<...>>>`.
No depth limit at the type level; `Schema::validate` enforces a runtime
depth limit (default 10) to prevent stack overflow.

### 5.2 Deeply nested Mode

A `ModeField` variant can itself contain another `ModeField` via
`Box<Field>`. Use case: conditional authentication with sub-variants (OAuth2
→ authorization_code / client_credentials / PKCE). Maximum practical depth
tested: 5 levels (Data Mapper prototype in § 12 of parameter-prototypes.md).

### 5.3 Rule on wrong field type

Escape hatch `with_rule(Rule)` accepts any `Rule` variant. A lint diagnostic
catches semantic mismatches:

```rust
Field::number("age").with_rule(Rule::Pattern { .. })
// ⚠️  lint: Rule::Pattern applies to String, not Number.
```

This is **runtime lint**, not compile-time — because `with_rule` is an
escape hatch. Type-safe entry points (`.min_length()`, `.pattern()`, `.min()`)
prevent misuse at compile time.

### 5.4 Expression type ambiguity

```yaml
timeout_ms: "{{ $config.ttl }}"    # string — will parse as Expression
timeout_ms: 5000                    # integer — will parse as Literal
```

Schema-time (`StaticOnly` mode) `NumberField` accepts both:
- JSON number → validate min/max directly
- JSON string starting with `{{` → expression, defer validation to runtime
- JSON string NOT starting with `{{` → type mismatch error

Runtime (`Full` mode) resolves expression → validates resolved value against
rules. Strict mode: fail on type coercion; non-strict: coerce with warning.

### 5.5 Literal string containing `{{`

Escape via `{{{{ literal }}}}` (double brace) — parser unescapes to
`{{ literal }}`. Alternatively, use explicit wrapper:

```yaml
weird:
  $expr: "literal with {{ inside }}"
```

### 5.6 Circular visibility/required references

`Field A visible_when → depends on Field B, Field B visible_when → depends on Field A`.
Schema lint detects cycles in the visibility DAG at load time and emits
`SchemaError::CircularDependency`.

### 5.7 Missing required field inside invisible Object

If `ObjectField` is invisible (`visible = When(rule)` where rule evaluates
false), its children are **not validated**. Rationale: invisible fields
have no user input, and defaults or absence don't fail required checks.

### 5.8 Dynamic loader failure

If a `DynamicField::loader` or `SelectField::loader` async fn fails, the
frontend receives `LoaderError` with optional `.retryable()` flag. Schema
validation skips fields whose loaders haven't resolved — treats them as
optional for that validation pass.

## 6. Configuration surface

`nebula-schema` is a library crate with no runtime configuration. All
behavior is controlled by the schema definition itself.

### 6.1 `Cargo.toml` features

```toml
[features]
default = []
# Enable schemars integration for generating JSON Schema documents from
# schemas (used by #[derive(Output)]).
schemars = ["dep:schemars"]
```

### 6.2 DisplayMode → ObjectWidget

The old `DisplayMode` enum (Inline/Collapsed/PickFields/Sections) is
replaced by `ObjectWidget` (same variants + Tabs). **Crucially,
normalization no longer special-cases PickFields/Sections** — defaults are
always backfilled regardless of widget. The widget only affects UI
rendering; backend semantics are uniform.

Breaking change for callers that depended on the old behavior: after
migration, values for an object field always contain all defined keys
(with defaults if absent). If a consumer needs "only explicitly-set keys",
they must filter at the consumer layer.

## 7. Testing criteria

### 7.1 Unit tests

- **FieldKey validation** — accepts `[a-zA-Z_][a-zA-Z0-9_]*{0,63}`, rejects
  empty / invalid chars / too long
- **Field::new() defaults** — every struct has correct `Default` behavior
- **Type-specific builders** — chained builders produce expected state
- **Compile-fail tests** (`trybuild` crate):
  - `StringField::new("x").min(5)` → compile error
  - `NumberField::new("x").multiline()` → compile error
  - `BooleanField::new("x").pattern("x")` → compile error
- **Serde round-trip** — every Field variant serializes and deserializes
  identically; JSON output matches canonical shape

### 7.2 Integration tests

- **Prototype translation** — all 16 schemas from `parameter-prototypes.md`
  rewritten in new API and validated:
  - Telegram Bot (resource → operation pattern)
  - HTTP Request (auth modes, headers, body)
  - Google Sheets (dynamic selects, depends_on chains)
  - Postgres Credential (secret fields, computed preview)
  - Postgres Resource Config (defaults-only)
  - If/Switch/ForEach/Wait (control flow)
  - E-commerce Order (computed line totals, list validation)
  - AI/LLM Node (slider, dynamic model list, advanced section)
  - Slack Send Message (dynamic channels, multi-select mentions)
  - Cron Schedule Trigger (Mode for schedule type, Time/DateTime)
  - Data Mapper (5-level nesting)
  - Email Send (lists of files, HTML body)
  - Stripe Payment (nested address, currency)
  - Database Query (SQL editor, dynamic row_data)
  - OAuth2 Credential (multi-step auth, grant type conditions)

- **Validation flow** — for each prototype, feed valid/invalid inputs and
  verify `ValidationReport` matches expectations

### 7.3 Round-trip tests

- **YAML workflow deserialization** — all files in `apps/cli/examples/*.yaml`
  load successfully, values deserialize correctly (literal vs expression
  vs mode), workflow executes against translated schemas

### 7.4 Lint tests

- **Duplicate keys** — `Schema::new().add(Field::string("x")).add(Field::string("x"))`
  → lint error
- **Dangling references** — `Rule::FieldEq { field: "missing", .. }` → warning
- **Rule type mismatch** — `Field::string(...).with_rule(Rule::Max { .. })`
  → warning
- **Circular visibility** — `A.visible_when(B), B.visible_when(A)` → error

### 7.5 Contract tests

- Every builder method is `#[must_use]` — linter catches unused results
- Every `Field` variant implements `From<SpecificField>` (verified via test)
- Every shared method (`.label()`, `.required()`, etc.) exists on every
  typed struct (verified via macro expansion test)

## 8. Performance targets

| Operation | Target | Rationale |
|---|---|---|
| Build 100-field Schema (all types) | < 100 µs | One-time per action registration |
| Validate 100 values against 100-field schema (StaticOnly) | < 1 ms | Hot path for CLI `validate` and API create |
| Validate 100 values (Full mode, no rules involving expressions) | < 5 ms | Execution time check per action invocation |
| Schema JSON round-trip (1000 fields) | < 10 ms | UI editor save/load |
| `Field::string("key")` builder construction | zero alloc beyond `FieldKey` | Every schema definition |
| Memory for 100-field schema | < 50 KB | Cached in action registry |

Measurement via `criterion` benches in `crates/schema/benches/`:

- `bench_build.rs` — Schema construction timing
- `bench_validate.rs` — Validation hot path
- `bench_serde.rs` — Serialization round-trip
- `bench_memory.rs` — Memory footprint

## 9. Module boundaries

`nebula-schema` sits in the **Core layer** (see `CLAUDE.md`):

```
Cross-cutting ── nebula-validator ──┐
                                    ├── nebula-schema (Core)
Core ─── nebula-core ───────────────┘
         nebula-error
         nebula-expression (interface only)
```

**Depends on:**
- `nebula-core` — type primitives, error helpers
- `nebula-error` — crate errors via `thiserror`
- `nebula-validator` — `Rule` enum, `ExecutionMode`, evaluation engine
- `nebula-expression` — interface for expression marker detection (implementation is separate)
- `nebula-schema-macros` — `Input` / `Output` derive macros
- `serde`, `serde_json` — serialization
- `thiserror` — error types
- `schemars` (optional feature) — JSON Schema generation for `Output`

**Does NOT depend on:**
- Any Business-layer crate (`nebula-action`, `nebula-credential`, `nebula-resource`)
- Any Exec-layer crate (`nebula-engine`, `nebula-runtime`, `nebula-storage`)
- Any UI / frontend crate

**Consumers** (reverse direction, consumers of `nebula-schema`):
- `nebula-action::ActionMetadata { input: Schema, output: OutputSchema }`
- `nebula-credential` — credential definitions as `Schema`
- `nebula-resource` — resource config schemas
- `nebula-derive` / `nebula-schema-macros` — `Input` / `Output` derives
- `apps/cli` — `nebula run` validation, `nebula import`
- `apps/desktop` — Tauri bridge for UI rendering
- `apps/web` — (future) web dashboard

## 10. Migration path

### 10.1 PR sequence

**PR 0 — this spec document**
- Add `docs/plans/2026-04-15-arch-specs/21-schema-crate.md` (this file)
- Link from `COMPACT.md` → `Files produced` section
- Link from `../2026-04-15-architecture-review-qa.md` open items

**PR 1 — new crate scaffold**
- Create `crates/schema/` with `Cargo.toml`, `src/lib.rs`
- Implement `FieldKey`, `VisibilityMode`, `RequiredMode`, all 7 widget enums
- Implement `define_field!` macro
- Implement all 18 per-type structs
- Implement `Field` enum wrapper with accessors and `From<>` impls
- Implement `Schema`, `FieldValue`, `FieldValues`, `FieldPath`, `SchemaError`
- Basic serde round-trip tests
- Add `nebula-schema` to workspace `Cargo.toml`
- Green on `cargo check -p nebula-schema && cargo nextest run -p nebula-schema`

**PR 2 — derive macros crate**
- Create `crates/schema-macros/` (`nebula-schema-macros`)
- Implement `#[derive(Input)]` → `HasInputSchema`
- Implement `#[derive(Output)]` → `HasOutputSchema` (delegates to `schemars::schema_for!`)
- Translate all 16 prototype schemas as integration tests
- Green on full workspace check

**PR 3 — migrate `nebula-action::ActionMetadata`**
- Add `input: nebula_schema::Schema` and `output: OutputSchema` fields
- Keep old `parameters: ParameterCollection` temporarily during dual-world
- Update all action definitions to provide both (dual-supply via shim)
- Green on `cargo check --workspace`

**PR 4 — migrate callsites**
- `nebula-credential` — credential schemas become `Schema`
- `nebula-resource` — resource config schemas become `Schema`
- `nebula-engine` — use `Schema::validate` for input validation
- `apps/cli` — parameter rendering via new types
- `nebula-derive` — replace old `Parameters` derive with new `Input`/`Output`
- Green on full workspace + nextest

**PR 5 — delete `nebula-parameter`**
- Remove `crates/parameter/` from workspace
- Remove `crates/parameter-macros/`
- Remove `Cargo.toml` entries
- Remove re-exports
- Run `cargo +nightly fmt --all && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc && cargo deny check`
- Canon fold-in: update §11.10 (parameter system) or create new section

### 10.2 Breaking changes

Anyone outside the workspace using `nebula-parameter` directly must migrate
to `nebula-schema`. Since Nebula is pre-1.0 with no external users, this is
acceptable.

**Internal breakages handled by the migration PRs:**
- `ActionMetadata { parameters: ParameterCollection }` → `ActionMetadata { input: Schema, output: OutputSchema }`
- `Parameter::string(id)` → `Field::string(id)` (returns `StringField`, not `StringBuilder`)
- `Condition::eq(...)` → `Rule::FieldEq { .. }` from validator
- `ParameterValue` → `FieldValue`
- `#[derive(Parameters)]` → `#[derive(Input)]` / `#[derive(Output)]`

### 10.3 Wire format compatibility

**New format** is incompatible with old `ParameterCollection` JSON. Old
workflows stored with old schema format must be migrated once:

```
old: { "id": "timeout", "type": "number", "label": "Timeout", "min": 0, "max": 300 }
new: { "type": "number", "key": "timeout", "label": "Timeout",
       "rules": [{"kind": "min", "min": 0}, {"kind": "max", "max": 300}] }
```

One-time migration tool: `nebula migrate schema old.json new.json` converts
old `ParameterCollection` JSON to new `Schema` JSON. Run once during PR 5
on any stored workflows.

### 10.4 Prototype YAML files

`apps/cli/examples/*.yaml` currently use legacy `{type: literal, value: X}`
wrapper per parameter value. These are outdated — rewrite in PR 1 to the
canonical format (bare literals + inline `{{ }}` expressions + `{mode, value}`
discriminated), matching `FieldValue::from_json` detection logic.

## 11. Open questions

### 11.1 `NumberHint` enum — defer

Currently `NumberField.widget: NumberWidget` includes Percent/Currency/
Duration/Bytes. These overload widget with semantic hints (e.g., Duration
implies unit selector + formatting). Alternative: separate `number_hint:
NumberHint` field, keep widget for rendering style only.

**Decision**: defer until we have 3+ real use cases that require hint
independent from widget. For v1, widget covers both.

### 11.2 Credential schema — different system

`CredentialSchema` derive was originally planned as a third focused macro
alongside `Input`/`Output`. The user has indicated credentials will be a
**separate system** with different semantics — deferred to a future spec.
For now, credentials use `Schema` directly (they're just parameter lists
with `SecretField` for sensitive values).

### 11.3 Template helpers

Removed from this spec on user's feedback. Users compose common patterns
(key-value lists, email lists, filter builders) from primitive types in
their own code. If a pattern becomes widespread, a helper function can be
added to `nebula-schema::templates` in a future PR — not urgent.

### 11.4 Multi-language support in labels

`label: Option<String>` is single-language. Future: `label: LocalizedString`
with per-locale variants. Not urgent for v1; workflow authors can provide
one label and let frontend translate via i18n keys if needed.

### 11.5 Widget subtypes in `SecretWidget`

Current two variants (Plain, Multiline) + `reveal_last: Option<u8>`. Future
additions (if needed):
- `Token` as standalone variant replacing `reveal_last` flag pattern
- `Code` for multiline masked code blocks

**Decision**: `#[non_exhaustive]` enables adding variants later without
breaking changes.

### 11.6 Lint level for escape hatch

`with_rule(Rule)` can cause `Rule::Max` on `StringField`. Current plan:
lint warning. Alternative: make it a schema-time **error** instead of
warning. Error is stricter but may annoy authors who genuinely want custom
rules. Start with warning, escalate if needed.

### 11.7 Default `#[derive(Input)]` behavior for Rust types

Mapping table (tentative):

| Rust type / pattern | Generated Field |
|---|---|
| `String` | `StringField` |
| `&str` (via attribute) | `StringField` |
| `String` + `#[field(options = ["a", "b"])]` | `SelectField` (static options, value=string) |
| `String` + `#[field(widget = "select", loader = "fn_name")]` | `SelectField` (dynamic options via loader) |
| `Vec<String>` + `#[field(multi_select, options = [...])]` | `SelectField` with `.multiple()` |
| `Secret` (newtype `nebula_schema::Secret`) | `SecretField` |
| `i8`..`i64`, `u8`..`u64`, `usize`, `isize` | `NumberField::new(...).integer()` |
| `f32`, `f64` | `NumberField::new(...)` |
| `bool` | `BooleanField` |
| **Unit-only enum** + `#[derive(EnumSelect)]` (all variants are unit, no data) | `SelectField` with `.option()` per variant; value = variant name as string |
| **Data-carrying enum** + `#[derive(Input)]` (at least one variant has fields) | `ModeField` with `.variant()` per enum variant, payload = variant field schema |
| **Mixed enum** (some unit, some data variants) | `ModeField` — unit variants become modes with `HiddenField` payload placeholder |
| `Option<T>` | `T` with `required = Never` (default visibility unchanged) |
| `Vec<T>` (non-scalar `T`) | `ListField::new(...).item(T field)` |
| struct with `#[derive(Input)]` | `ObjectField` (nested collection) |
| `chrono::NaiveDate` | `DateField` |
| `chrono::DateTime<Utc>` / `chrono::DateTime<Local>` | `DateTimeField` |
| `chrono::NaiveTime` | `TimeField` |
| `serde_json::Value` | `CodeField { language: "json" }` (arbitrary JSON editor) |
| `PathBuf` / `std::path::Path` | `FileField` |

**Select vs Mode distinction** (important):

- **Unit-only enum** → `SelectField`. Example:
  ```rust
  #[derive(EnumSelect)]
  enum Priority { Low, Normal, High }
  // Generates: SelectField with options ["low", "normal", "high"]
  // Value: "low" | "normal" | "high" (plain string)
  ```

- **Data-carrying enum** → `ModeField`. Example:
  ```rust
  #[derive(Input)]
  enum Auth {
      None,                                            // unit variant → HiddenField payload
      Bearer(String),                                   // tuple variant → StringField payload
      OAuth2 { client_id: String, scope: String },     // struct variant → ObjectField payload
  }
  // Generates: ModeField with 3 variants
  // Value: { "mode": "bearer", "value": "xyz" } or { "mode": "oauth2", "value": { client_id: ..., scope: ... } }
  ```

- **String with explicit options** (not an enum) → `SelectField`. Example:
  ```rust
  struct MyInput {
      #[field(label = "Method", options = ["GET", "POST", "PUT"])]
      method: String,
  }
  // Generates: SelectField with 3 static options
  ```

- **Dynamic Select** (options from loader) → `SelectField` with loader. Example:
  ```rust
  struct MyInput {
      #[field(
          label = "Channel",
          widget = "select",
          loader = "load_slack_channels",
          depends_on = ["workspace_id"]
      )]
      channel: String,
  }
  fn load_slack_channels(ctx: LoaderContext) -> Result<Vec<SelectOption>, LoaderError> { ... }
  ```

**Rationale for enum split**: unit enums are natural enumerations (closed set of labels with no payload), matching Blender's `EnumProperty` and n8n's `options`. Data-carrying enums carry variant-specific payload and require discriminated-union semantics — exactly what `ModeField` models. Automatic derive dispatch on variant shape keeps DX clean and matches Rust enum conventions.

Subject to refinement during PR 2.

---

## Appendix A — Complete example

Real schema: Stripe create payment (from prototype 14, rewritten in new API).

```rust
use nebula_schema::{Schema, Field, FieldKey, Rule};
use serde_json::json;

fn stripe_create_payment_schema() -> Schema {
    Schema::new()
        .add(Field::integer("amount")
            .label("Amount (in cents)")
            .description("Amount in smallest currency unit (e.g. cents for USD)")
            .required()
            .min(1))

        .add(Field::select("currency")
            .label("Currency")
            .searchable()
            .allow_custom()
            .option("usd", "USD — US Dollar")
            .option("eur", "EUR — Euro")
            .option("gbp", "GBP — British Pound")
            .option("jpy", "JPY — Japanese Yen")
            .default(json!("usd"))
            .required())

        .add(Field::computed("amount_display")
            .label("Display Amount")
            .returns_string()
            .expression("{{ format_currency(amount, currency) }}"))

        .add(Field::string("description")
            .label("Description")
            .placeholder("Payment for order #1234"))

        .add(Field::string("customer_email")
            .label("Customer Email")
            .email()
            .required())

        .add(Field::secret("api_key")
            .label("Stripe Secret Key")
            .required()
            .reveal_last(4)
            .with_rule(Rule::Pattern {
                pattern: r"^sk_(live|test)_[a-zA-Z0-9]{24,}$".into(),
                message: Some("invalid Stripe key format".into()),
            }))

        .add(Field::object("shipping")
            .label("Shipping Address")
            .add(Field::string("name").label("Full Name").required())
            .add(Field::string("line1").label("Address Line 1").required())
            .add(Field::string("line2").label("Address Line 2"))
            .add(Field::string("city").label("City").required())
            .add(Field::string("state").label("State / Region"))
            .add(Field::string("postal_code").label("Postal Code").required())
            .add(Field::select("country")
                .label("Country")
                .searchable()
                .allow_custom()
                .option("US", "United States")
                .option("CA", "Canada")
                .option("GB", "United Kingdom")
                .required()))

        .add(Field::list("metadata")
            .label("Metadata")
            .description("Key-value pairs attached to the payment")
            .item(Field::object("kv")
                .add(Field::string("key").label("Key").required())
                .add(Field::string("value").label("Value").required()))
            .max_items(50))
}
```

## Appendix B — Compile-fail test examples

```rust
// crates/schema/tests/compile_fail/string_no_min.rs
fn main() {
    let _ = nebula_schema::Field::string("x").min(5);
    //~^ ERROR method `min` not found for type `StringField`
}

// crates/schema/tests/compile_fail/number_no_multiline.rs
fn main() {
    let _ = nebula_schema::Field::number("x").multiline();
    //~^ ERROR method `multiline` not found for type `NumberField`
}

// crates/schema/tests/compile_fail/boolean_no_pattern.rs
fn main() {
    let _ = nebula_schema::Field::boolean("x").pattern("yes|no");
    //~^ ERROR method `pattern` not found for type `BooleanField`
}

// crates/schema/tests/compile_fail/secret_no_widget_from_string.rs
fn main() {
    use nebula_schema::StringWidget;
    let _ = nebula_schema::Field::secret("x").widget(StringWidget::Email);
    //~^ ERROR expected `SecretWidget`, found `StringWidget`
}
```

## Changelog

- **2026-04-15** — initial draft. Replaces `nebula-parameter`. Pattern 4
  architecture, 18 primitive types, 7 widget enums, unified `Rule` from
  `nebula-validator` for validation + visibility, dedicated `SecretField`,
  `ActionMetadata { input, output }` integration, derive macro split into
  `Input` / `Output`.
