# Archived From "docs/archive/final.md"

### nebula-derive
**Назначение:** Процедурные макросы для code generation.

**Макросы:**
- `#[derive(Action)]` - автогенерация Action boilerplate
- `#[derive(Parameters)]` - генерация параметров
- `#[derive(Workflow)]` - декларативные workflows
- `#[derive(Resource)]` - resource definitions

```rust
// Макрос Action генерирует
#[derive(Action)]
#[action(id = "test.action", name = "Test Action")]
pub struct TestAction;

// Превращается в:
impl Action for TestAction {
    fn metadata(&self) -> &ActionMetadata {
        static METADATA: Lazy<ActionMetadata> = Lazy::new(|| {
            ActionMetadata {
                id: ActionId::new("test.action"),
                name: "Test Action".to_string(),
                // ...
            }
        });
        &METADATA
    }
    // ... остальная имплементация
}
```

---

