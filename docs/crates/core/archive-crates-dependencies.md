# Archived From "docs/archive/crates-dependencies.md"

## Global Sections

# Полная карта зависимостей крейтов Nebula

## Базовые крейты (не зависят от других Nebula крейтов)

---

## Правила зависимостей

1. **Никакие крейты не зависят от Presentation Layer**
2. **Developer Tools зависят только от нижних слоев**
3. **Execution Layer - центр координации, использует почти все Core и Node**
4. **Cross-cutting доступны всем через optional features**
5. **nebula-derive всегда optional**
---

### Source section: ## Базовые крейты (не зависят от других Nebula крейтов)

### nebula-core
```toml
[dependencies]
uuid = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
```
Экспортирует базовые типы для всей системы, включая `ParamValue` для разделения expressions и литеральных значений.

