# Parameter Flags Reference

Comprehensive guide to `Flags` bitflags for controlling parameter behavior.

---

## Overview

`Flags` is a bitflag type that replaces multiple boolean fields with a single efficient representation.

### Benefits

| Aspect | Old Approach | Flags |
|--------|--------------|----------------|
| **Storage** | N bytes (N bools) | 4 bytes (u32) |
| **Extensibility** | Add new fields | Add new bits |
| **Combinations** | Manual | Built-in presets |
| **Type Safety** | Weak | Strong (bitflags!) |

---

## Flag Definitions

```rust
bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Flags: u32 {
        // === Core Behavior ===
        const REQUIRED      = 1 << 0;   // Must have a value
        const READONLY      = 1 << 1;   // Cannot be modified by user
        const HIDDEN        = 1 << 2;   // Not shown in UI
        const DISABLED      = 1 << 3;   // Shown but not interactive
        
        // === Security ===
        const SENSITIVE     = 1 << 4;   // Mask in logs, careful handling
        const WRITE_ONLY    = 1 << 5;   // Cannot read back (passwords)
        
        // === Lifecycle ===
        const DEPRECATED    = 1 << 6;   // Show warning, will be removed
        const EXPERIMENTAL  = 1 << 7;   // Unstable, may change
        const INTERNAL      = 1 << 8;   // For internal use only
        
        // === Animation & Realtime ===
        const ANIMATABLE    = 1 << 9;   // Can be keyframed
        const REALTIME      = 1 << 10;  // Updates in realtime (no debounce)
        
        // === Expressions ===
        const EXPRESSION    = 1 << 11;  // Supports {{ expression }} syntax
        const TEMPLATABLE   = 1 << 12;  // Supports template variables
        
        // === Storage ===
        const SKIP_SAVE     = 1 << 13;  // Don't persist to storage
        const RUNTIME       = 1 << 14;  // Computed at runtime
        const CONSTANT      = 1 << 15;  // Value never changes
        
        // === Network ===
        const REPLICATED    = 1 << 16;  // Sync over network
        
        // === UI Hints ===
        const ADVANCED      = 1 << 17;  // Show in advanced section
        const COMPACT       = 1 << 18;  // Use compact UI representation
    }
}
```

---

## Flag Categories

### Core Behavior Flags

| Flag | Description | UI Effect |
|------|-------------|-----------|
| `REQUIRED` | Value must be provided | Shows required indicator (*) |
| `READONLY` | User cannot modify | Disabled input, no edit |
| `HIDDEN` | Not visible | Not rendered |
| `DISABLED` | Visible but not interactive | Grayed out |

```rust
// Required email field
TextParameter::email("email")
    .required()
    .build()

// Read-only computed value
TextParameter::builder("full_name")
    .readonly()
    .build()

// Hidden internal parameter
TextParameter::builder("session_id")
    .hidden()
    .build()
```

### Security Flags

| Flag | Description | Behavior |
|------|-------------|----------|
| `SENSITIVE` | Contains sensitive data | Masked in logs, careful in errors |
| `WRITE_ONLY` | Cannot read value back | Password fields, API keys |

```rust
// Password field (masked, write-only)
TextParameter::password("password")
    .flags(Flags::REQUIRED | Flags::SENSITIVE | Flags::WRITE_ONLY)
    .build()

// API key (sensitive, not saved)
TextParameter::api_key("api_token")
    .flags(Flags::SENSITIVE | Flags::WRITE_ONLY | Flags::SKIP_SAVE)
    .build()
```

### Lifecycle Flags

| Flag | Description | UI Effect |
|------|-------------|-----------|
| `DEPRECATED` | Will be removed | Shows warning icon/message |
| `EXPERIMENTAL` | May change | Shows experimental badge |
| `INTERNAL` | Not for end users | Hidden from normal UI |

```rust
// Deprecated parameter (shows warning)
TextParameter::builder("old_api_url")
    .deprecated()
    .description("Use 'api_url' instead")
    .build()

// Experimental feature
NumberParameter::builder::<f64>("new_algorithm_threshold")
    .experimental()
    .build()
```

### Animation & Realtime Flags

| Flag | Description | Use Case |
|------|-------------|----------|
| `ANIMATABLE` | Can be keyframed | 3D editors, motion graphics |
| `REALTIME` | Updates immediately | Sliders, color pickers |

```rust
// Animatable position (3D editor)
NumberParameter::builder::<f64>("position_x")
    .animatable()
    .realtime()
    .build()

// Realtime preview updates
VectorParameter::<f64, 3>::color_rgb("preview_color")
    .realtime()
    .build()
```

### Expression Flags

| Flag | Description | Syntax |
|------|-------------|--------|
| `EXPRESSION` | Supports expressions | `{{ $json.value }}` |
| `TEMPLATABLE` | Supports templates | `Hello {{ name }}!` |

```rust
// Workflow node input with expression support
TextParameter::builder("input")
    .expression()
    .placeholder("{{ $json.data }}")
    .build()

// Email template
TextParameter::builder("email_body")
    .templatable()
    .placeholder("Dear {{ customer.name }},")
    .build()
```

### Storage Flags

| Flag | Description | Effect |
|------|-------------|--------|
| `SKIP_SAVE` | Don't persist | Not saved to file/DB |
| `RUNTIME` | Computed value | Calculated, not stored |
| `CONSTANT` | Never changes | Immutable after creation |

```rust
// Computed display value (not saved)
TextParameter::builder("display_name")
    .runtime()
    .readonly()
    .skip_save()
    .build()

// Cache (transient)
TextParameter::builder("cached_result")
    .skip_save()
    .build()

// Constant ID
TextParameter::builder("node_id")
    .constant()
    .build()
```

### Network Flags

| Flag | Description | Use Case |
|------|-------------|----------|
| `REPLICATED` | Sync across network | Multiplayer, distributed systems |

```rust
// Synced player position
NumberParameter::builder::<f64>("player_x")
    .replicated()
    .realtime()
    .build()
```

---

## Preset Combinations

Common flag combinations as convenience methods:

```rust
impl Flags {
    /// Password field: REQUIRED | SENSITIVE | WRITE_ONLY
    pub fn password_field() -> Self {
        Self::REQUIRED | Self::SENSITIVE | Self::WRITE_ONLY
    }
    
    /// Sensitive data: SENSITIVE | WRITE_ONLY | SKIP_SAVE
    pub fn sensitive() -> Self {
        Self::SENSITIVE | Self::WRITE_ONLY | Self::SKIP_SAVE
    }
    
    /// Computed value: RUNTIME | READONLY | SKIP_SAVE
    pub fn computed() -> Self {
        Self::RUNTIME | Self::READONLY | Self::SKIP_SAVE
    }
    
    /// Internal parameter: HIDDEN | SKIP_SAVE
    pub fn internal() -> Self {
        Self::HIDDEN | Self::SKIP_SAVE
    }
    
    /// Animation property: ANIMATABLE | REALTIME | REPLICATED
    pub fn animation() -> Self {
        Self::ANIMATABLE | Self::REALTIME | Self::REPLICATED
    }
}
```

**Usage:**
```rust
// Using preset
TextParameter::password("password")
    .flags(Flags::password_field())
    .build()

// Or convenience constructor (same result)
TextParameter::password("password").build()
```

---

## Convenience Methods

### Checking Flags

```rust
impl Flags {
    pub fn is_required(&self) -> bool {
        self.contains(Self::REQUIRED)
    }
    
    pub fn is_readonly(&self) -> bool {
        self.contains(Self::READONLY)
    }
    
    pub fn is_hidden(&self) -> bool {
        self.contains(Self::HIDDEN)
    }
    
    pub fn is_visible(&self) -> bool {
        !self.contains(Self::HIDDEN)
    }
    
    pub fn is_editable(&self) -> bool {
        !self.contains(Self::READONLY) && !self.contains(Self::DISABLED)
    }
    
    pub fn is_sensitive(&self) -> bool {
        self.contains(Self::SENSITIVE)
    }
    
    pub fn should_save(&self) -> bool {
        !self.contains(Self::SKIP_SAVE) && !self.contains(Self::RUNTIME)
    }
    
    pub fn can_display(&self) -> bool {
        !self.contains(Self::HIDDEN) && !self.contains(Self::INTERNAL)
    }
}
```

**Usage:**
```rust
let param = TextParameter::password("password").build();

if param.flags().is_sensitive() {
    println!("Handle with care!");
}

if param.flags().should_save() {
    save_to_storage(&param);
}

if param.is_editable() {
    render_input(&param);
} else {
    render_readonly(&param);
}
```

---

## Builder Methods

### Individual Flag Methods

```rust
impl TextParameterBuilder {
    pub fn required(mut self) -> Self {
        self.flags |= Flags::REQUIRED;
        self
    }
    
    pub fn optional(mut self) -> Self {
        self.flags &= !Flags::REQUIRED;
        self
    }
    
    pub fn readonly(mut self) -> Self {
        self.flags |= Flags::READONLY;
        self
    }
    
    pub fn hidden(mut self) -> Self {
        self.flags |= Flags::HIDDEN;
        self
    }
    
    pub fn disabled(mut self) -> Self {
        self.flags |= Flags::DISABLED;
        self
    }
    
    pub fn sensitive(mut self) -> Self {
        self.flags |= Flags::SENSITIVE;
        self
    }
    
    pub fn deprecated(mut self) -> Self {
        self.flags |= Flags::DEPRECATED;
        self
    }
    
    pub fn experimental(mut self) -> Self {
        self.flags |= Flags::EXPERIMENTAL;
        self
    }
    
    pub fn animatable(mut self) -> Self {
        self.flags |= Flags::ANIMATABLE;
        self
    }
    
    pub fn realtime(mut self) -> Self {
        self.flags |= Flags::REALTIME;
        self
    }
    
    pub fn expression(mut self) -> Self {
        self.flags |= Flags::EXPRESSION;
        self
    }
    
    pub fn skip_save(mut self) -> Self {
        self.flags |= Flags::SKIP_SAVE;
        self
    }
    
    pub fn runtime(mut self) -> Self {
        self.flags |= Flags::RUNTIME;
        self
    }
    
    pub fn replicated(mut self) -> Self {
        self.flags |= Flags::REPLICATED;
        self
    }
}
```

### Direct Flag Setting

```rust
// Set flags directly
TextParameter::builder("custom")
    .flags(Flags::REQUIRED | Flags::EXPRESSION)
    .build()

// Add flags to existing
TextParameter::builder("combined")
    .required()
    .with_flags(Flags::EXPRESSION | Flags::REALTIME)
    .build()
```

---

## Serialization

Flags serialize as u32 for compact storage:

```rust
#[derive(Serialize, Deserialize)]
struct ParameterData {
    key: String,
    flags: u32,  // Flags serialized as bits
}

// Serialize
let flags = Flags::REQUIRED | Flags::SENSITIVE;
let bits: u32 = flags.bits();  // 0b10001 = 17

// Deserialize
let flags = Flags::from_bits(bits).unwrap_or_default();
```

---

## Use Cases by Domain

### Form Builder

```rust
// Username: required, unique validation
TextParameter::builder("username")
    .required()
    .build()

// Password: required, sensitive, write-only
TextParameter::password("password")
    .required()
    .build()

// Remember me: optional
BoolParameter::builder("remember")
    .build()

// Hidden CSRF token
TextParameter::builder("csrf_token")
    .hidden()
    .required()
    .build()
```

### 3D Editor

```rust
// Animatable transform
NumberParameter::builder::<f64>("position_x")
    .animatable()
    .realtime()
    .replicated()
    .build()

// Computed world matrix (not saved)
TextParameter::builder("world_matrix")
    .runtime()
    .readonly()
    .skip_save()
    .build()
```

### Workflow Automation (n8n-style)

```rust
// Input with expression support
TextParameter::builder("url")
    .required()
    .expression()
    .placeholder("{{ $json.api_url }}")
    .build()

// API key (sensitive)
TextParameter::api_key("api_key")
    .required()
    .build()

// Output (runtime computed)
TextParameter::builder("response")
    .runtime()
    .readonly()
    .build()
```

### Game Development

```rust
// Player health (replicated, realtime)
NumberParameter::builder::<f64>("health")
    .realtime()
    .replicated()
    .range(0.0, 100.0)
    .build()

// Debug mode (internal)
BoolParameter::builder("debug_mode")
    .flags(Flags::internal())
    .build()

// Deprecated old setting
NumberParameter::builder::<f64>("old_sensitivity")
    .deprecated()
    .description("Use 'mouse_sensitivity' instead")
    .build()
```

---

## Migration Guide

### From Boolean Fields

**Before:**
```rust
struct TextParameter {
    is_required: bool,
    is_readonly: bool,
    is_hidden: bool,
    is_sensitive: bool,
    // ... more bools
}

let param = TextParameter {
    is_required: true,
    is_readonly: false,
    is_hidden: false,
    is_sensitive: true,
};
```

**After:**
```rust
struct TextParameter {
    flags: Flags,
}

let param = TextParameter::builder("key")
    .required()
    .sensitive()
    .build();

// Or with flags directly
let param = TextParameter::builder("key")
    .flags(Flags::REQUIRED | Flags::SENSITIVE)
    .build();
```

---

## Summary

### Quick Reference Table

| Flag | Bit | Method | Effect |
|------|-----|--------|--------|
| `REQUIRED` | 0 | `.required()` | Must have value |
| `READONLY` | 1 | `.readonly()` | Cannot edit |
| `HIDDEN` | 2 | `.hidden()` | Not visible |
| `DISABLED` | 3 | `.disabled()` | Grayed out |
| `SENSITIVE` | 4 | `.sensitive()` | Masked in logs |
| `WRITE_ONLY` | 5 | `.write_only()` | Cannot read back |
| `DEPRECATED` | 6 | `.deprecated()` | Shows warning |
| `EXPERIMENTAL` | 7 | `.experimental()` | Unstable feature |
| `INTERNAL` | 8 | - | Internal use only |
| `ANIMATABLE` | 9 | `.animatable()` | Can keyframe |
| `REALTIME` | 10 | `.realtime()` | Instant updates |
| `EXPRESSION` | 11 | `.expression()` | Supports `{{ }}` |
| `TEMPLATABLE` | 12 | `.templatable()` | Template vars |
| `SKIP_SAVE` | 13 | `.skip_save()` | Not persisted |
| `RUNTIME` | 14 | `.runtime()` | Computed value |
| `CONSTANT` | 15 | `.constant()` | Never changes |
| `REPLICATED` | 16 | `.replicated()` | Network sync |
| `ADVANCED` | 17 | `.advanced()` | Advanced section |
| `COMPACT` | 18 | `.compact()` | Compact UI |

### Presets

| Preset | Flags | Use Case |
|--------|-------|----------|
| `password_field()` | REQUIRED \| SENSITIVE \| WRITE_ONLY | Passwords |
| `sensitive()` | SENSITIVE \| WRITE_ONLY \| SKIP_SAVE | API keys |
| `computed()` | RUNTIME \| READONLY \| SKIP_SAVE | Calculated values |
| `internal()` | HIDDEN \| SKIP_SAVE | Internal data |
| `animation()` | ANIMATABLE \| REALTIME \| REPLICATED | 3D properties |
