# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Presentation Layer

### nebula-ui
```toml
# Frontend - отдельный стек (React/Vue/etc)
# Взаимодействует через nebula-api
```

## Правила зависимостей

1. **Никакие крейты не зависят от Presentation Layer**
2. **Developer Tools зависят только от нижних слоев**
3. **Execution Layer - центр координации, использует почти все Core и Node**
4. **Cross-cutting доступны всем через optional features**
5. **nebula-derive всегда optional**

