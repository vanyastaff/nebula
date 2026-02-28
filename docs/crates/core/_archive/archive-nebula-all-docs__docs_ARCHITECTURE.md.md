# Archived From "docs/archive/nebula-all-docs.md"

## FILE: docs/ARCHITECTURE.md
---

# Архитектура Nebula

## Общий обзор

Nebula построена как модульная система с четким разделением ответственности между компонентами.

### Основные принципы

1. **Type Safety First** - Максимальное использование системы типов Rust
2. **Zero-Cost Abstractions** - Производительность без компромиссов
3. **Progressive Complexity** - Простой старт, возможность глубокой кастомизации
4. **Event-Driven** - Асинхронная, event-based архитектура

### Высокоуровневая архитектура

```
┌─────────────────────────────────────────────────────────┐
│                    User Interface                        │
│              (Web UI / Desktop App / CLI)                │
└─────────────────────────────────────────────────────────┘
                            │
┌─────────────────────────────────────────────────────────┐
│                      API Layer                           │
│                  (REST + WebSocket)                       │
└─────────────────────────────────────────────────────────┘
                            │
┌─────────────────────────────────────────────────────────┐
│                  Orchestration Layer                     │
├─────────────────┬─────────────────┬────────────────────┤
│     Engine      │     Runtime     │      Workers       │
│  (Scheduling)   │   (Triggers)    │   (Execution)      │
└─────────────────┴─────────────────┴────────────────────┘
                            │
┌─────────────────────────────────────────────────────────┐
│                     Core Layer                           │
├─────────────────┬─────────────────┬────────────────────┤
│     Value       │     Memory      │    Expression      │
│    (Types)      │   (Caching)     │   (Evaluation)     │
└─────────────────┴─────────────────┴────────────────────┘
                            │
┌─────────────────────────────────────────────────────────┐
│                   Storage Layer                          │
├─────────────────┬─────────────────┬────────────────────┤
│   PostgreSQL    │  Object Store   │   Message Bus      │
│   (Metadata)    │   (Binary)      │    (Kafka)         │
└─────────────────┴─────────────────┴────────────────────┘
```

### Поток данных

1. **Workflow Definition** → API → Engine → Storage
2. **Trigger Event** → Runtime → Kafka → Engine
3. **Execution** → Worker → Node → Storage → Next Worker
4. **Results** → Storage → API → UI

### Компоненты системы

#### Core Components
- **nebula-core**: Базовые trait'ы и типы
- **serde / serde_json::Value**: Значения и сериализация (crate nebula-value не используется)
- **nebula-memory**: In-memory состояние и кеширование

#### Execution Components
- **nebula-engine**: Orchestration и scheduling
- **nebula-runtime**: Управление triggers
- **nebula-worker**: Выполнение nodes

#### Storage Components
- **nebula-storage**: Абстракции хранилища
- **nebula-binary**: Управление бинарными данными

#### Developer Components
- **nebula-sdk**: All-in-one SDK
- **nebula-derive**: Procedural macros
- **nebula-node-registry**: Управление nodes

---

