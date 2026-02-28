# Archived From "docs/archive/final.md"

### nebula-cli
**Назначение:** Command-line interface для управления Nebula.

```bash
# Workflow management
nebula workflow deploy my-workflow.yaml
nebula workflow execute user-registration --input data.json
nebula workflow list --filter "status=active"

# Execution monitoring  
nebula execution watch exec-123
nebula execution logs exec-123 --follow
nebula execution cancel exec-123

# Action development
nebula action create --template simple
nebula action test my-action --input test.json
nebula action publish my-action

# Cluster management
nebula cluster status
nebula cluster add-node node-4
nebula cluster rebalance
```

---

