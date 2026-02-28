# Archived From "docs/archive/node-development.md"

## Parameter Types

### Text Parameters
```rust
#[param(
    type = "text",
    label = "API Key",
    placeholder = "Enter your API key",
    validation = "min_length:10"
)]
api_key: String,
```

### Number Parameters
```rust
#[param(
    type = "number",
    label = "Timeout",
    min = 1,
    max = 300,
    default = 30
)]
timeout_seconds: u32,
```

### Select Parameters
```rust
#[param(
    type = "select",
    label = "Region",
    options = ["us-east-1", "eu-west-1", "ap-south-1"],
    default = "us-east-1"
)]
region: String,
```

