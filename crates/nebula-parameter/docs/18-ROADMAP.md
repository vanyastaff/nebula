# Nebula Parameters - Implementation Roadmap

**Step-by-step guide to implementation**

Version: 1.0  
Status: Ready for Implementation ✅

---

## Overview

This roadmap provides a structured approach to implementing the Nebula Parameter System, organized into phases with clear deliverables and dependencies.

**Total Estimated Effort:** 8-12 weeks  
**Team Size:** 1-2 developers  
**Prerequisites:** Rust 1.75+, familiarity with Arc, trait objects

---

## Phase 1: Foundation (Week 1-2)

### Goal: Core infrastructure and basic types

### 1.1 Project Setup

**Tasks:**
- [ ] Create workspace structure (`nebula-key`, `nebula-parameter`, `nebula-validator`)
- [ ] Set up Cargo.toml with dependencies
- [ ] Configure CI/CD (GitHub Actions)
- [ ] Set up documentation structure

**Files:**
```
nebula/
├── Cargo.toml (workspace)
├── crates/
│   ├── nebula-key/
│   │   └── Cargo.toml
│   └── nebula-parameter/
│       └── Cargo.toml
```

**Dependencies:**
```toml
tokio = { version = "1", features = ["sync", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
smartstring = "1"
regex = "1"
thiserror = "1"
crossbeam = "0.8"
```

**Deliverable:** Working workspace that compiles

---

### 1.2 Core Types

**Priority: CRITICAL**

**Implementation Order:**

#### Step 1: Key (nebula-key)
Already exists! ✅

#### Step 2: Metadata
```rust
// src/core/metadata.rs
pub struct Metadata {
    key: Key,
    label: Option<String>,
    description: Option<String>,
    group: Option<String>,
    tags: Vec<String>,
}
```

**Test Coverage:** 90%+

#### Step 3: Flags
```rust
// src/core/flags.rs
bitflags! {
    pub struct Flags: u64 {
        const REQUIRED = 1 << 0;
        const READONLY = 1 << 1;
        const HIDDEN = 1 << 2;
        const ADVANCED = 1 << 3;
        const ANIMATABLE = 1 << 4;
        // ... (total: 20 flags)
    }
}

bitflags! {
    pub struct StateFlags: u32 {
        const DIRTY = 1 << 0;
        const TOUCHED = 1 << 1;
        const VALID = 1 << 2;
        const VISIBLE = 1 << 3;
        const ENABLED = 1 << 4;
        const READONLY = 1 << 5;
    }
}
```

**Test Coverage:** 95%+

#### Step 4: Value Enum
```rust
// src/core/value.rs
pub enum Value {
    None,
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
    Vector(Vec<f64>),
}
```

**Test Coverage:** 90%+

**Deliverable:** Core types module compiles with tests

---

### 1.3 Subtypes

**Priority: HIGH**

**Implementation Order:**

#### Week 1: TextSubtype
```rust
// src/core/subtype/text.rs
pub enum TextSubtype {
    Generic, SingleLine, MultiLine, RichText,
    Code, Json, Xml, Yaml, Toml,
    Email, Url, FilePath,
    Uuid, Slug, Username, Secret,
    // ... (56 total)
    Custom(String),
}

impl TextSubtype {
    pub fn is_code(&self) -> bool;
    pub fn is_sensitive(&self) -> bool;
    pub fn is_structured(&self) -> bool;
    pub fn mime_type_hint(&self) -> Option<&'static str>;
}
```

**Files:** Use provided `text.rs` ✅

#### Week 2: NumberSubtype + Unit
```rust
// src/core/subtype/number.rs
pub enum NumberSubtype {
    Integer, Float, Decimal, Percentage,
    Currency, Price, Temperature, Distance,
    // ... (60 total)
    Custom(String),
}

// src/core/unit.rs
pub enum Unit {
    None,
    Temperature(TemperatureUnit),
    Distance(DistanceUnit),
    // ... (17 categories)
}
```

**Files:** Use provided `number.rs` and `unit.rs` ✅

#### Week 2: VectorSubtype
```rust
// src/core/subtype/vector.rs
pub enum VectorSubtype {
    Vector2, Vector3, Vector4,
    ColorRgb, ColorRgba,
    Quaternion, Matrix4x4,
    // ... (35 total)
    Custom(String),
}

impl VectorSubtype {
    pub fn component_count(&self) -> Option<usize>;
    pub fn component_names(&self) -> Option<&[&'static str]>;
}
```

**Files:** Use provided `vector.rs` ✅

**Deliverable:** All subtypes implemented with helper methods and tests

---

## Phase 2: Parameter Types (Week 3-4)

### Goal: Implement all parameter types

### 2.1 Parameter Trait

```rust
// src/parameter/base.rs
pub trait Parameter: Send + Sync {
    fn metadata(&self) -> &Metadata;
    fn param_type(&self) -> ParameterType;
    fn validate(&self, value: &Value) -> ValidationResult<()>;
    fn transform(&self, value: Value) -> Value;
    fn default_value(&self) -> Value;
}
```

**Deliverable:** Parameter trait with documentation

---

### 2.2 TextParameter

**Priority: CRITICAL (most common type)**

```rust
// src/parameter/text.rs
pub struct TextParameter {
    metadata: Metadata,
    flags: Flags,
    subtype: TextSubtype,
    min_length: Option<usize>,
    max_length: Option<usize>,
    pattern: Option<Regex>,
    allowed_values: Option<Vec<String>>,
    validators: Vec<Arc<dyn Validator>>,
    transformers: Vec<Arc<dyn Transformer>>,
    display: Option<ParameterDisplay>,
}

// Builder
pub struct TextParameterBuilder { ... }

impl TextParameter {
    pub fn builder(key: impl Into<Key>) -> TextParameterBuilder;
    pub fn email(key: impl Into<Key>) -> TextParameterBuilder;
    pub fn url(key: impl Into<Key>) -> TextParameterBuilder;
    pub fn password(key: impl Into<Key>) -> TextParameterBuilder;
}
```

**Test Coverage:** 90%+

**Deliverable:** TextParameter with builder and tests

---

### 2.3 NumberParameter

```rust
// src/parameter/number.rs
pub struct NumberParameter {
    metadata: Metadata,
    flags: Flags,
    subtype: NumberSubtype,
    unit: Option<Unit>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    validators: Vec<Arc<dyn Validator>>,
}

// Builder
impl NumberParameter {
    pub fn integer(key: impl Into<Key>) -> NumberParameterBuilder;
    pub fn float(key: impl Into<Key>) -> NumberParameterBuilder;
    pub fn percentage(key: impl Into<Key>) -> NumberParameterBuilder;
}
```

**Test Coverage:** 90%+

---

### 2.4 BoolParameter

```rust
// src/parameter/boolean.rs
pub struct BoolParameter {
    metadata: Metadata,
    flags: Flags,
    default: bool,
    display: Option<ParameterDisplay>,
}

// Simple - no subtype!
```

**Test Coverage:** 95%+

---

### 2.5 ChoiceParameter

```rust
// src/parameter/choice.rs
pub struct ChoiceParameter {
    metadata: Metadata,
    flags: Flags,
    mode: ChoiceMode,
    options: Vec<ChoiceOption>,
}

pub enum ChoiceMode {
    Single,
    Multi,
}

pub struct ChoiceOption {
    value: String,
    label: Option<String>,
    icon: Option<String>,
    enabled: bool,
}
```

**Test Coverage:** 90%+

---

### 2.6 VectorParameter

```rust
// src/parameter/vector.rs
pub struct VectorParameter {
    metadata: Metadata,
    flags: Flags,
    subtype: VectorSubtype,
    validators: Vec<Arc<dyn Validator>>,
}

impl VectorParameter {
    pub fn vector3(key: impl Into<Key>) -> VectorParameterBuilder;
    pub fn color_rgba(key: impl Into<Key>) -> VectorParameterBuilder;
    pub fn matrix4x4(key: impl Into<Key>) -> VectorParameterBuilder;
}
```

**Test Coverage:** 90%+

---

### 2.7 ArrayParameter, ObjectParameter

**Priority: MEDIUM (can be Phase 3)**

```rust
// src/parameter/array.rs
pub struct ArrayParameter {
    metadata: Metadata,
    flags: Flags,
    element_type: Box<dyn Parameter>,
    min_items: Option<usize>,
    max_items: Option<usize>,
}

// src/parameter/object.rs
pub struct ObjectParameter {
    metadata: Metadata,
    flags: Flags,
    properties: Vec<Arc<dyn Parameter>>,
}
```

**Test Coverage:** 85%+

**Deliverable:** All parameter types implemented

---

## Phase 3: Schema and Runtime (Week 4-5)

### Goal: Schema definition and runtime state

### 3.1 Schema

```rust
// src/schema/schema.rs
pub struct Schema {
    parameters: Vec<Arc<dyn Parameter>>,
    groups: Vec<ParameterGroup>,
}

pub struct SchemaBuilder {
    parameters: Vec<Arc<dyn Parameter>>,
}

impl Schema {
    pub fn new() -> SchemaBuilder;
    pub fn parameters(&self) -> &[Arc<dyn Parameter>];
    pub fn get_parameter(&self, key: &str) -> Option<&Arc<dyn Parameter>>;
}
```

**Test Coverage:** 90%+

---

### 3.2 RuntimeParameter

```rust
// src/runtime/parameter.rs
pub struct RuntimeParameter<T: Parameter> {
    schema: Arc<T>,
    state: ParameterState,
    value: Value,
    event_bus: EventBus,
}

pub struct ParameterState {
    flags: StateFlags,
    errors: Vec<ValidationError>,
    modified_at: Option<Instant>,
}

impl<T: Parameter> RuntimeParameter<T> {
    pub fn new(schema: Arc<T>, event_bus: EventBus) -> Self;
    pub fn set_value(&mut self, value: Value) -> ValidationResult<()>;
    pub fn get_value(&self) -> &Value;
    pub fn validate(&mut self) -> bool;
    pub fn is_dirty(&self) -> bool;
    pub fn is_touched(&self) -> bool;
}
```

**Test Coverage:** 95%+

---

### 3.3 Context

```rust
// src/context/context.rs
pub struct Context {
    schema: Arc<Schema>,
    parameters: HashMap<Key, RuntimeParameter<dyn Parameter>>,
    event_bus: EventBus,
    // history and display_observer added in Phase 4
}

impl Context {
    pub fn new(schema: Schema) -> Self;
    pub fn get_value(&self, key: &str) -> Option<&Value>;
    pub fn set_value(&mut self, key: &str, value: Value) -> Result<()>;
    pub fn validate(&mut self, key: &str) -> bool;
    pub fn validate_all(&mut self) -> bool;
}
```

**Test Coverage:** 90%+

**Deliverable:** Schema, Runtime, Context working together

---

## Phase 4: Reactive Systems (Week 5-7)

### Goal: Events, observers, undo/redo

### 4.1 Event System

**Week 5**

```rust
// src/event/event.rs
pub enum ParameterEvent {
    ValueChanging { key: Key, old_value: Value, new_value: Value },
    ValueChanged { key: Key, old_value: Value, new_value: Value },
    Validated { key: Key, is_valid: bool, errors: Vec<ValidationError> },
    Dirtied { key: Key },
    Touched { key: Key },
    VisibilityChanged { key: Key, visible: bool },
    BatchBegin { description: String },
    BatchEnd { description: String },
}

// src/event/bus.rs
pub struct EventBus {
    tx: broadcast::Sender<ParameterEvent>,
    observers: Mutex<HashMap<SubscriptionId, Box<dyn Observer>>>,
    batch_state: Mutex<BatchState>,
}
```

**Deliverable:** Event system with tokio::broadcast

---

### 4.2 Built-in Observers

**Week 5-6**

```rust
// src/observer/logger.rs
pub struct LoggerObserver { ... }

// src/observer/validation.rs
pub struct ValidationObserver { ... }

// src/observer/dependency.rs
pub struct DependencyObserver { ... }

// src/observer/ui.rs
pub struct UiObserver {
    sender: mpsc::Sender<UiUpdate>,
}
```

**Test Coverage:** 85%+

---

### 4.3 Validation System

**Week 6**

```rust
// src/validation/validator.rs
pub trait Validator: Send + Sync {
    fn validate(&self, value: &Value) -> ValidationResult<()>;
}

// src/validation/builtin.rs
pub struct RequiredValidator;
pub struct MinLengthValidator(usize);
pub struct MaxLengthValidator(usize);
pub struct RegexValidator(Regex);
pub struct EmailValidator;
pub struct UrlValidator;
pub struct RangeValidator { min: f64, max: f64 }
```

**Deliverable:** Validation system with 10+ built-in validators

---

### 4.4 Transformer System

**Week 6**

```rust
// src/transformer/transformer.rs
pub trait Transformer: Send + Sync {
    fn transform(&self, value: Value) -> Value;
}

// src/transformer/builtin.rs
pub struct TrimTransformer;
pub struct LowercaseTransformer;
pub struct UppercaseTransformer;
pub struct StripWhitespaceTransformer;
```

**Deliverable:** Transformer system with 5+ built-in transformers

---

### 4.5 History System (Undo/Redo)

**Week 7**

```rust
// src/history/command.rs
pub trait Command: Send + Sync {
    fn execute(&mut self, ctx: &mut Context) -> Result<()>;
    fn undo(&mut self, ctx: &mut Context) -> Result<()>;
    fn redo(&mut self, ctx: &mut Context) -> Result<()>;
    fn merge(&mut self, other: &dyn Command) -> bool;
}

pub struct SetValueCommand {
    key: Key,
    old_value: Value,
    new_value: Value,
}

// src/history/macro_command.rs
pub struct MacroCommand {
    commands: Vec<Box<dyn Command>>,
    description: String,
}

// src/history/manager.rs
pub struct HistoryManager {
    undo_stack: VecDeque<Box<dyn Command>>,
    redo_stack: VecDeque<Box<dyn Command>>,
    max_history: usize,
    current_transaction: Option<MacroCommand>,
}
```

**Test Coverage:** 90%+

**Deliverable:** Full undo/redo with transactions

---

## Phase 5: Display System (Week 7-8)

### Goal: Conditional visibility

### 5.1 Display Conditions

```rust
// src/core/display/condition.rs
pub enum DisplayCondition {
    Equals(Value),
    NotEquals(Value),
    IsSet, IsNull,
    IsEmpty, IsNotEmpty,
    IsTrue, IsFalse,
    GreaterThan(f64),
    LessThan(f64),
    InRange { min: f64, max: f64 },
    Contains(String),
    StartsWith(String),
    EndsWith(String),
    OneOf(Vec<Value>),
    IsValid,
    IsInvalid,
}
```

**Deliverable:** Display condition system

---

### 5.2 Display Rules

```rust
// src/core/display/rule.rs
pub struct DisplayRule {
    pub field: Key,
    pub condition: DisplayCondition,
}

pub enum DisplayRuleSet {
    Single(DisplayRule),
    All(Vec<DisplayRuleSet>),
    Any(Vec<DisplayRuleSet>),
    Not(Box<DisplayRuleSet>),
}

pub struct ParameterDisplay {
    show_when: Option<DisplayRuleSet>,
    hide_when: Option<DisplayRuleSet>,
}
```

**Deliverable:** Display rule system

---

### 5.3 DisplayObserver

```rust
// src/observer/display.rs
pub struct DisplayObserver {
    context: DisplayContext,
    displays: HashMap<Key, ParameterDisplay>,
    dependencies: HashMap<Key, HashSet<Key>>,
}

impl Observer for DisplayObserver {
    fn on_event(&mut self, event: &ParameterEvent) {
        // Update context → recalculate visibility → emit events
    }
}
```

**Test Coverage:** 90%+

**Deliverable:** Reactive display system integrated with events

---

## Phase 6: Polish and Optimization (Week 9-10)

### Goal: Performance, docs, examples

### 6.1 Performance Optimization

**Tasks:**
- [ ] Benchmark critical paths
- [ ] Optimize hot loops
- [ ] Profile memory usage
- [ ] Reduce allocations

**Targets:**
- Event dispatch: <200ns
- Validation: <1µs per validator
- Command execution: <500ns

---

### 6.2 Documentation

**Tasks:**
- [ ] Complete API documentation (100% coverage)
- [ ] Write user guide
- [ ] Write architecture guide
- [ ] Create examples (10+)

**Examples:**
- Basic usage
- Form validation
- Workflow automation
- Undo/redo
- Reactive UI
- 3D transform editor
- Game settings
- CLI tool
- Custom validators
- Custom transformers

---

### 6.3 Testing

**Target Coverage:**
- Core types: 95%+
- Parameter types: 90%+
- Runtime: 90%+
- Event system: 85%+
- History: 90%+
- Display: 90%+
- **Overall: 90%+**

**Test Types:**
- Unit tests
- Integration tests
- Property-based tests (proptest)
- Benchmarks (Criterion)

---

## Phase 7: UI Integration (Week 11-12)

### Goal: egui integration (example)

**Priority: OPTIONAL (proof of concept)**

### 7.1 UI Metadata

```rust
// src/ui/hints.rs (feature-gated)
pub struct UIHints {
    widget_type: WidgetType,
    placeholder: Option<String>,
    icon: Option<String>,
    tooltip: Option<String>,
}

pub enum WidgetType {
    TextInput,
    TextArea,
    NumberInput,
    Slider,
    Checkbox,
    Toggle,
    Dropdown,
    ColorPicker,
    // ...
}
```

---

### 7.2 egui Integration Example

```rust
// examples/egui_form.rs
fn render_parameter(ui: &mut egui::Ui, param: &RuntimeParameter) {
    match param.schema.param_type() {
        ParameterType::Text => {
            ui.text_edit_singleline(param.value.as_str_mut());
        }
        ParameterType::Number => {
            ui.add(egui::Slider::new(param.value.as_f64_mut(), 0.0..=100.0));
        }
        // ...
    }
}
```

**Deliverable:** Working egui example

---

## Implementation Guidelines

### Code Quality Standards

**Required:**
- [ ] Clippy clean (no warnings)
- [ ] Rustfmt formatted
- [ ] Documentation for all public APIs
- [ ] Tests for all features
- [ ] No `unsafe` without justification
- [ ] Error handling with `thiserror`
- [ ] Serialization with `serde`

### Git Workflow

**Branches:**
- `main` - stable releases
- `develop` - integration branch
- `feature/*` - feature branches
- `fix/*` - bug fix branches

**Commit Messages:**
```
type(scope): subject

- feat: new feature
- fix: bug fix
- docs: documentation
- test: tests
- refactor: code refactoring
- perf: performance improvement
```

### Code Review

**Required Reviewers:** 1+  
**Merge Requirements:**
- [ ] All tests pass
- [ ] Code coverage maintained
- [ ] Documentation updated
- [ ] Changelog updated

---

## Milestones

### Milestone 1: Foundation Complete (Week 2)
- Core types
- Subtypes
- ✅ **Deliverable:** Core module compiles

### Milestone 2: Parameters Complete (Week 4)
- All parameter types
- Builders
- ✅ **Deliverable:** Can define schemas

### Milestone 3: Runtime Complete (Week 5)
- Schema
- RuntimeParameter
- Context
- ✅ **Deliverable:** Can create and use parameters

### Milestone 4: Reactive Complete (Week 7)
- Events
- Observers
- History
- ✅ **Deliverable:** Full reactive system

### Milestone 5: Display Complete (Week 8)
- Display conditions
- DisplayObserver
- ✅ **Deliverable:** Conditional visibility

### Milestone 6: Production Ready (Week 10)
- Performance optimized
- Fully documented
- Comprehensive tests
- ✅ **Deliverable:** v1.0 release

### Milestone 7: UI Example (Week 12)
- egui integration
- Example application
- ✅ **Deliverable:** Working demo

---

## Risk Management

### Technical Risks

**Risk 1: Performance Issues**
- **Mitigation:** Benchmark early, optimize hot paths
- **Fallback:** Profile-guided optimization

**Risk 2: Type Erasure Complexity**
- **Mitigation:** Use proven patterns (Arc<dyn Trait>)
- **Fallback:** Simplify if needed

**Risk 3: Event System Overhead**
- **Mitigation:** Use efficient channel (tokio::broadcast)
- **Fallback:** Optional events (feature-gated)

### Schedule Risks

**Risk 1: Scope Creep**
- **Mitigation:** Strict phase boundaries, defer non-critical features
- **Fallback:** Move Phase 7 to v2.0

**Risk 2: Testing Takes Longer**
- **Mitigation:** Write tests alongside code
- **Fallback:** Extend Week 9-10

---

## Success Criteria

### Must Have (v1.0)
- ✅ All parameter types working
- ✅ Schema and runtime functional
- ✅ Event system operational
- ✅ Undo/redo working
- ✅ Display conditions working
- ✅ 90%+ test coverage
- ✅ Complete documentation

### Nice to Have (v1.1)
- ⭐ egui integration
- ⭐ Additional examples
- ⭐ Performance benchmarks published

### Future (v2.0)
- 🚀 iced integration
- 🚀 Web assembly support
- 🚀 Async validation
- 🚀 Plugin system

---

## Getting Started

### Day 1 Tasks

1. Clone repo structure
2. Set up Cargo workspace
3. Add dependencies
4. Implement Key (already done)
5. Implement Metadata
6. Write first tests

### Week 1 Goals

- Core types module complete
- TextSubtype complete
- Tests passing
- Documentation started

**LET'S BUILD IT!** 🚀
