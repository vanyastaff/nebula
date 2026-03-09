# Industry Reference — Form Builder Systems Analysis

Comprehensive analysis of property/parameter systems across 18 professional platforms.

---

## Overview

This reference documents how major platforms implement parameter/property systems for automatic UI generation. Each system has unique strengths that informed paramdef's design.

---

## 3D Graphics & Game Engines

### Blender RNA (Python/C)

**Key Innovations:**
- Semantic kind + Unit pattern (semantic meaning + measurement)
- Soft/Hard min/max (UI hints vs validation enforcement)
- Poll functions for conditional visibility

```python
bpy.props.FloatProperty(
    name="Distance",
    semantic_kind='DISTANCE',     # Semantic meaning
    unit='LENGTH',          # Measurement system
    soft_min=0.0,           # Slider starts at 0
    soft_max=10.0,          # Slider ends at 10
    min=0.0,                # Hard minimum (enforced)
    max=100.0,              # Hard maximum (enforced)
    default=1.0
)
```

**paramdef Adoption:** Semantic kind + Unit pattern, Soft/Hard constraints

---

### Unreal Engine UPROPERTY (C++)

**Key Innovations:**
- Rich metadata system
- Category/subcategory organization
- Edit conditions

```cpp
UPROPERTY(
    EditAnywhere,
    BlueprintReadWrite,
    Category = "Transform|Position",
    meta = (
        EditCondition = "bIsEnabled",
        ClampMin = "0.0",
        ClampMax = "100.0"
    )
)
float Speed;
```

**paramdef Adoption:** Metadata flags, Display conditions

---

### Unity Inspector (C#)

**Key Innovations:**
- Attribute-based decoration
- Property drawers
- Serialization control

```csharp
[Header("Movement")]
[Range(0f, 100f)]
[Tooltip("Movement speed")]
[SerializeField]
private float speed = 10f;

[ShowIf("enableAdvanced")]
[ReadOnly]
public string computedValue;
```

**paramdef Adoption:** Attribute-based flags, Conditional visibility

---

### Godot @export (GDScript)

**Key Innovations:**
- Type hints with annotations
- Range hints
- Export categories

```gdscript
@export var speed: float = 10.0
@export_range(0, 100, 0.1) var health: float
@export_enum("Walk", "Run", "Jump") var mode: int
@export_group("Advanced")
@export var internal_value: int
```

**paramdef Adoption:** Range hints, Group organization

---

## Node-Based Editors

### Houdini Parameters (Python/HDK)

**Key Innovations:**
- Parameter templates
- Conditional disable/hide
- Multiparms (dynamic arrays)
- Parameter pages/folders

```python
hou.FloatParmTemplate(
    name="scale",
    label="Scale",
    num_components=3,
    default_value=(1.0, 1.0, 1.0),
    min=0.0, max=10.0,
    look=hou.parmLook.Vector,
    naming_scheme=hou.parmNamingScheme.XYZW,
    tags={"disable_when": "{ method != advanced }"}
)
```

**paramdef Adoption:** Pages/groups, Display conditions, Vector parameters

---

### TouchDesigner (Python)

**Key Innovations:**
- Parameter modes (constant, expression, export, bound)
- Pulse buttons (action triggers)
- Internal parameters (ipar)

```python
par = page.appendFloat('scale', label='Scale')
par[0].default = 1.0
par[0].min = 0.0
par[0].max = 10.0
par[0].mode = ParMode.EXPRESSION
par[0].expr = 'me.time.seconds'
```

**paramdef Adoption:** Expression support

---

### ComfyUI INPUT_TYPES (Python)

**Key Innovations:**
- Simple declarative format
- Required vs optional separation
- Lazy evaluation

```python
@classmethod
def INPUT_TYPES(cls):
    return {
        "required": {
            "model": ("MODEL",),
            "steps": ("INT", {"default": 20, "min": 1, "max": 100}),
        },
        "optional": {
            "seed": ("INT", {"default": 0}),
        }
    }
```

**paramdef Adoption:** Required/optional separation

---

## Workflow Automation

### n8n INodeProperties (TypeScript)

**Key Innovations:**
- displayOptions for conditional fields
- Resource/operation pattern
- Credential types

```typescript
{
    displayName: 'Email',
    name: 'email',
    type: 'string',
    required: true,
    displayOptions: {
        show: {
            resource: ['user'],
            operation: ['create'],
        },
    },
    placeholder: 'user@example.com',
}
```

**paramdef Adoption:** Display conditions, Resource pattern

---

### Apache Airflow (Python)

**Key Innovations:**
- Jinja templating for dynamic values
- Param validation with JSON Schema
- Operator inheritance

```python
class MyOperator(BaseOperator):
    template_fields = ('sql', 'params')
    
    def __init__(self, sql: str, params: dict = None, **kwargs):
        super().__init__(**kwargs)
        self.sql = sql
        self.params = params
```

**paramdef Adoption:** Expression/template support

---

### Prefect (Python)

**Key Innovations:**
- Pydantic-based config validation
- Type hints for automatic UI
- Task/Flow hierarchy

```python
class MyConfig(Config):
    api_endpoint: str = Field(
        description="API endpoint URL",
        default="https://api.example.com"
    )
    batch_size: int = Field(default=100, ge=1, le=10000)
```

**paramdef Adoption:** Pydantic-style validation

---

### Apache NiFi (Java)

**Key Innovations:**
- PropertyDescriptor builder pattern
- Standard validators
- Expression language support
- Sensitive property handling

```java
PropertyDescriptor.Builder()
    .name("password")
    .displayName("Password")
    .sensitive(true)
    .required(true)
    .addValidator(StandardValidators.NON_EMPTY_VALIDATOR)
    .expressionLanguageSupported(ExpressionLanguageScope.FLOWFILE_ATTRIBUTES)
    .build()
```

**paramdef Adoption:** Sensitive flags, Standard validators

---

### Temporal (Multi-language)

**Key Innovations:**
- Durable execution
- Activity/Workflow separation
- Heartbeat for long operations

```python
@workflow.defn
class DataWorkflow:
    @workflow.run
    async def run(self, params: WorkflowParams) -> dict:
        result = await workflow.execute_activity(
            process_data,
            params,
            start_to_close_timeout=timedelta(minutes=5),
        )
        return result
```

**paramdef Adoption:** Schema/Context separation (similar to Activity/Workflow)

---

### Dagster (Python)

**Key Innovations:**
- Pydantic Config classes
- PermissiveConfig for dynamic fields
- Resource configuration

```python
class MyOpConfig(Config):
    mode: ProcessingMode = Field(default=ProcessingMode.BALANCED)
    timeout_seconds: Optional[int] = Field(default=None, ge=0)

@op
def my_op(context: OpExecutionContext, config: MyOpConfig):
    pass
```

**paramdef Adoption:** Config class pattern

---

## Application Frameworks

### Qt Q_PROPERTY (C++)

**Key Innovations:**
- Meta-object system
- RESET functionality
- Property flags (STORED, DESIGNABLE, SCRIPTABLE)
- Qt 6 bindable properties

```cpp
Q_PROPERTY(int count 
    READ count 
    WRITE setCount 
    RESET resetCount
    NOTIFY countChanged
    STORED true
    DESIGNABLE true
)
```

**paramdef Adoption:** Reset functionality, Storage flags, Change notifications

---

### WPF DependencyProperty (C#)

**Key Innovations:**
- Property system with precedence
- Coerce value callback
- Validation callback
- Attached properties

```csharp
public static readonly DependencyProperty ValueProperty =
    DependencyProperty.Register(
        "Value",
        typeof(double),
        typeof(RangeControl),
        new PropertyMetadata(
            50.0,
            OnValueChanged,
            CoerceValue
        )
    );
```

**paramdef Adoption:** Value transformers (coercion), Validation callbacks

---

### Node-RED (JavaScript)

**Key Innovations:**
- HTML edit templates
- Built-in validators
- Config node references

```javascript
RED.nodes.registerType('my-node', {
    defaults: {
        value: {
            value: 0,
            required: true,
            validate: RED.validators.number()
        },
        server: {
            value: "",
            type: "mqtt-broker"  // Config node reference
        }
    }
});
```

**paramdef Adoption:** Config references, Built-in validators

---

## Pattern Summary

### Universal Patterns (Found in 10+ Systems)

| Pattern | Systems | paramdef |
|---------|---------|----------|
| Required/Optional | All | ✅ `REQUIRED` flag |
| Default values | All | ✅ `.default()` |
| Min/Max constraints | All | ✅ `.range()` |
| Labels/Descriptions | All | ✅ Metadata |
| Change notifications | Qt, WPF, Unreal | ✅ EventBus |
| Conditional visibility | All node editors | ✅ Expr |
| Type validation | All | ✅ Type-safe keys |

### Advanced Patterns (Found in 5-9 Systems)

| Pattern | Systems | paramdef |
|---------|---------|----------|
| Semantic-kind semantics | Blender, Unreal, Houdini | ✅ Semantic kinds |
| Unit conversion | Blender, Unreal | ✅ Units |
| Soft/Hard limits | Blender, Houdini | ✅ Soft/Hard |
| Expression support | Airflow, n8n, Houdini | ✅ Expression flag |
| Reset to default | Qt, Houdini | ✅ Reset |
| Sensitive data | NiFi, n8n | ✅ SENSITIVE flag |

### Specialized Patterns (Found in 2-4 Systems)

| Pattern | Systems | paramdef |
|---------|---------|----------|
| Value coercion | WPF, Qt | ✅ Transformers |
| Discriminated unions | n8n (resources) | ✅ Mode |
| Storage control | Qt (STORED) | ✅ SKIP_SAVE flag |
| Animation support | Blender, Unreal | ✅ ANIMATABLE flag |

---

## paramdef's Unique Contributions

### 1. Compile-Time Type Safety

```rust
// Type-safe builders
Text::builder("username").required().min_length(3).build();
Number::builder("age").range(0, 150).build();
Vector::builder::<f64, 3>("position").default([0.0, 0.0, 0.0]).build();

// Type-safe getters
let name: &str = context.get_string("username")?;
let age: i64 = context.get_int("age")?;
```

Type safety achieved through builders and typed getters, not stringly-typed access.

### 2. Zero-Cost Abstractions

```rust
// SmartString - stack allocation for small strings
// Arc sharing - single schema copy
// Const generics - stack-allocated vectors
// No runtime overhead for safety
```

### 3. Rust Safety Guarantees

- Thread-safe by default
- No null pointer exceptions
- Ownership prevents data races
- Result types for error handling

### 4. Unified Best Practices

paramdef combines patterns from 18 different systems into a cohesive API:

- Blender's semantic kind + Unit
- Qt's Reset + Notifications
- WPF's Coercion (Transformers)
- n8n's Display Conditions
- NiFi's Sensitive Handling
- Houdini's Organization (Pages/Groups)
- And more...

---

## Feature Matrix

| Feature | Blender | Unreal | Unity | Godot | Qt | WPF | n8n | Airflow | NiFi | Houdini | **paramdef** |
|---------|---------|--------|-------|-------|----|----|-----|---------|------|---------|------------|
| Type Safety | - | ~ | ~ | - | ~ | ✓ | - | - | - | - | **✓** |
| Compile-Time | - | ~ | - | - | ~ | ~ | - | - | - | - | **✓** |
| SemanticKind+Unit | ✓ | ✓ | - | - | - | - | - | - | - | ~ | **✓** |
| Soft/Hard | ✓ | ✓ | - | - | - | ~ | - | - | - | ✓ | **✓** |
| Transformers | - | - | - | - | - | ✓ | - | - | - | - | **✓** |
| Reset | - | - | - | - | ✓ | - | - | - | - | ✓ | **✓** |
| Conditions | ~ | ✓ | ~ | - | - | - | ✓ | - | ✓ | ✓ | **✓** |
| Expressions | ✓ | ~ | - | - | ~ | ~ | ✓ | ✓ | ✓ | ✓ | **✓** |
| Events | - | ✓ | - | ✓ | ✓ | ✓ | - | - | - | ~ | **✓** |
| Flags | ~ | ✓ | ~ | - | ✓ | ~ | - | - | ✓ | ~ | **✓** |
| Pages/Groups | - | ✓ | - | ✓ | - | - | ~ | - | - | ✓ | **✓** |
| Actions | - | - | - | - | - | - | - | - | - | ✓ | **✓** |
| i18n | - | ~ | - | - | - | - | - | - | - | - | **✓** |
| Zero-Cost | - | ~ | - | - | ~ | - | - | - | - | - | **✓** |

**Legend:** ✓ = Full support, ~ = Partial, - = Not available

---

## Recommendations from Industry

### For Form Builders
1. Use semantic kind to encode semantic meaning
2. Provide soft limits for better UX
3. Support conditional visibility
4. Include reset functionality

### For Workflow Systems
1. Support expression/template values
2. Handle sensitive data properly
3. Validate both sync and async
4. Provide meaningful error messages

### For 3D Editors
1. Use Unit system for measurements
2. Support animation (keyframes)
3. Provide real-time updates
4. Include undo/redo

### For All Systems
1. Separate schema from runtime state
2. Use type-safe property access
3. Implement change notifications
4. Support serialization control

