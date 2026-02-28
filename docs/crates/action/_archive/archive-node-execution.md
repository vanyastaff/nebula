# Archived From "docs/archive/node-execution.md"

### nebula-action
**Назначение:** Система Actions - атомарных единиц работы с гибким подходом к разработке.

**Подходы к разработке:**
1. **Simple approach** - для быстрых решений
2. **Derive macros** - для полноценной интеграции
3. **Trait approach** - для максимального контроля

```rust
// Подход 1: Простой код
pub struct SimpleEmailAction;

impl SimpleAction for SimpleEmailAction {
    type Input = EmailInput;
    type Output = EmailOutput;
    
    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        let smtp = ctx.get_credential("smtp").await?;
        let client = EmailClient::new(&smtp);
        let message_id = client.send(&input.to, &input.subject, &input.body).await?;
        Ok(EmailOutput { message_id })
    }
}

// Подход 2: Derive макросы
#[derive(Action)]
#[action(
    id = "database.user_lookup",
    name = "User Database Lookup",
    description = "Look up user with caching"
)]
#[resources([DatabaseResource, CacheResource])]
#[credentials(["database"])]
pub struct UserLookupAction;

#[derive(Parameters)]
pub struct UserLookupInput {
    #[parameter(description = "User ID to lookup")]
    pub user_id: String,
    
    #[parameter(description = "Use cache", default = true)]
    pub use_cache: bool,
}

impl ProcessAction for UserLookupAction {
    type Input = UserLookupInput;
    type Output = User;
    
    async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
        let db = ctx.get_resource::<DatabaseResource>().await?;
        let cache = ctx.get_resource::<CacheResource>().await?;
        
        // Проверяем кеш
        if input.use_cache {
            if let Some(user) = cache.get(&input.user_id).await? {
                return Ok(ActionResult::Success(user));
            }
        }
        
        // Загружаем из БД
        let user = db.query_one("SELECT * FROM users WHERE id = $1", &[&input.user_id]).await?;
        cache.set(&input.user_id, &user).await?;
        
        Ok(ActionResult::Success(user))
    }
}
```

**Типы Actions:**
- `SimpleAction` - простые операции
- `ProcessAction` - обработка данных
- `StatefulAction` - с состоянием между вызовами
- `TriggerAction` - источники событий
- `SupplyAction` - поставщики ресурсов

---

