# Archived From "docs/archive/nebula-complete.md"

#### 4. nebula-derive (Week 3)
- [ ] 4.1 **Macro Setup**
  - [ ] 4.1.1 Create proc-macro crate
  - [ ] 4.1.2 Setup syn and quote
  - [ ] 4.1.3 Add error handling
  - [ ] 4.1.4 Setup testing framework

- [ ] 4.2 **Parameters Derive**
  - [ ] 4.2.1 Parse struct attributes
  - [ ] 4.2.2 Generate parameter_collection()
  - [ ] 4.2.3 Generate from_values()
  - [ ] 4.2.4 Add validation attributes
  - [ ] 4.2.5 Add display attributes
  - [ ] 4.2.6 Generate documentation

- [ ] 4.3 **Action Derive**
  - [ ] 4.3.1 Parse action attributes
  - [ ] 4.3.2 Generate metadata
  - [ ] 4.3.3 Generate boilerplate
  - [ ] 4.3.4 Add node attributes
  - [ ] 4.3.5 Validate at compile time

### Phase 2: Execution Engine (Weeks 4-6)

---

## nebula-derive

### Purpose
Procedural macros для уменьшения boilerplate кода при создании nodes и parameters.

### Responsibilities
- Генерация кода для Parameters
- Генерация кода для Actions
- Compile-time валидация
- Автоматическая документация

### Architecture
```rust
// Макросы
#[proc_macro_derive(Parameters, attributes(param, validate, display))]
#[proc_macro_derive(Action, attributes(action, node))]
#[proc_macro_attribute]
pub fn node(args: TokenStream, input: TokenStream) -> TokenStream
```

### Roadmap Details

#### 4.1 Macro Setup
- [ ] 4.1.1 **Create proc-macro crate**
  - Setup Cargo.toml with proc-macro = true
  - Add syn, quote, proc-macro2 dependencies
  - Create lib.rs structure

- [ ] 4.1.2 **Setup syn and quote**
  - Configure feature flags
  - Setup parsing infrastructure
  - Create helper modules

- [ ] 4.1.3 **Add error handling**
  - Create error types
  - Add span information
  - Implement error recovery

- [ ] 4.1.4 **Setup testing framework**
  - Add trybuild for compile tests
  - Create test fixtures
  - Setup expansion tests

#### 4.2 Parameters Derive
- [ ] 4.2.1 **Parse struct attributes**
  - Parse field types
  - Extract param attributes
  - Handle nested attributes

- [ ] 4.2.2 **Generate parameter_collection()**
  - Create ParameterCollection
  - Add each field as parameter
  - Handle Option types

- [ ] 4.2.3 **Generate from_values()**
  - Extract values by key
  - Type conversion
  - Error handling

- [ ] 4.2.4 **Add validation attributes**
  - #[validate(required)]
  - #[validate(min = 1, max = 100)]
  - #[validate(regex = "pattern")]

- [ ] 4.2.5 **Add display attributes**
  - #[display(show_when(...))]
  - #[display(hide_when(...))]
  - Conditional visibility

- [ ] 4.2.6 **Generate documentation**
  - Extract doc comments
  - Generate parameter descriptions
  - Add to metadata

---

