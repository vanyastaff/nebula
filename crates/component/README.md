# nebula-component

The one contract every integration component implements.

An action, a resource and a credential are all the same thing to the catalog:
a Rust type with a key, a configuration schema and metadata to show. This crate
states that once, as the `Component` trait, so the three kinds cannot drift
apart in names or in rules.

```rust
use nebula_component::{Component, key_of, kind};
use nebula_metadata::{Metadata, metadata_name};

struct SlackBotToken;

impl Component for SlackBotToken {
    type Kind = kind::Credential;
    type Config = ();

    const KEY: &'static str = "slack_bot_token";

    fn metadata() -> Metadata {
        Metadata::new(metadata_name!("Slack bot token"), "Bot token for the Slack API")
    }
}

let key: nebula_core::CredentialKey = key_of::<SlackBotToken>();
assert_eq!(key.as_str(), "slack_bot_token");

let definition = nebula_component::admit::<SlackBotToken>()?;
assert_eq!(definition.key(), &key);
# Ok::<(), nebula_metadata::MetadataBuildError>(())
```

## What is here

| Item | What it is |
|---|---|
| `Component` | `Kind`, `Config`, `KEY`, `metadata()`: what every component declares |
| `kind::{Action, Resource, Credential}` | The closed set of kinds, as types |
| `key_of::<C>()` | The typed key (`ActionKey`, `ResourceKey`, `CredentialKey`), checked at compile time |
| `admit::<C>()` | The shared admission: metadata, key and `Config` schema into a `BaseDefinition` |

## Rules

- The key is written once, as `Component::KEY`. Metadata cannot hold a key and
  cannot hold a schema; admission supplies both from the type.
- `Component::Kind` is an associated type, so a type has exactly one kind and
  generic code can choose behavior per kind at compile time.
- The kind traits (`Action`, the resource `Provider`, `Credential`) have `Component` as a
  supertrait and add only what that kind does. They live in their own crates.

## Status

`Action`, `Credential` and the resource `Provider` are built on `Component`.
