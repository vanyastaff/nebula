# Archived From "docs/archive/overview.md"

### nebula-expression
**Назначение:** Мощный язык выражений для динамической обработки данных.

**Ключевые возможности:**
- Доступ к результатам узлов: `$nodes.user_lookup.result.email`
- Условная логика: `if $user.premium then ... else ...`
- Pipeline операции: `$array | filter(...) | map(...) | sort(...)`
- String interpolation: `"Hello ${user.name}!"`
- Null safety: `$user?.address?.city ?? "Unknown"`

Все значения представлены как `serde_json::Value`. Expressions из `ParamValue::Expression` 
резолвятся на уровне Execution Layer до передачи в Action.

```rust
// Примеры expressions
let examples = vec![
    // Простой доступ
    "$nodes.input.result.user_email",
    
    // Условная логика
    "if $user.premium && $order.amount > 1000 then 'vip' else 'standard'",
    
    // Pipeline обработка
    r#"$nodes.fetch_users.result
       | filter(user => user.active == true)
       | map(user => user.email)
       | take(10)"#,
    
    // String template
    "${workflow.variables.base_url}/users/${nodes.create_user.result.id}",
];

// Использование — результат всегда serde_json::Value
let result: serde_json::Value = context.evaluate_expression(expression).await?;
```

---

