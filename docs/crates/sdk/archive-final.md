# Archived From "docs/archive/final.md"

### nebula-sdk
**Назначение:** Публичное API для разработчиков.

**Модули:**
- `prelude` - часто используемые типы
- `action` - разработка Actions
- `workflow` - создание Workflows
- `testing` - утилиты тестирования

```rust
// nebula-sdk/src/prelude.rs
pub use nebula_action::{Action, SimpleAction, ProcessAction};
pub use nebula_workflow::{WorkflowBuilder, NodeBuilder};
pub use nebula_core::{ParamValue, Expression, TemplateString};
pub use nebula_parameter::{Parameters, Parameter};
pub use serde_json::{json, Value};

// Удобные макросы
#[macro_export]
macro_rules! workflow {
    ($name:expr => {
        $($node:ident: $action:expr $(=> $next:ident)?)*
    }) => {
        WorkflowBuilder::new($name)
            $(.add_node(stringify!($node), $action))*
            $($(.connect(stringify!($node), stringify!($next)))?)*
            .build()
    };
}

// Пример использования SDK
use nebula_sdk::prelude::*;

#[derive(Action)]
#[action(id = "my.custom_action")]
pub struct MyAction;

impl SimpleAction for MyAction {
    type Input = MyInput;
    type Output = MyOutput;
    
    // Action получает уже resolve-нные serde_json::Value
    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        // Implementation
    }
}

let workflow = workflow!("my-workflow" => {
    input: "validation.input" => process
    process: "my.custom_action" => output
    output: "notification.send"
});
```

---

