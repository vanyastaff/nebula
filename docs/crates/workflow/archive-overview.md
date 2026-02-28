# Archived From "docs/archive/overview.md"

### nebula-workflow
**Назначение:** Декларативное определение workflow - описывает "что нужно делать".

**Ключевые компоненты:**
- WorkflowDefinition - структура workflow
- NodeDefinition - узлы workflow
- Connection - связи между узлами
- Validation - проверка корректности

```rust
// Пример определения workflow
let workflow = WorkflowDefinition {
    id: WorkflowId::new("user-registration"),
    name: "User Registration Process",
    nodes: vec![
        NodeDefinition {
            id: NodeId::new("validate"),
            action_id: ActionId::new("validation.user_data"),
            parameters: params!{
                "email_pattern" => ParamValue::Literal(json!("^[^@]+@[^@]+$")),
                "required_fields" => ParamValue::Literal(json!(["email", "password"]))
            },
        },
        NodeDefinition {
            id: NodeId::new("create_user"),
            action_id: ActionId::new("database.insert"),
            parameters: params!{
                "table" => ParamValue::Literal(json!("users")),
                // Expression — данные из предыдущего узла,
                // резолвится в serde_json::Value на этапе execution
                "data" => ParamValue::Expression(Expression {
                    raw: "$nodes.validate.result.validated_data".into()
                })
            },
        }
    ],
    connections: vec![
        Connection {
            from_node: "validate",
            to_node: "create_user",
            condition: Some("$nodes.validate.success"),
        }
    ],
};
```

---

