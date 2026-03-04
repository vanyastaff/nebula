# План: убрать крейты ports и drivers

## Цель

Избавиться от отдельных крейтов `nebula-ports` и папки `drivers/`. Контракты персистенции — в **одном** месте (nebula-storage), чтобы workflow и execution не тянули репозитории, с которыми сами не работают.

## Итоговая схема

| Контракт / реализация | Где живёт | Обоснование |
|------------------------|-----------|-------------|
| `WorkflowRepo`, `ExecutionRepo` + ошибки + `InMemoryWorkflowRepo` | **nebula-storage** | Вся персистенция (KV + workflow/execution репозитории) в одном крейте; API и app только используют эти traits |
| `TaskQueue`, `SandboxRunner` + `MemoryQueue`, `InProcessSandbox` | **nebula-runtime** | Очередь и сандбокс — часть рантайма, не персистенция |
| `resource-postgres` | **crates/resource-postgres** | Адаптер Resource, не порт; вынести из `drivers/` в корень `crates/` |

## Что сделано

1. **nebula-storage:** добавлены модули `workflow_repo` и `execution_repo` (traits, ошибки, `InMemoryWorkflowRepo`). API зависит только от storage для репозиториев.
2. **nebula-workflow / nebula-execution:** репозитории убраны; крейты остаются чисто доменными (типы, валидация, графы).
3. **nebula-api:** зависит от `nebula-storage`; state и services используют `storage::WorkflowRepo`, `storage::ExecutionRepo`.
4. **nebula-runtime:** оставлены `TaskQueue` и `SandboxRunner` + in-memory/inprocess реализации (как и было).
5. **resource-postgres:** перенесён в `crates/resource-postgres`.
6. Крейты **ports**, **drivers/** удалены: папки `crates/ports`, `crates/drivers/queue-memory`, `crates/drivers/sandbox-inprocess`, `crates/drivers/resource-postgres` и пустая `crates/drivers` удалены. `cargo check --workspace` проходит.
