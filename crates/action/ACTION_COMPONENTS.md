# ActionComponents

`ActionComponents` - это тип для декларирования зависимостей action на credentials и resources в крейте `nebula-action`.

## Использование

### Создание компонентов

```rust
use nebula_action::ActionComponents;
use nebula_credential::CredentialRef;
use nebula_resource::ResourceRef;

// Определяем типы credentials
struct GithubToken;
struct SlackWebhook;

// Определяем типы resources
struct PostgresDb;
struct RedisCache;

// Создаём компоненты с builder pattern
let components = ActionComponents::new()
    .credential(CredentialRef::of::<GithubToken>())
    .credential(CredentialRef::of::<SlackWebhook>())
    .resource(ResourceRef::of::<PostgresDb>())
    .resource(ResourceRef::of::<RedisCache>());
```

### Batch-методы

```rust
// Добавление нескольких зависимостей за раз
let components = ActionComponents::new()
    .with_credentials(vec![
        CredentialRef::of::<GithubToken>(),
        CredentialRef::of::<SlackWebhook>(),
    ])
    .with_resources(vec![
        ResourceRef::of::<PostgresDb>(),
        ResourceRef::of::<RedisCache>(),
    ]);
```

### Доступ к зависимостям

```rust
// Получение списка credentials
let creds = components.credentials();

// Получение списка resources
let resources = components.resources();

// Проверка наличия зависимостей
if components.is_empty() {
    println!("No dependencies declared");
}

// Подсчёт общего количества зависимостей
let total = components.len();
```

### Деструктуризация

```rust
// Разделение на составляющие
let (creds, resources) = components.into_parts();
```

## API

### Методы создания

- `new()` - создать пустую коллекцию
- `default()` - то же, что и `new()`

### Методы добавления зависимостей

- `credential(cred: CredentialRef) -> Self` - добавить credential
- `resource(res: ResourceRef) -> Self` - добавить resource
- `with_credentials(creds: Vec<CredentialRef>) -> Self` - добавить несколько credentials
- `with_resources(resources: Vec<ResourceRef>) -> Self` - добавить несколько resources

### Методы доступа

- `credentials() -> &[CredentialRef]` - получить список credentials
- `resources() -> &[ResourceRef]` - получить список resources
- `is_empty() -> bool` - проверить отсутствие зависимостей
- `len() -> usize` - получить общее количество зависимостей
- `into_parts() -> (Vec<CredentialRef>, Vec<ResourceRef>)` - разделить на составляющие

## Пример

Полный пример использования находится в `crates/action/examples/action_components.rs`:

```bash
cargo run -p nebula-action --example action_components
```

## Связанные типы

- `CredentialRef` из `nebula-credential` - type-safe ссылка на credential
- `ResourceRef` из `nebula-resource` - type-safe ссылка на resource
