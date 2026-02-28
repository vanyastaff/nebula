# Archived From "docs/archive/final.md"

### nebula-hub
**Назначение:** Marketplace для sharing Actions и Workflows.

```rust
pub struct Hub {
    registry: PackageRegistry,
    storage: PackageStorage,
}

// Publishing
nebula hub publish my-actions-pack v1.0.0
nebula hub install slack-integration

// Package format
pub struct Package {
    pub name: String,
    pub version: Version,
    pub actions: Vec<ActionDefinition>,
    pub workflows: Vec<WorkflowTemplate>,
    pub dependencies: Vec<Dependency>,
}
```

---

## Полный пример использования

```rust
// 1. Создаем Action
#[derive(Action)]
#[action(id = "weather.fetch")]
pub struct WeatherAction;

impl SimpleAction for WeatherAction {
    type Input = WeatherInput;
    type Output = WeatherData;
    
    async fn execute_simple(&self, input: Self::Input, ctx: &ActionContext) -> Result<Self::Output> {
        let api_key = ctx.get_credential("weather_api").await?;
        let client = WeatherClient::new(api_key);
        Ok(client.get_weather(&input.city).await?)
    }
}

// 2. Создаем Workflow
let weather_workflow = WorkflowBuilder::new("weather-notification")
    .add_node("fetch", "weather.fetch")
    .add_node("check", "condition.check")
    .add_node("notify", "notification.send")
    .connect("fetch", "check")
    .connect_conditional("check", "notify", "$nodes.fetch.result.temp > 30")
    .build();

// 3. Деплоим и запускаем
let engine = WorkflowEngine::new(config);
engine.deploy_workflow(weather_workflow).await?;

let execution = engine.execute_workflow(
    "weather-notification",
    json!({ "city": "Moscow" }),
    ExecutionOptions::default(),
).await?;

// 4. Мониторим выполнение
execution.on_complete(|result| {
    println!("Workflow completed: {:?}", result);
});
```

