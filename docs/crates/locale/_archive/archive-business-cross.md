# Archived From "docs/archive/business-cross.md"

### nebula-locale
**Назначение:** Локализация и интернационализация.

```rust
// Fluent формат локализации
/*
# en-US.ftl
welcome = Welcome { $user }!
error-validation = Field { $field } is invalid: { $reason }

# ru-RU.ftl  
welcome = Добро пожаловать, { $user }!
error-validation = Поле { $field } некорректно: { $reason }
*/

// Использование
let locale_manager = LocaleManager::new()
    .add_locale("en-US", "locales/en-US.ftl")
    .add_locale("ru-RU", "locales/ru-RU.ftl");

// Автоматический выбор локали
let msg = t!("welcome", user = "John");

// Локализация ошибок
impl ActionError {
    pub fn localized(&self, locale: &LocaleContext) -> String {
        match self {
            Self::ValidationFailed { field, reason } => {
                t!("error-validation", field = field, reason = reason)
            }
        }
    }
}
```

---

