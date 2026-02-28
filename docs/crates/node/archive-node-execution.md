# Archived From "docs/archive/node-execution.md"

### nebula-node
**Назначение:** Группировка связанных Actions и Credentials в логические узлы для удобной организации и discovery.

**Ключевые концепции:**
- Node как пакет связанной функциональности
- Версионирование на уровне Node
- Метаданные для UI discovery

```rust
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub version: semver::Version,
    pub actions: Vec<ActionDefinition>,
    pub credentials: Vec<CredentialDefinition>,
    pub metadata: NodeMetadata,
}

// Пример: Node для работы со Slack
let slack_node = Node {
    id: NodeId::new("slack"),
    name: "Slack Integration",
    version: Version::new(2, 1, 0),
    actions: vec![
        ActionDefinition::new("slack.send_message"),
        ActionDefinition::new("slack.create_channel"),
        ActionDefinition::new("slack.upload_file"),
    ],
    credentials: vec![
        CredentialDefinition::new("slack_token", CredentialType::Bearer),
        CredentialDefinition::new("slack_webhook", CredentialType::Webhook),
    ],
};
```

---

