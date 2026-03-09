# Parameter & Property System Deep Dive
### Technical reference across 18 platforms — schema design, type encoding, constraint propagation, UI generation

---

## Table of Contents

1. [Core Problem Space](#core-problem-space)
2. [Blender RNA — C/Python hybrid](#blender-rna)
3. [Unreal UPROPERTY — compile-time metadata](#unreal-uproperty)
4. [Unity Inspector — attribute-driven reflection](#unity-inspector)
5. [Godot @export — annotation pipeline](#godot-export)
6. [Houdini HDK — parametric templates](#houdini-hdk)
7. [TouchDesigner — mode-switched evaluation](#touchdesigner)
8. [ComfyUI — minimal declarative format](#comfyui)
9. [n8n INodeProperties — discriminated visibility](#n8n)
10. [Apache NiFi — PropertyDescriptor builder](#apache-nifi)
11. [Airflow — Jinja + Pydantic hybrid](#airflow)
12. [Prefect — Pydantic-first config](#prefect)
13. [Dagster — Config class pattern](#dagster)
14. [Qt Q_PROPERTY — meta-object system](#qt-qproperty)
15. [WPF DependencyProperty — coercion pipeline](#wpf-dependencyproperty)
16. [Node-RED — JSON schema + validators](#node-red)
17. [Cross-Cutting Patterns](#cross-cutting-patterns)
18. [Constraint System Taxonomy](#constraint-system-taxonomy)
19. [Conditional Visibility Strategies](#conditional-visibility-strategies)
20. [Type System Approaches](#type-system-approaches)
21. [Implications for Rust Implementation](#implications-for-rust-implementation)

---

## Core Problem Space

Every parameter system solves the same fundamental problem: **bridge between typed runtime values and a rendered UI**, while encoding constraints, visibility rules, and semantic metadata in a way that survives serialization, versioning, and cross-language boundaries.

The key tension is between **schema** (what a parameter _is_) and **state** (what a parameter _currently holds_). Systems that conflate these two tend to produce brittle UIs or make versioning painful.

```
Schema: name, type, constraints, display metadata, visibility rules
State:  current value, dirty flag, expression string, override mode
```

A secondary tension is **static vs dynamic**: most systems want static schema for performance and type safety, but need dynamic behavior for conditional visibility and computed defaults.

---

## Blender RNA

**Language:** C (core) + Python (scripting layer)  
**Key file:** `source/blender/makesrna/intern/rna_define.c`

### Architecture

RNA (RNA is Not Attributes) is Blender's reflection system. Every property is registered into a `StructRNA` at startup, building a runtime type registry. The Python layer (`bpy.props.*`) is a thin wrapper over this C API.

```c
// C side — actual registration
PropertyRNA *RNA_def_float(StructOrFunctionRNA *cont, const char *identifier,
                           float default_value, float hardmin, float hardmax,
                           const char *ui_name, const char *ui_description,
                           float softmin, float softmax);

// What gets stored internally per property:
typedef struct FloatPropertyRNA {
    PropertyRNA property;   // base — name, type, flag, description
    PropFloatGetFunc get;   // optional getter override
    PropFloatSetFunc set;   // optional setter override
    float defaultvalue;
    float softmin, softmax; // UI range (slider clamps)
    float hardmin, hardmax; // validation range (enforced)
    int totarraylength;
    float step;             // increment step
    int precision;          // decimal places shown
} FloatPropertyRNA;
```

### Soft vs Hard Constraints

This is Blender's most influential innovation. The distinction:

- **`soft_min/soft_max`**: The slider widget clamps to this range. The user _can_ type outside it.
- **`min/max`** (hard): Enforced by the RNA system itself. Values outside this range are rejected or clamped before storage.

```python
# Python API
bpy.props.FloatProperty(
    name="Radius",
    subtype='DISTANCE',
    unit='LENGTH',
    soft_min=0.001,   # slider range
    soft_max=100.0,   #
    min=0.0,          # hard floor — negative radius nonsensical
    max=1e6,          # hard ceiling — prevents float overflow downstream
    default=1.0,
    step=1,           # UI increment = step/100 = 0.01
    precision=3,
)
```

The design rationale: `soft` bounds encode UX intent ("this is the _normal_ range"), `hard` bounds encode domain invariants ("this value _cannot physically_ be negative"). The UI never shows the hard bounds — they exist purely for validation.

### Subtype + Unit System

Subtypes encode **semantic meaning** beyond the base type:

```python
# Same underlying float, very different UI widget + behavior
FloatProperty(subtype='NONE')        # plain number field
FloatProperty(subtype='DISTANCE')    # unit-aware, converts m/cm/inch
FloatProperty(subtype='ANGLE')       # stores radians, displays degrees
FloatProperty(subtype='FACTOR')      # always 0..1, shown as percentage
FloatProperty(subtype='PIXEL')       # integer-like, no decimals
FloatProperty(subtype='UNSIGNED')    # non-negative enforcement in UI
FloatProperty(subtype='DIRECTION')   # normalized automatically
FloatProperty(subtype='COLOR')       # color picker widget
FloatProperty(subtype='COLOR_GAMMA') # same + gamma correction display
```

Unit system is **orthogonal** to subtype:

```python
FloatProperty(subtype='DISTANCE', unit='LENGTH')
# unit='LENGTH' → respects scene unit scale (metric/imperial)
# unit='NONE'   → raw value, no conversion
# unit='TIME'   → shows as frames or seconds depending on FPS setting
```

This means the _stored value_ is always in Blender's internal unit (meters, radians, etc.) but the _displayed value_ is automatically converted based on scene settings. Localisation for free.

### Property Flags

```c
// PropertyFlag — bitmask on every PropertyRNA
PROP_EDITABLE          // can be modified (runtime toggle)
PROP_ANIMATABLE        // can have keyframes
PROP_LIBRARY_EDITABLE  // editable in linked libraries
PROP_HIDDEN            // not shown in UI
PROP_SKIP_SAVE         // not serialized to .blend file
PROP_OUTPUT            // output-only parameter
PROP_REQUIRED          // must be set (for function params)
PROP_NEVER_NULL        // pointer cannot be None
PROP_ENUM_NO_CONTEXT   // enum items don't depend on context
```

### Poll Functions

Conditional visibility implemented as C function pointers on property groups:

```python
# Python equivalent of RNA poll
class MyPanel(bpy.types.Panel):
    @classmethod
    def poll(cls, context):
        return context.object and context.object.type == 'MESH'
```

At the RNA level this is a `StructRNA.refine` function that gates access to the struct entirely, or `EnumPropertyRNA.item_fn` for dynamic enum items.

---

## Unreal UPROPERTY

**Language:** C++ with UHT (Unreal Header Tool) preprocessing  
**Key concept:** `UPROPERTY` is a marker processed by UHT at compile time, not a runtime attribute

### Processing Pipeline

```
.h file with UPROPERTY macros
         ↓
    UHT (Unreal Header Tool)
         ↓
    Generated .generated.h
         ↓
    Compiled with reflection data baked in
         ↓
    UClass runtime registry
```

The generated code contains static `FProperty*` objects for each annotated field. These live in the class's `UClass` object and are the basis for the editor, serialization, GC, and replication systems.

### Specifier Categories

```cpp
// Access control specifiers
UPROPERTY(VisibleAnywhere)        // read-only in all panels
UPROPERTY(EditAnywhere)           // editable in all panels  
UPROPERTY(VisibleDefaultsOnly)    // read-only in Blueprint defaults
UPROPERTY(EditDefaultsOnly)       // editable in Blueprint defaults only
UPROPERTY(VisibleInstanceOnly)    // read-only for instances only
UPROPERTY(EditInstanceOnly)       // editable for instances only

// Blueprint exposure
UPROPERTY(BlueprintReadOnly)
UPROPERTY(BlueprintReadWrite)
UPROPERTY(BlueprintSetter = "MySetterFunction")
UPROPERTY(BlueprintGetter = "MyGetterFunction")

// Replication (networking)
UPROPERTY(Replicated)
UPROPERTY(ReplicatedUsing = OnRep_MyVar)  // callback on client when value changes

// Serialization
UPROPERTY(Transient)       // not serialized
UPROPERTY(SaveGame)        // serialized to save games (separate from level data)
UPROPERTY(NonPIEDuplicateTransient)  // cleared when duplicating for PIE

// Organization
UPROPERTY(Category = "Movement|Ground")  // pipe-delimited hierarchy
UPROPERTY(DisplayName = "Max Speed (cm/s)")
UPROPERTY(AdvancedDisplay)  // hidden behind "advanced" toggle
```

### Meta Specifiers

`meta=(...)` is a freeform key-value dictionary processed by both UHT and the editor:

```cpp
UPROPERTY(EditAnywhere, meta = (
    // Clamping
    ClampMin = "0.0",
    ClampMax = "1000.0",
    UIMin = "0.0",          // slider range (soft min)
    UIMax = "100.0",        // slider range (soft max)
    
    // Conditional editing  
    EditCondition = "bIsEnabled",                    // simple bool gate
    EditCondition = "Mode == EMode::Advanced",       // enum comparison
    EditConditionHides = true,                       // hide vs just grey out
    
    // Display
    Units = "cm",
    ForceUnits = "cm",      // disallow unit switching
    DisplayAfter = "OtherProperty",
    NoResetToDefault = true,
    
    // Validation
    MustImplement = "MyInterface",  // for UObject* properties
    AllowedClasses = "StaticMesh,SkeletalMesh",
    
    // Inline editing
    ShowOnlyInnerProperties,  // expand struct inline
    FullyExpand = true,
    
    // Numeric
    Delta = "0.1",          // drag increment
    LinearDeltaSensitivity = "1.0",
    Multiple = "5.0"        // must be multiple of this
))
float Speed;
```

### EditCondition Internals

`EditCondition` is evaluated by `FPropertyEditorPermissionList` at editor render time. The expression language is deliberately simple:

- Single boolean property: `"bEnabled"`
- Enum comparison: `"Mode == EMyEnum::Value"`  
- Negation: `"!bEnabled"`
- AND (since UE5.1): `"bEnabled && bAdvanced"`

More complex conditions require a custom `FPropertyCustomization` in editor modules.

---

## Unity Inspector

**Language:** C# with reflection  
**Key innovation:** Attribute-based decoration — properties describe themselves

### Attribute Architecture

Unity's Inspector reads `[Attribute]` decorations via C# reflection at editor time. No code generation, no preprocessing — pure runtime reflection.

```csharp
// Built-in Unity attributes
[Header("Section Title")]           // visual separator + label
[Space(10)]                         // vertical spacing
[Tooltip("Shown on hover")]         // tooltip
[Range(min, max)]                   // slider widget
[Min(0)]                            // minimum only (no slider)
[Multiline(5)]                      // textarea for string
[TextArea(3, 10)]                   // textarea with min/max lines
[HideInInspector]                   // serialize but don't show
[SerializeField]                    // serialize private field
[NonSerialized]                     // don't serialize public field
[FormerlySerializedAs("oldName")]   // migration: map old name to new field

// From popular Odin Inspector (de facto standard in production)
[ShowIf("condition")]               // conditional visibility
[HideIf("condition")]
[EnableIf("condition")]             // conditional interactivity
[DisableIf("condition")]
[ShowIf("@this.value > 5")]        // expression syntax
[ValidateInput("Validate")]         // custom validator method
[OnValueChanged("OnChanged")]       // change callback
[PropertyOrder(5)]                  // explicit ordering
[TabGroup("Tab Name")]
[FoldoutGroup("Group")]
[BoxGroup("Box")]
[Button("Label")]                   // action button (no backing field)
[ReadOnly]
[Required]
```

### Property Drawer System

Unity's extensibility point is `PropertyDrawer` — a class that overrides how a specific type or attribute is rendered:

```csharp
// Define custom attribute
public class PercentageAttribute : PropertyAttribute { }

// Draw it
[CustomPropertyDrawer(typeof(PercentageAttribute))]
public class PercentageDrawer : PropertyDrawer {
    public override void OnGUI(Rect position, SerializedProperty property, GUIContent label) {
        property.floatValue = EditorGUI.Slider(position, label, 
            property.floatValue, 0f, 1f);
        // Override display: show as "37%" instead of "0.37"
        var pct = position;
        EditorGUI.LabelField(pct, $"{property.floatValue * 100:F0}%");
    }
}
```

### SerializedProperty Internals

Unity's `SerializedProperty` is a cursor into the serialized C# object tree. Key behaviors:

- Operates on the _serialized form_, not the live object (allows undo/redo)
- Supports `SerializedObject.ApplyModifiedProperties()` to flush changes
- Handles prefab override marking automatically
- Type-erased: access via `floatValue`, `intValue`, `stringValue`, `objectReferenceValue` etc.

The serialization format is YAML-based internally but is the same system that backs .prefab and .asset files, so inspector changes are the same as file changes.

---

## Godot @export

**Language:** GDScript / C# / C++ GDExtension  
**Key innovation:** Annotation pipeline directly on typed variables

### Annotation Types

```gdscript
# Basic type inference
@export var speed: float = 10.0          # FloatProperty
@export var name: String = "Player"      # StringProperty
@export var texture: Texture2D           # ResourceProperty with picker

# Range (maps to Blender-style soft bounds)
@export_range(0.0, 100.0) var health: float
@export_range(0.0, 100.0, 0.5) var health: float          # with step
@export_range(0.0, 100.0, 0.5, "or_less") var health: float  # allow below min
@export_range(0.0, 100.0, 0.5, "or_greater") var health: float

# Enum (auto-generates dropdown from enum type)
@export var mode: MovementMode
@export_enum("Walk", "Run", "Fly") var mode: int  # anonymous enum

# File/path
@export_file("*.png", "*.jpg") var icon_path: String
@export_dir var save_dir: String
@export_global_file var config_path: String

# Organization
@export_group("Movement")              # collapsible section
@export_subgroup("Ground Movement")   # nested section
@export_category("Physics")           # tab in inspector

# Node references
@export var target: Node               # any node
@export var mesh: MeshInstance3D       # typed node reference

# Array
@export var items: Array[Weapon]       # typed array with add/remove UI

# Storage control
@export_storage var internal_value: int  # serialize but less visible
```

### C++ GDExtension

For C++ plugins, annotations map to `ClassDB::bind_property`:

```cpp
ClassDB::bind_property(
    PropertyInfo(Variant::FLOAT, "speed", 
        PROPERTY_HINT_RANGE, "0,200,0.1,or_greater"),
    "set_speed", "get_speed"
);

// PropertyInfo fields:
// type: Variant::Type
// name: StringName
// hint: PropertyHint enum (RANGE, ENUM, FILE, etc.)
// hint_string: hint-specific format ("min,max,step,flags")
// usage: PropertyUsageFlags bitmask
```

The `hint_string` format is hint-specific and poorly documented — it's a comma-separated string whose meaning depends on `hint`. For `RANGE`: `"min,max"` or `"min,max,step"` or `"min,max,step,or_less,or_greater,degrees,radians,hide_slider,exp"`.

---

## Houdini HDK

**Language:** C++ (HDK) + Python (pdg / hom)  
**Key innovation:** Parameter templates as first-class data, Multiparms for dynamic arrays

### ParmTemplate Architecture

```cpp
// Houdini's ParmTemplate is a value type (copyable, serializable)
hou::FloatParmTemplate(
    "scale",              // internal name (token)
    "Scale",              // UI label
    3,                    // num_components (1=scalar, 2=vec2, 3=vec3, 4=vec4)
    {1.0f, 1.0f, 1.0f},  // default values
    0.0f,                 // min
    10.0f,                // max
    true,                 // min_is_strict (hard min)
    true,                 // max_is_strict (hard max)
    hou::parmLook::Vector,
    hou::parmNamingScheme::XYZW  // names each component: scalex, scaley, scalez
);
```

### Conditional Expressions (disable_when / hide_when)

Houdini uses a string-based expression language for visibility:

```python
# Disable when another param has a specific value
tags={"disable_when": "{ method != advanced }"}

# Multiple conditions (OR semantics by default)
tags={"disable_when": "{ method == simple } { method == basic }"}

# AND requires separate tokens
tags={"disable_when": "{ method == advanced quality == low }"}
# "disable when (method==advanced AND quality==low)"

# Hide entirely (vs greyed out)
tags={"hide_when": "{ show_advanced != 1 }"}
```

This expression language is interpreted at draw time by the parameter dialog renderer. It's distinct from HScript expressions (which compute values) — it only checks equality/inequality and logical combinations.

### Multiparms — Dynamic Arrays

Unique to Houdini: a `FolderParmTemplate` of type `MultiparmBlock` creates a dynamically-sized list of parameter groups. The count is itself a parameter.

```python
multi = hou.FolderParmTemplate(
    "layers",
    "Layers",
    folder_type=hou.folderType.MultiparmBlock
)
multi.addParmTemplate(hou.FloatParmTemplate("opacity#", "Opacity #", 1))
multi.addParmTemplate(hou.StringParmTemplate("name#", "Name #", 1))
# '#' expands to the instance index at runtime: opacity1, opacity2, ...
```

The `#` token in parameter names expands to the multiparm index. This creates parameter names like `opacity1`, `opacity2`, ... at runtime, making dynamic arrays addressable by name.

### Parameter Pages/Folders

```python
parm_template_group = hou.ParmTemplateGroup()

basic_folder = hou.FolderParmTemplate("basic", "Basic", 
    folder_type=hou.folderType.Tabs)
basic_folder.addParmTemplate(size_parm)

advanced_folder = hou.FolderParmTemplate("advanced", "Advanced",
    folder_type=hou.folderType.Collapsible)
advanced_folder.addParmTemplate(detail_parm)

parm_template_group.addParmTemplate(basic_folder)
parm_template_group.addParmTemplate(advanced_folder)
```

Folder types: `Tabs`, `Collapsible`, `Simple` (no UI frame), `RadioButtons`, `MultiparmBlock`, `ScrollingMultiparmBlock`, `TabbedMultiparmBlock`.

---

## TouchDesigner

**Language:** Python  
**Key innovation:** Parameter _modes_ — each parameter can be in different evaluation modes simultaneously

### Parameter Mode System

```python
# ParMode enum
ParMode.CONSTANT   # raw stored value — no evaluation
ParMode.EXPRESSION # live Python expression — evaluated every cook
ParMode.EXPORT     # driven by another operator's output — CHOP/DAT binding
ParMode.BIND       # two-way binding to another parameter

# Same parameter, switched to expression mode
op('geo1').par.tx.mode = ParMode.EXPRESSION
op('geo1').par.tx.expr = 'op("wave1").chan("chan1")[0] * 10'

# Bind mode — bidirectional sync
op('slider1').par.value0.bindExpr = "op('other_slider').par.value0"
op('slider1').par.value0.mode = ParMode.BIND
```

This is fundamentally different from other systems: the parameter's _value_ and _mode_ are separate concerns. A UI slider shows `value` when in CONSTANT mode, shows the expression string when in EXPRESSION mode, and shows the binding source when in EXPORT/BIND mode.

### Pulse Parameters

TouchDesigner has a parameter type with no persistent state — it fires once when clicked:

```python
# Pulse — no value, just an action trigger
page.appendPulse('reset', label='Reset')
# In execute callback:
def onParValueChange(par, val, prev):
    if par.name == 'reset':
        do_reset()
```

Pulses are critical for "action" semantics in node-based systems: they don't represent _state_, they represent _events_. Most systems fake this with buttons bound to callbacks; TouchDesigner makes it a first-class parameter type.

---

## ComfyUI

**Language:** Python  
**Key innovation:** Minimal declarative format, `required` vs `optional` separation at the API level

### INPUT_TYPES Protocol

```python
@classmethod
def INPUT_TYPES(cls):
    return {
        "required": {
            # (TYPE_STRING, {options_dict})
            "model": ("MODEL",),                         # no options = opaque handle
            "clip":  ("CLIP",),
            "steps": ("INT",    {"default": 20, "min": 1, "max": 100, "step": 1}),
            "cfg":   ("FLOAT",  {"default": 7.0, "min": 0.0, "max": 30.0, "round": 0.01}),
            "sampler_name": (comfy.samplers.KSampler.SAMPLERS,),  # enum from list
            "scheduler":    (comfy.samplers.KSampler.SCHEDULERS,),
            "positive": ("CONDITIONING",),
            "negative": ("CONDITIONING",),
            "latent_image": ("LATENT",),
            "denoise": ("FLOAT", {"default": 1.0, "min": 0.0, "max": 1.0, "step": 0.01}),
        },
        "optional": {
            "seed": ("INT", {"default": 0, "min": 0, "max": 0xffffffffffffffff}),
        },
        "hidden": {
            "unique_id": "UNIQUE_ID",     # injected by runtime
            "prompt": "PROMPT",           # full graph JSON
            "extra_pnginfo": "EXTRA_PNGINFO",
        }
    }
```

The type string determines the socket color and type-checking for connections. Uppercase custom types (`MODEL`, `CLIP`, `LATENT`) are opaque handles — no UI widget, only connectable ports. Lowercase built-ins (`INT`, `FLOAT`, `STRING`, `BOOLEAN`) generate widgets when unconnected.

### Lazy Evaluation

`"optional"` inputs aren't evaluated unless the node actually uses them. The executor supports lazy evaluation via `INPUT_IS_LIST` flag and lazy tensors — critical for performance with large models.

---

## n8n INodeProperties

**Language:** TypeScript  
**Key innovation:** `displayOptions` for cascading conditional visibility based on other field values

### Full Property Schema

```typescript
interface INodePropertyOptions {
    name: string;          // display label
    value: string | number;
    description?: string;
    action?: string;       // "setKeyValue" etc
}

interface INodeProperties {
    displayName: string;
    name: string;          // internal key
    type: NodePropertyTypes;
    // NodePropertyTypes = 'boolean' | 'collection' | 'color' | 'dateTime'
    //   | 'fixedCollection' | 'hidden' | 'json' | 'notice' | 'number'
    //   | 'options' | 'string' | 'resourceLocator' | 'credentialsSelect'
    //   | 'filter' | 'assignmentCollection'
    
    default: any;
    required?: boolean;
    
    description?: string;
    hint?: string;          // shown below field, not on hover
    placeholder?: string;
    
    // Options for type='options' (dropdown)
    options?: Array<INodePropertyOptions | INodeProperties>;
    
    // Constraints
    typeOptions?: {
        minValue?: number;
        maxValue?: number;
        numberPrecision?: number;
        multipleValues?: boolean;   // array input
        multipleValueButtonText?: string;
        loadOptionsMethod?: string; // method to call for dynamic options
        loadOptionsDependsOn?: string[]; // re-load when these change
        password?: boolean;        // mask value
        rows?: number;             // textarea height
        editor?: 'code' | 'json' | 'sqlEditor' | 'cssEditor';
        editorLanguage?: string;
        alwaysOpenEditWindow?: boolean;
    };
    
    // Conditional display — THIS is the key innovation
    displayOptions?: {
        show?: {
            [key: string]: Array<string | number | boolean>;
            // "show this field when fieldX is one of these values"
        };
        hide?: {
            [key: string]: Array<string | number | boolean>;
        };
    };
    
    // Nesting
    options?: INodeProperties[];   // for type='collection' | 'fixedCollection'
    
    noDataExpression?: boolean;    // disable expression mode for this field
    validateType?: string;         // built-in validators: 'url', 'email'
}
```

### Resource/Operation Pattern

n8n's convention for CRUD-style nodes: a `resource` field selects the entity, an `operation` field selects the action, and all other fields use `displayOptions.show` to appear only for relevant combinations:

```typescript
[
    {
        displayName: 'Resource',
        name: 'resource',
        type: 'options',
        options: [
            { name: 'User', value: 'user' },
            { name: 'Post', value: 'post' },
        ],
        default: 'user',
    },
    {
        displayName: 'Operation',
        name: 'operation',
        type: 'options',
        displayOptions: { show: { resource: ['user'] } },
        options: [
            { name: 'Create', value: 'create' },
            { name: 'Get', value: 'get' },
        ],
        default: 'create',
    },
    {
        displayName: 'Email',
        name: 'email',
        type: 'string',
        displayOptions: {
            show: {
                resource: ['user'],
                operation: ['create'],  // AND semantics across keys
            },
        },
        required: true,
    },
]
```

`displayOptions` is evaluated purely by value equality — no expressions. The `show` object has AND semantics across keys (all conditions must match), but OR semantics within each array (any value in the array matches).

---

## Apache NiFi

**Language:** Java  
**Key innovation:** `PropertyDescriptor` builder pattern, built-in validator library, expression language

### PropertyDescriptor Builder

```java
// Full API
static final PropertyDescriptor MY_PROPERTY = new PropertyDescriptor.Builder()
    .name("my-property")                  // internal ID (stable across renames)
    .displayName("My Property")           // shown in UI
    .description("Detailed description")
    .required(true)
    .sensitive(false)                     // if true: masked, encrypted at rest
    .defaultValue("default")
    .allowableValues("opt1", "opt2")      // enum constraint
    .allowableValues(MyEnum.class)        // from Java enum
    .addValidator(StandardValidators.NON_EMPTY_VALIDATOR)
    .addValidator(StandardValidators.URI_VALIDATOR)
    .addValidator(StandardValidators.createAttributeExpressionLanguageValidator(
        AttributeExpression.ResultType.STRING))
    .expressionLanguageSupported(
        ExpressionLanguageScope.FLOWFILE_ATTRIBUTES)  // can use ${attr}
    .dynamic(false)                       // if true: user-defined property
    .dependsOn(OTHER_PROPERTY, "value1")  // conditional — show only when
    .identifiesControllerService(SSLContextService.class)  // typed ref
    .build();
```

### StandardValidators Library

```java
// Built-in validators (composable with addValidator calls)
StandardValidators.NON_EMPTY_VALIDATOR
StandardValidators.NON_BLANK_VALIDATOR
StandardValidators.INTEGER_VALIDATOR
StandardValidators.POSITIVE_INTEGER_VALIDATOR
StandardValidators.POSITIVE_LONG_VALIDATOR
StandardValidators.NON_NEGATIVE_INTEGER_VALIDATOR
StandardValidators.LONG_VALIDATOR
StandardValidators.NUMBER_VALIDATOR
StandardValidators.PORT_VALIDATOR          // 1-65535
StandardValidators.DATA_SIZE_VALIDATOR     // "10 MB", "1 GB"
StandardValidators.TIME_PERIOD_VALIDATOR   // "10 sec", "5 min"
StandardValidators.URI_VALIDATOR
StandardValidators.URL_VALIDATOR
StandardValidators.FILE_EXISTS_VALIDATOR
StandardValidators.READABLE_FILE_VALIDATOR
StandardValidators.WRITEABLE_DIRECTORY_VALIDATOR
StandardValidators.REGULAR_EXPRESSION_VALIDATOR
StandardValidators.BOOLEAN_VALIDATOR
StandardValidators.createRegexValidator(pattern)
StandardValidators.createDirectoryExistsValidator(create, readPermission, writePermission)
```

### Expression Language

NiFi's EL evaluates against FlowFile attributes at runtime:

```
${filename}                             # attribute access
${filename:substringAfter('_')}        # string ops
${file.size:greaterThan(1024)}         # predicate
${literal(5):multiply(${count})}       # arithmetic
${ip:matches('192\\.168\\..*')}        # regex
```

`expressionLanguageSupported(ExpressionLanguageScope.FLOWFILE_ATTRIBUTES)` marks a property as EL-capable. The scope enum controls what's available: `NONE`, `VARIABLE_REGISTRY`, `FLOWFILE_ATTRIBUTES`, `ENVIRONMENT`.

---

## Airflow

**Language:** Python  
**Key innovation:** `template_fields` for Jinja templating, Pydantic-style `Param` for DAG-level config

### template_fields Protocol

```python
class MyOperator(BaseOperator):
    # Declare which fields support Jinja templating
    template_fields: Sequence[str] = ('sql', 'params', 'output_key')
    template_fields_renderers = {
        'sql': 'sql',     # syntax highlighting hint for UI
    }
    template_ext: Sequence[str] = ('.sql',)  # auto-load from file if string ends with this
    
    def __init__(self, sql: str, params: dict | None = None, **kwargs):
        super().__init__(**kwargs)
        self.sql = sql      # can contain {{ ds }}, {{ execution_date }}, etc.
        self.params = params
    
    def execute(self, context: Context):
        # By the time execute() is called, template_fields have been rendered
        # self.sql is now the rendered string (Jinja substitutions applied)
        self.log.info("Running: %s", self.sql)
```

### DAG-level Params (Pydantic-backed)

```python
from airflow.sdk import Param

with DAG(
    "my_dag",
    params={
        "batch_size": Param(
            default=100,
            type="integer",
            minimum=1,
            maximum=10000,
            description="Number of rows per batch",
            title="Batch Size",
        ),
        "target_env": Param(
            default="staging",
            enum=["dev", "staging", "prod"],
            description="Target environment",
        ),
        "start_date": Param(
            type="string",
            format="date",                    # JSON Schema format
            description="ISO 8601 date",
        ),
        "config": Param(
            type="object",
            title="Advanced Config",
            description="JSON configuration blob",
        ),
    }
):
    ...
```

`Param` uses JSON Schema under the hood. The `type`, `minimum`, `maximum`, `enum`, `format` keys are passed directly to a JSON Schema validator. The UI (Trigger DAG w/ config) generates form fields from this schema.

---

## Prefect

**Language:** Python  
**Key innovation:** Pydantic `BaseModel` subclass as `Config` — full Pydantic validation + UI generation

### Config Class Pattern

```python
from prefect.utilities.pydantic import get_dispatch_key
from pydantic import BaseModel, Field, validator, field_validator
from typing import Literal, Optional, Union, Annotated

class DatabaseConfig(Config):
    host: str = Field(default="localhost", description="Database hostname")
    port: int = Field(default=5432, ge=1, le=65535, description="Port number")
    database: str
    username: str
    password: SecretStr                    # masked in UI and logs
    ssl_mode: Literal["disable", "require", "verify-full"] = "require"
    
    @field_validator("host")
    @classmethod
    def validate_host(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("Host cannot be empty")
        return v.strip()
    
    # Discriminated union for polymorphic config
    auth: Union[
        Annotated[ApiKeyAuth, Field(discriminator="type")],
        Annotated[OAuthConfig, Field(discriminator="type")],
    ] = Field(discriminator="type")

class MyFlow(Config):
    db: DatabaseConfig
    batch_size: int = Field(default=100, ge=1)
    mode: Literal["fast", "safe"] = "safe"

@flow
def process(config: MyFlow):
    ...
```

### PermissiveConfig

```python
class DynamicConfig(PermissiveConfig):
    # Allows arbitrary extra fields (Pydantic's extra="allow")
    # Useful for pass-through config to external tools
    known_field: str = "default"
    # Any extra fields are accepted and accessible via .model_extra
```

### Block System (Credential Storage)

```python
from prefect.blocks.core import Block

class MyCredentials(Block):
    _block_type_name = "My Service Credentials"
    _logo_url = "https://..."
    
    api_key: SecretStr = Field(description="API key")
    base_url: str = Field(default="https://api.example.com")
    
    async def get_client(self) -> httpx.AsyncClient:
        return httpx.AsyncClient(
            base_url=self.base_url,
            headers={"Authorization": f"Bearer {self.api_key.get_secret_value()}"}
        )

# Usage
creds = await MyCredentials.load("production-creds")
```

Blocks are stored in Prefect's backend, versioned, and referenceable by name. The schema is auto-generated from the Pydantic model for the UI form.

---

## Dagster

**Language:** Python  
**Key innovation:** Typed `Config` classes, `ConfigurableResource` for DI, clean separation of schema from execution context

### Config Class System

```python
import dagster as dg
from dagster import Config, OpExecutionContext
from pydantic import Field
from enum import Enum
from typing import Optional

class ProcessingMode(str, Enum):
    FAST = "fast"
    BALANCED = "balanced" 
    THOROUGH = "thorough"

class MyOpConfig(Config):
    mode: ProcessingMode = Field(
        default=ProcessingMode.BALANCED,
        description="Processing strategy"
    )
    batch_size: int = Field(
        default=100,
        ge=1,
        le=10000,
        description="Items per batch"
    )
    timeout_seconds: Optional[int] = Field(
        default=None,
        ge=0,
        description="Operation timeout (None = unlimited)"
    )
    output_prefix: str = "result_"

@dg.op
def my_op(context: OpExecutionContext, config: MyOpConfig) -> None:
    context.log.info(f"Mode: {config.mode}, Batch: {config.batch_size}")
```

### ConfigurableResource

```python
from dagster import ConfigurableResource, resource
import httpx

class APIClient(ConfigurableResource):
    base_url: str = Field(description="API base URL")
    api_key: str = Field(description="API key")
    timeout: float = Field(default=30.0, ge=0)
    
    def get_client(self) -> httpx.Client:
        return httpx.Client(
            base_url=self.base_url,
            headers={"X-API-Key": self.api_key},
            timeout=self.timeout,
        )
    
    def setup_for_execution(self, context) -> None:
        # Called before any op uses this resource
        self._client = self.get_client()
    
    def teardown_after_execution(self, context) -> None:
        self._client.close()

# Binding at job level
defs = dg.Definitions(
    jobs=[my_job],
    resources={
        "api_client": APIClient(
            base_url="https://api.prod.example.com",
            api_key=EnvVar("API_KEY"),          # late-bound env var
        )
    }
)
```

### Env Vars and Late Binding

```python
# EnvVar — resolved at execution time, not at import time
api_key: str = EnvVar("MY_API_KEY")

# StringSource — either literal or env var
url: str = StringSource  # accepts "literal_value" OR {"env": "URL_VAR"}
```

---

## Qt Q_PROPERTY

**Language:** C++  
**Key innovation:** Meta-object system (MOC), property precedence, coerce value callback

### Full Q_PROPERTY Syntax

```cpp
Q_PROPERTY(type name
    READ getter                     // required
    WRITE setter                    // makes it writable
    RESET reset_fn                  // restores to default
    NOTIFY change_signal            // emitted when value changes
    REVISION int                    // version control for QML
    DESIGNABLE bool_expr            // visible in Qt Designer
    SCRIPTABLE bool_expr            // accessible from scripting
    STORED bool_expr                // serialized to stream
    USER bool_expr                  // "user-facing" property (one per class)
    CONSTANT                        // immutable after construction
    FINAL                           // cannot be overridden by subclass
    REQUIRED                        // must be set before component complete (QML)
    BINDABLE bindable_getter        // Qt 6 property bindings
)
```

### Qt 6 Bindable Properties

New in Qt 6: `QProperty<T>` and `QBindable<T>` create a dependency tracking system where changing one property automatically recomputes all properties that depend on it:

```cpp
class Circle : public QObject {
    Q_OBJECT
    Q_PROPERTY(qreal radius READ radius WRITE setRadius BINDABLE bindableRadius)
    Q_PROPERTY(qreal area READ area BINDABLE bindableArea)
public:
    // Bindable radius
    QProperty<qreal> m_radius{1.0};
    QProperty<qreal> m_area;
    
    Circle() {
        // area automatically recomputes when radius changes
        m_area.setBinding([this]() { 
            return M_PI * m_radius * m_radius; 
        });
    }
    
    QBindable<qreal> bindableRadius() { return &m_radius; }
    QBindable<qreal> bindableArea() { return &m_area; }
};
```

This is reactive programming baked into the property system. The dependency graph is tracked automatically — no manual signal/slot connections needed.

---

## WPF DependencyProperty

**Language:** C#  
**Key innovation:** Priority system, coerce-validate pipeline, attached properties

### Registration and Pipeline

```csharp
public static readonly DependencyProperty ValueProperty =
    DependencyProperty.Register(
        "Value",                        // property name
        typeof(double),                 // value type
        typeof(RangeControl),           // owner type
        new FrameworkPropertyMetadata(
            50.0,                       // default value
            FrameworkPropertyMetadataOptions.AffectsRender |  // layout/render hints
            FrameworkPropertyMetadataOptions.BindsTwoWayByDefault,
            OnValueChanged,             // PropertyChangedCallback
            CoerceValue                 // CoerceValueCallback
        ),
        ValidateValue                   // ValidateValueCallback (static, no context)
    );

// Validation (simple, no DependencyObject context — runs first)
private static bool ValidateValue(object value) {
    double d = (double)value;
    return !double.IsNaN(d) && !double.IsInfinity(d);
}

// Coercion (has DependencyObject context — runs after validation)
private static object CoerceValue(DependencyObject d, object baseValue) {
    RangeControl ctrl = (RangeControl)d;
    double val = (double)baseValue;
    // Clamp to current Min/Max (which may themselves be DependencyProperties)
    return Math.Clamp(val, ctrl.Minimum, ctrl.Maximum);
}

// Change notification
private static void OnValueChanged(DependencyObject d, DependencyPropertyChangedEventArgs e) {
    ((RangeControl)d).OnValueChanged((double)e.OldValue, (double)e.NewValue);
}
```

### Value Precedence System

WPF's most powerful feature: every dependency property can be set from multiple sources, and a fixed precedence determines the _effective_ value:

```
Highest precedence (wins):
  1. Active animations / animations holding final value
  2. Local value (explicitly set: myControl.Value = 5)
  3. TemplatedParent template bindings  
  4. Style setters (Trigger setters > Property setters)
  5. Theme/default style
  6. Property value inheritance (from parent in visual tree)
  7. Default value from metadata
Lowest precedence (loses)
```

This means you can set a default in metadata, override in a style, override in a template, override locally, and animations can temporarily override the local value — all without any if/else logic.

### Attached Properties

Properties defined on one type but attachable to any `DependencyObject`:

```csharp
// Define on Grid
public static readonly DependencyProperty RowProperty =
    DependencyProperty.RegisterAttached(
        "Row", typeof(int), typeof(Grid),
        new FrameworkPropertyMetadata(0,
            FrameworkPropertyMetadataOptions.AffectsParentArrange));

// Used on Button (not Grid)
<Button Grid.Row="2" Grid.Column="1" />  // attaches Grid.Row to Button
```

Attached properties are the extensibility mechanism: `Grid.Row`, `DockPanel.Dock`, `Canvas.Left`, `Validation.HasError` are all attached properties defined elsewhere but stored per-element.

---

## Node-RED

**Language:** JavaScript  
**Key innovation:** Config nodes as shared typed references, built-in validator compositions

### Full Node Definition

```javascript
RED.nodes.registerType('my-node', {
    category: 'function',
    color: '#a6bbcf',
    defaults: {
        name:    { value: "" },
        topic:   { value: "", required: true },
        timeout: { 
            value: 5000, 
            required: true,
            validate: RED.validators.number()
        },
        url: {
            value: "",
            required: true,
            validate: RED.validators.regex(/^https?:\/\/.*/)
        },
        // Config node reference — resolves to another node
        server: { 
            value: "", 
            type: "my-config-node",  // must be this node type
            required: true 
        },
    },
    credentials: {
        username: { type: "text" },
        password: { type: "password" },  // masked, stored encrypted
    },
    inputs: 1,
    outputs: 1,
    icon: "font-awesome/fa-cog",
    label: function() { return this.name || "my-node"; },
    labelStyle: function() { return this.name ? "node_label_italic" : ""; },
    
    // Dynamic output count
    outputLabels: function(index) {
        return ["success", "error"][index];
    },
    
    // Called when another node's config changes
    oneditprepare: function() {
        // Runs when editor dialog opens — can set up dynamic UI
        $("#node-input-timeout").typedInput({
            default: 'num',
            types: ['num', 'jsonata', 'env']
        });
    },
});
```

### Config Node Pattern

Config nodes are shared credential/connection stores referenced by ID:

```javascript
// Config node definition (server connection settings)
RED.nodes.registerType('my-server', {
    category: 'config',   // 'config' category = not shown in palette
    defaults: {
        host: { value: "localhost", required: true },
        port: { value: 1883, required: true, validate: RED.validators.number() },
        tls:  { value: false },
    },
    credentials: {
        username: { type: "text" },
        password: { type: "password" },
    },
    label: function() { return `${this.host}:${this.port}`; },
});
```

Worker nodes reference by ID; the runtime injects the config node's resolved value at execution time. This is the same DI pattern as Dagster's `ConfigurableResource` and Airflow's `Connections`.

---

## Cross-Cutting Patterns

### Pattern 1: Schema/State Separation

Every mature system eventually separates these:

| System | Schema object | State/value storage |
|--------|--------------|---------------------|
| Blender | `PropertyRNA` | `PointerRNA` target data |
| Unreal | `FProperty*` in `UClass` | per-instance memory via offset |
| Qt | `QMetaProperty` in `QMetaObject` | per-instance member variable |
| WPF | `DependencyProperty` (static) | `EffectiveValueEntry[]` per-instance |
| n8n | `INodeProperties[]` (static) | JSON workflow data |
| Dagster | `Config` class (Pydantic schema) | `OpExecutionContext` |

**Rule:** schema is shared across all instances; state is per-instance.

### Pattern 2: Constraint Layering

Most systems implement 2–3 constraint layers:

```
Layer 0: Type constraint        (must be int, float, string...)
Layer 1: Hard constraint        (must be ≥ 0 — domain invariant)
Layer 2: Soft/UI constraint     (slider range — UX guidance)
Layer 3: Computed constraint    (max ≤ other_field.value — runtime rule)
```

Blender names these explicitly (`min/max` vs `soft_min/soft_max`). Unreal uses `ClampMin/ClampMax` vs `UIMin/UIMax` in meta. WPF uses `CoerceValue` for layer 3.

### Pattern 3: Conditional Visibility Strategies

Three fundamentally different approaches:

```
1. Value equality (n8n displayOptions, NiFi dependsOn):
   Show field X when field Y == "value"
   Pro: Simple, predictable, no expression security issues
   Con: Can't express ranges, negations, cross-field math

2. Boolean expression (Unreal EditCondition, Unity [ShowIf]):
   Show field X when some_bool && other_field != value
   Pro: Handles most real cases
   Con: Limited expression power, static at compile time

3. Arbitrary code / poll function (Blender, Houdini, TouchDesigner):
   Show field X when my_poll_function(context) returns true
   Pro: Unlimited power
   Con: Performance (called every frame), security, serialization
```

### Pattern 4: The Sensitive/Secret Pattern

Every system that handles credentials eventually needs this:

```
NiFi:     .sensitive(true)          → encrypted at rest, masked in UI
n8n:      typeOptions: {password}   → masked in UI, stored encrypted
Prefect:  SecretStr                 → Pydantic type, .get_secret_value()
Dagster:  EnvVar("SECRET")          → late-bound, never in code
WPF:      credentials: {type:password}  → masked, encrypted in keychain
```

Common requirement: the value should never appear in logs, serialized workflow definitions, or version control.

### Pattern 5: Reset to Default

Less common but important for node editors where users experiment:

```
Qt:      RESET function in Q_PROPERTY declaration
Houdini: revertToDefaults() on ParmTemplate
Blender: right-click → "Reset to Default" (always available via RNA default)
Unity:   right-click property → "Reset" (reads PropertyAttribute.defaultValue)
```

---

## Constraint System Taxonomy

```
Constraint
├── Type constraints
│   ├── Primitive (int, float, string, bool)
│   ├── Composite (array, map, struct)
│   └── Reference (pointer to object of specific type)
│
├── Value constraints
│   ├── Range (min, max)
│   │   ├── Hard (enforced) / Soft (UI hint only)
│   │   └── Inclusive / Exclusive bounds
│   ├── Step / Multiple-of
│   ├── Precision (decimal places)
│   ├── Enumeration (allowable values set)
│   └── Pattern (regex for strings)
│
├── Presence constraints
│   ├── Required / Optional
│   ├── Non-null
│   └── Non-empty (distinct from non-null)
│
├── Cross-field constraints
│   ├── One field ≤ other field (min/max pair)
│   ├── Mutual exclusion
│   └── Sum constraint
│
└── Semantic constraints
    ├── Format (email, URL, ISO date, MIME type)
    ├── Unit (length, angle, time — for unit conversion)
    └── Subtype (color, direction, distance — for widget selection)
```

---

## Conditional Visibility Strategies

### Value-Based (n8n style)

```typescript
// Show field only when resource == "user" AND operation == "create"
displayOptions: {
    show: {
        resource: ["user"],         // OR within array
        operation: ["create"],      // AND across keys
    }
}
```

**Evaluation:** O(1) — pure hash lookup. Safe, predictable, serializable.  
**Limitation:** equality only, no range checks, no cross-field math.

### Expression-Based (Unreal/Unity/Houdini style)

```cpp
// Unreal meta
EditCondition = "Speed > 0.0 && Mode == EMode::Advanced"

// Houdini tags  
"disable_when": "{ method != advanced } { quality < 2 }"
```

**Evaluation:** String parsed to AST at load time, evaluated at render time.  
**Limitation:** DSL must be kept simple for security; serialization is lossy.

### Reactive/Computed (Qt Bindable, WPF Binding)

```cpp
// Qt 6 — reactive expression
m_area.setBinding([this]() { 
    return M_PI * m_radius * m_radius; 
});
```

**Evaluation:** Dependency graph tracked automatically; recomputes on change.  
**Limitation:** Only works within the same object graph, heavy for cross-object cases.

### Arbitrary Code (Blender poll, TouchDesigner)

```python
# Blender panel poll
@classmethod
def poll(cls, context):
    return (context.object 
            and context.object.type == 'MESH'
            and context.mode == 'OBJECT')
```

**Evaluation:** Called every draw frame — must be fast.  
**Limitation:** Not serializable, not previewable, security surface.

---

## Type System Approaches

### Stringly-Typed (n8n, ComfyUI, NiFi)

```typescript
type: 'string' | 'number' | 'boolean' | 'options' // runtime string
```

Pros: Trivially serializable to JSON. Easy to extend.  
Cons: No compile-time safety. Mistakes caught at runtime or in UI.

### Reflection-Based (Unity, Unreal, Qt)

```csharp
// Unity: reads Type object at runtime
FieldInfo field = typeof(MyClass).GetField("speed");
RangeAttribute range = field.GetCustomAttribute<RangeAttribute>();
```

Pros: No separate schema — the class _is_ the schema. Easy to keep in sync.  
Cons: Requires runtime reflection overhead. Can't inspect without an instance.

### Code-Generated (Unreal UHT, Qt MOC)

```
Source → Header Tool → Generated .cpp → Compiler → Binary with embedded metadata
```

Pros: Zero runtime overhead for schema lookup. Full type safety.  
Cons: Extra build step. Metadata frozen at compile time.

### Schema-as-Value (Blender RNA, Houdini ParmTemplate)

```python
# The schema IS a data structure you manipulate
tmpl = hou.FloatParmTemplate("radius", "Radius", 1)
tmpl.setMinValue(0.0)
group.addParmTemplate(tmpl)
```

Pros: Dynamic schema at runtime. Procedural generation. Easy introspection.  
Cons: Schema changes don't propagate cleanly. No compile-time guarantees.

### Type-Class Based (Prefect/Dagster Pydantic Config)

```python
class MyConfig(Config):
    speed: float = Field(ge=0.0, le=200.0)
```

Pros: Python type annotations _are_ the schema. Pydantic validates. IDE support.  
Cons: Python-only. Schema validation at runtime (not compile time in Python).

---

## Implications for Rust Implementation

### The Core Trade-off: Compile-Time vs Runtime Schema

```rust
// Compile-time approach — zero overhead, but schema is fixed
struct MyNodeParams {
    speed: f32,      // validator lives in a const somewhere
    mode: Mode,
}

// Runtime schema approach — dynamic, introspectable
struct ParamDef {
    name: SmartString<LazyCompact>,
    kind: ParamKind,
    constraints: Constraints,
    display_opts: DisplayOptions,
}
```

The Blender/Houdini model (schema-as-value) is more appropriate for a workflow engine than the Unreal/Unity model (schema-from-reflection), because:
1. Nodes are loaded dynamically (plugins/WASM)
2. Schema needs to be serialized to JSON for the frontend
3. Schema needs to be inspectable without instantiating a node

### Encoding Soft vs Hard Constraints

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberConstraints {
    // Hard bounds — enforced at write time (validation error if violated)
    pub hard_min: Option<f64>,
    pub hard_max: Option<f64>,
    
    // Soft bounds — UI hints only (slider range)
    // If None, UI derives from hard bounds or uses sensible default
    pub soft_min: Option<f64>,
    pub soft_max: Option<f64>,
    
    // Step for increment/decrement in UI
    pub step: Option<f64>,
    pub precision: Option<u8>,
}

impl NumberConstraints {
    pub fn validate(&self, value: f64) -> Result<(), ConstraintError> {
        if let Some(min) = self.hard_min {
            if value < min { return Err(ConstraintError::BelowMin { value, min }); }
        }
        if let Some(max) = self.hard_max {
            if value > max { return Err(ConstraintError::AboveMax { value, max }); }
        }
        Ok(())
    }
    
    // Clamp to soft range for UI display (no error)
    pub fn ui_range(&self) -> (f64, f64) {
        let min = self.soft_min
            .or(self.hard_min)
            .unwrap_or(f64::NEG_INFINITY);
        let max = self.soft_max
            .or(self.hard_max)
            .unwrap_or(f64::INFINITY);
        (min, max)
    }
}
```

### Subtype as a UI Dispatch Key

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloatSubtype {
    Plain,          // number input
    Distance,       // unit-aware, LENGTH
    Angle,          // stored radians, displayed degrees  
    Factor,         // 0..1, shown as percentage
    Pixel,          // integer-like
    Color,          // color picker (0..1)
    Percentage,     // 0..100 display
}

// The subtype drives:
// 1. Widget selection in the frontend
// 2. Unit conversion (if any)
// 3. Default soft_min/soft_max if not specified
// 4. Input validation hints
```

### Conditional Visibility — Value-Based First

n8n's `displayOptions` approach is the right starting point: pure value equality, no expressions, serializable. Expressions can be a later layer:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VisibilityCondition {
    /// Show when field `key` equals one of `values`
    WhenEquals {
        key: SmartString<LazyCompact>,
        values: Vec<serde_json::Value>,
    },
    /// All conditions must hold (AND)
    All(Vec<VisibilityCondition>),
    /// Any condition must hold (OR)
    Any(Vec<VisibilityCondition>),
    /// Invert
    Not(Box<VisibilityCondition>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayOptions {
    pub show: Option<VisibilityCondition>,
    pub hide: Option<VisibilityCondition>,
    /// If true, hidden fields are removed from DOM (not just invisible)
    pub remove_when_hidden: bool,
}
```

### The Sensitive Pattern in Rust

```rust
use secrecy::{SecretString, ExposeSecret};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum ParamValue {
    String(SmartString<LazyCompact>),
    Secret(SecretString),     // zeroized on drop, never serialized plaintext
    Number(f64),
    Bool(bool),
    // ...
}

impl Serialize for ParamValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Secret(_) => s.serialize_str("[REDACTED]"),
            // ...
        }
    }
}
```

### Reset / Default Value

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DefaultValue {
    /// Literal default
    Literal(serde_json::Value),
    /// Computed from expression (evaluated at param instantiation)
    Expression(String),
    /// No default — field is required
    None,
}
```

---

*End of reference document.*


