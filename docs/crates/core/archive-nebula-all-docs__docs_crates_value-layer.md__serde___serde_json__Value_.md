# Archived From "docs/archive/nebula-all-docs.md"

## FILE: docs/crates/value-layer.md (serde / serde_json::Value)
---

# Value layer: serde / serde_json::Value

Отдельный crate nebula-value не используется. Единый тип данных в runtime — `serde_json::Value`, сериализация — через serde.

## Ответственность

- Данные workflow: `serde_json::Value`
- Сериализация/десериализация: serde
- Валидация: nebula-validator или core поверх Value

---

