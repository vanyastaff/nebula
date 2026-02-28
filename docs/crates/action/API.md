# API

## Stable surface (current)

- `Action`
- `ActionMetadata`
- `ActionComponents`
- `Context`
- `ActionError`
- `ActionResult<T>`
- `ActionOutput<T>`
- `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`

## Minimal action skeleton

```rust
use nebula_action::{Action, ActionComponents, ActionMetadata};

struct MyAction {
    meta: ActionMetadata,
}

impl Action for MyAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    fn components(&self) -> ActionComponents {
        ActionComponents::new()
    }
}
```

## Metadata and ports (contract-first)

```rust
use nebula_action::{ActionMetadata, InputPort, OutputPort};

let meta = ActionMetadata::new("http.request", "HTTP Request", "Calls external API")
    .with_inputs(vec![InputPort::flow("in")])
    .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);
```

Rules:
- key is globally unique per action type (`namespace.name` style recommended)
- default ports are acceptable, but explicit port declaration is preferred for stable contracts

## Dependency declaration (resources + credentials)

```rust
use nebula_action::ActionComponents;
use nebula_credential::CredentialRef;
use nebula_resource::ResourceRef;

struct ApiToken;
struct HttpClient;

let components = ActionComponents::new()
    .credential(CredentialRef::of::<ApiToken>())
    .resource(ResourceRef::of::<HttpClient>());
```

## Execution result and output forms

```rust
use nebula_action::{ActionResult, ActionOutput};

let ok = ActionResult::success_output(ActionOutput::Value(42));
let wait = ActionResult::Wait {
    condition: nebula_action::WaitCondition::Duration {
        duration: std::time::Duration::from_secs(30),
    },
    timeout: Some(std::time::Duration::from_secs(300)),
    partial_output: None,
};
```

Guidelines:
- use `ActionResult::Retry` for intentional reschedule signals
- use `ActionError::Retryable` for transient failures
- use `ActionError::Fatal`/`Validation` for hard stops

## Error helpers

```rust
use nebula_action::ActionError;

let retry = ActionError::retryable_with_backoff("rate limited", std::time::Duration::from_secs(5));
let fatal = ActionError::fatal("invalid schema");
```

## Production authoring rules

1. Keep metadata and ports backward compatible inside one major interface version.
2. Declare all external dependencies in `ActionComponents`.
3. Return explicit flow intent with `ActionResult`; avoid out-of-band control channels.
4. Ensure output size/type is predictable for downstream compatibility.
5. Distinguish retryable and fatal errors consistently.
