# Archived From "docs/archive/node-execution.md"

### nebula-parameter
**Назначение:** Типобезопасная система параметров с валидацией и expression support.

**Ключевые возможности:**
- Декларативное определение параметров
- Автоматическая валидация
- Expression resolution
- UI metadata generation

```rust
// Программный подход
let parameters = ParameterCollection::new()
    .add_required("email", ParameterType::String {
        pattern: Some(r"^[^@]+@[^@]+$"),
    })
    .add_optional("age", ParameterType::Integer {
        min: Some(18),
        max: Some(150),
    })
    .add_conditional_parameter(ConditionalParameter {
        parameter_name: "send_notification",
        condition: "age >= 18",
        show_when: true,
    });

// Derive подход
#[derive(Parameters)]
pub struct UserRegistrationParams {
    #[parameter(description = "Email address", validation = "email")]
    pub email: String,
    
    #[parameter(description = "User age", min = 18, max = 150)]
    pub age: u8,
    
    #[parameter(
        description = "Send notification",
        show_when = "age >= 18"  // Условный параметр
    )]
    pub send_notification: bool,
    
    #[parameter(
        description = "Scheduled time",
        expression_type = "DateTime"  // Поддержка expressions
    )]
    pub scheduled_at: Option<String>,  // "$nodes.scheduler.result.time"
}

// Expression parameters — используют ParamValue из nebula-core
let params = hashmap! {
    "to" => ParamValue::Expression(Expression {
        raw: "$nodes.user_lookup.result.email".into()
    }),
    "subject" => ParamValue::Template(TemplateString {
        template: "Order #{order_id} for {name}".into(),
        bindings: vec![
            "$nodes.order.result.id".into(),
            "$user.name".into(),
        ],
    }),
};
```

---

