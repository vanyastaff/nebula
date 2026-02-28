# Archived From "docs/archive/nebula-complete.md"

#### 1. nebula-core (Week 1)
- [ ] 1.1 **Project Setup**
  - [ ] 1.1.1 Initialize crate structure
  - [ ] 1.1.2 Setup CI/CD pipeline
  - [ ] 1.1.3 Configure linting and formatting
  - [ ] 1.1.4 Add base dependencies

- [ ] 1.2 **Identifier Types**
  - [ ] 1.2.1 Implement WorkflowId with validation
  - [ ] 1.2.2 Implement NodeId with string normalization
  - [ ] 1.2.3 Implement ExecutionId with UUID
  - [ ] 1.2.4 Implement TriggerId
  - [ ] 1.2.5 Add Display and Debug traits
  - [ ] 1.2.6 Add serialization support
  - [ ] 1.2.7 Write property-based tests

- [ ] 1.3 **Error Handling**
  - [ ] 1.3.1 Design Error enum hierarchy
  - [ ] 1.3.2 Implement error contexts
  - [ ] 1.3.3 Add error conversion traits
  - [ ] 1.3.4 Create Result type alias
  - [ ] 1.3.5 Add error chaining support
  - [ ] 1.3.6 Write error documentation

- [ ] 1.4 **Core Traits**
  - [ ] 1.4.1 Define Action trait
  - [ ] 1.4.2 Define TriggerAction trait
  - [ ] 1.4.3 Define PollingAction trait
  - [ ] 1.4.4 Define SupplyAction trait
  - [ ] 1.4.5 Define ProcessAction trait
  - [ ] 1.4.6 Add async trait support
  - [ ] 1.4.7 Write trait documentation

- [ ] 1.5 **Metadata Types**
  - [ ] 1.5.1 Implement ActionMetadata
  - [ ] 1.5.2 Implement NodeMetadata
  - [ ] 1.5.3 Implement WorkflowMetadata
  - [ ] 1.5.4 Implement ParameterDescriptor
  - [ ] 1.5.5 Add builder patterns
  - [ ] 1.5.6 Add validation logic

#### 2. Value layer: serde / serde_json::Value (Week 1-2)
Отдельный crate nebula-value не используется.
- [ ] 2.1 Использовать `serde_json::Value` для данных workflow, serde для сериализации
- [ ] 2.2 Валидация поверх Value (nebula-validator / core)

---

# Nebula Complete Roadmap

## 🎯 Master Roadmap

### Phase 1: Core Foundation (Weeks 1-3)

