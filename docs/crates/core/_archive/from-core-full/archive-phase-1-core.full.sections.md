#### nebula-core (Days 1-3)
- **Day 1**: Setup и базовая структура
  - [ ] Инициализация crate
  - [ ] Настройка CI/CD
  - [ ] Базовые зависимости
  - [ ] Структура модулей

- **Day 2**: Identifier types
  - [ ] WorkflowId implementation
  - [ ] NodeId implementation  
  - [ ] ExecutionId implementation
  - [ ] TriggerId implementation
  - [ ] Tests для ID types

- **Day 3**: Error handling
  - [ ] Error enum design
  - [ ] Error contexts
  - [ ] Error conversion traits
  - [ ] Result type alias

#### Value layer: serde / serde_json::Value (Days 4-5)
- **Day 4**: Единый тип значений
  - [ ] Использовать `serde_json::Value` для данных workflow
  - [ ] Интеграция с параметрами и контекстом

- **Day 5**: Сериализация и валидация
  - [ ] Serde для всех структур
  - [ ] Валидация поверх Value (где нужно)

### Week 1 Checklist
- [ ] CI/CD работает
- [ ] Все ID types готовы
- [ ] Error handling complete
- [ ] serde_json::Value в контуре данных
- [ ] Serialization тесты проходят

### Week 2: Advanced Types and Memory

#### Доп. валидация и типы (Days 6-8)
- **Day 6-8**: Валидаторы и конвертация
  - [ ] Валидаторы поверх serde_json::Value (nebula-validator / core)
  - [ ] Извлечение типизированных полей где нужно
  - [ ] Интеграция с expression/parameter


---

#### nebula-core (Days 11-12)
- **Day 11**: Action traits
  - [ ] Action trait finalization
  - [ ] TriggerAction trait
  - [ ] SupplyAction trait
  - [ ] Trait composition tests

- **Day 12**: Metadata types
  - [ ] ActionMetadata
  - [ ] NodeMetadata
  - [ ] WorkflowMetadata
  - [ ] ParameterDescriptor
