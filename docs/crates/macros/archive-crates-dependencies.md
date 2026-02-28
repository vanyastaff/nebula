# Archived From "docs/archive/crates-dependencies.md"

### Source section: ## Базовые крейты (не зависят от других Nebula крейтов)

### nebula-derive
```toml
[dependencies]
syn = { version = "2.0", features = ["full", "derive", "extra-traits"] }
quote = "1.0"
proc-macro2 = "1.0"
```
Процедурные макросы, используется как optional dependency везде.

## Infrastructure Layer

