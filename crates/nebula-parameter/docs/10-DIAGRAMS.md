# Architecture Diagrams

Visual representation of the Nebula Parameter System architecture.

---

## High-Level Architecture

```
+----------------------------------------------------------+
|                    USER APPLICATION                       |
|                                                           |
|  use nebula_parameter::prelude::*;                       |
|                                                           |
|  let mut context = Context::new(schema);                 |
|  context.set_value("email", "test@example.com".into())?; |
|  context.undo()?;                                        |
+----------------------------------------------------------+
                           |
                           | uses
                           v
+----------------------------------------------------------+
|                  NEBULA-PARAMETER CRATE                   |
|                                                           |
|  +----------------------------------------------------+  |
|  |                    PUBLIC API                       |  |
|  |                                                     |  |
|  |  - Schema      - Parameter definitions             |  |
|  |  - Context     - Runtime state + operations        |  |
|  |  - Parameters  - Text, Number, Bool, Vector, etc.  |  |
|  |  - Validation  - Validators, Errors                |  |
|  |  - Events      - Event system, Observers           |  |
|  |  - History     - Undo/Redo commands                |  |
|  +----------------------------------------------------+  |
|                           |                               |
|  +----------------------------------------------------+  |
|  |                 INTERNAL LAYERS                     |  |
|  |                                                     |  |
|  |  +----------------------------------------------+  |  |
|  |  |       LAYER 1: SCHEMA (Immutable)            |  |  |
|  |  |                                               |  |  |
|  |  |  TextParameter {                             |  |  |
|  |  |    metadata: Arc<Metadata>,                  |  |  |
|  |  |    flags: Flags,                    |  |  |
|  |  |    validators: Vec<Arc<Validator>>,          |  |  |
|  |  |    transformers: Vec<Arc<Transformer>>,      |  |  |
|  |  |  }                                            |  |  |
|  |  +----------------------------------------------+  |  |
|  |                        |                            |  |
|  |                        v                            |  |
|  |  +----------------------------------------------+  |  |
|  |  |       LAYER 2: RUNTIME (Mutable)             |  |  |
|  |  |                                               |  |  |
|  |  |  RuntimeParameter<T> {                       |  |  |
|  |  |    schema: Arc<T>,       // Shared           |  |  |
|  |  |    state: ParameterState, // Mutable         |  |  |
|  |  |    value: Value,          // Mutable         |  |  |
|  |  |  }                                            |  |  |
|  |  +----------------------------------------------+  |  |
|  |                        |                            |  |
|  |                        v                            |  |
|  |  +----------------------------------------------+  |  |
|  |  |       LAYER 3: CONTEXT (Orchestration)       |  |  |
|  |  |                                               |  |  |
|  |  |  Context {                                    |  |  |
|  |  |    schema: Arc<Schema>,                      |  |  |
|  |  |    parameters: HashMap<Key, RuntimeParam>,   |  |  |
|  |  |    event_bus: EventBus,                      |  |  |
|  |  |    history: HistoryManager,                  |  |  |
|  |  |  }                                            |  |  |
|  |  +----------------------------------------------+  |  |
|  +----------------------------------------------------+  |
+----------------------------------------------------------+
                           |
                           | depends on
                           v
+----------------------------------------------------------+
|                  EXTERNAL DEPENDENCIES                    |
|                                                           |
|  - tokio::sync::broadcast  - Event bus                   |
|  - serde                   - Serialization               |
|  - smartstring             - Efficient strings           |
|  - regex                   - Pattern matching            |
|  - bitflags                - Flag enums                  |
+----------------------------------------------------------+
```

---

## Module Dependency Graph

```
                    +-------------+
                    |   prelude   |
                    +------+------+
                           | re-exports
                           |
        +------------------+------------------+
        |                  |                  |
        v                  v                  v
   +--------+        +---------+       +----------+
   | schema |        | context |       | parameter|
   +----+---+        +----+----+       +-----+----+
        |                 |                   |
        |                 |                   |
        |    +------------+------------+      |
        |    |                         |      |
        v    v                         v      v
   +------------+                +----------------+
   |  runtime   |                |     core       |
   | parameter  |                | - metadata     |
   +-----+------+                | - flags        |
         |                       | - value        |
         |                       | - subtype      |
         |                       +--------+-------+
         |                                |
         +----------------+---------------+
                          |
         +----------------+----------------+
         |                |                |
         v                v                v
    +---------+     +----------+    +------------+
    | history |     |  event   |    | validation |
    +----+----+     +-----+----+    +------------+
         |                |
         |                v
         |          +----------+
         |          | observer |
         |          +----------+
         |
         +----------+------------+
                    |            |
                    v            v
              +----------+  +------------+
              |validator |  |transformer |
              +----------+  +------------+
```

---

## Data Flow: Set Value Operation

```
User calls:
  context.set_value("email", "test@example.com")
       |
       v
+----------------------------------------------+
| Context::set_value()                         |
|  1. Get old value                            |
|  2. Create SetValueCommand                   |
|  3. Pass to HistoryManager                   |
+----------------------+-----------------------+
                       |
                       v
+----------------------------------------------+
| HistoryManager::execute()                    |
|  1. Try merge with last command              |
|  2. Execute command                          |
|  3. Add to undo stack                        |
|  4. Clear redo stack                         |
+----------------------+-----------------------+
                       |
                       v
+----------------------------------------------+
| SetValueCommand::execute()                   |
|  1. Call context.set_value_internal()        |
+----------------------+-----------------------+
                       |
                       v
+----------------------------------------------+
| RuntimeParameter::set_value()                |
|  1. Emit BeforeChange event                  |
|  2. Apply transformers (clamp, round, etc.)  |
|  3. Validate value (sync validators)         |
|  4. Store value                              |
|  5. Update state (dirty, touched)            |
|  6. Emit AfterChange event                   |
+----------------------+-----------------------+
                       |
                       v
+----------------------------------------------+
| EventBus::emit()                             |
|  1. Notify callback observers                |
|  2. Send to broadcast channel                |
+----------------------+-----------------------+
                       |
                       v
+----------------------------------------------+
| Observers receive events                     |
|  - LoggerObserver: logs event                |
|  - ValidationObserver: auto-validates        |
|  - DisplayObserver: updates visibility       |
|  - UiObserver: updates UI                    |
+----------------------------------------------+
```

---

## Value Processing Pipeline

```
     User Input
          |
          v
+-------------------+
|   1. TRANSFORM    |  Coerce value to valid form
|                   |  - ClampTransformer
|                   |  - RoundTransformer
|                   |  - ModuloTransformer
+--------+----------+
         |
         v
+-------------------+
|   2. VALIDATE     |  Check constraints (sync)
|      (SYNC)       |  - Type checks
|                   |  - Range validation
|                   |  - Pattern matching
+--------+----------+
         |
         v
+-------------------+
|   3. VALIDATE     |  External checks (async)
|      (ASYNC)      |  - API validation
|                   |  - Database checks
|                   |  - Uniqueness
+--------+----------+
         |
         v
+-------------------+
|   4. SET VALUE    |  Store in context
|                   |  - Update value
|                   |  - Update state flags
+--------+----------+
         |
         v
+-------------------+
|   5. NOTIFY       |  Emit change events
|                   |  - BeforeChange
|                   |  - AfterChange
|                   |  - ValidationPassed
+-------------------+
```

---

## Memory Layout

```
Stack:                          Heap:

+----------------+             +------------------------+
|    Context     |------------>|    Schema (Arc)        |
|                |             |  - parameters          |
|  schema -------+------+      +------------------------+
|  parameters    |      |               |
|  event_bus     |      |               | shared by
|  history       |      |               |
+----------------+      |               v
                        |      +------------------------+
                        |      | TextParameter (Arc)    |
                        |      |  - metadata            |
                        |      |  - flags               |
                        |      |  - validators          |
                        |      +------------------------+
                        |               ^
                        |               | shared by
                        |               |
                        +---------------+
                                        |
+---------------------------------------+-----------------+
| HashMap<Key, RuntimeParameter>                          |
|                                                         |
|  "email" -> RuntimeParameter {                          |
|               schema: Arc (shared) -------------------+ |
|               state: ParameterState,                  | |
|               value: Value::Text("..."),              | |
|             }                                         | |
|                                                       | |
|  "username" -> RuntimeParameter {                     | |
|                  schema: Arc (shared) ----------------+ |
|                  state: ParameterState,                 |
|                  value: Value::Text("..."),             |
|                }                                        |
+---------------------------------------------------------+
```

**Key Points:**
- Schema is `Arc<T>` - shared across all RuntimeParameters
- Each RuntimeParameter has its own state + value
- Memory efficient: schema shared, only state/value duplicated

---

## Threading Model

```
+----------------------------------------------------------+
|                     Main Thread                           |
|                                                           |
|  Context (NOT Send, NOT Sync)                            |
|    +-- schema: Arc<Schema>        (shareable)            |
|    +-- parameters: HashMap         (owned)               |
|    +-- event_bus: EventBus         (Clone + Send)        |
|    +-- history: HistoryManager     (owned)               |
+----------------------------+-----------------------------+
                             |
                             | .clone() event_bus
                             |
         +-------------------+-------------------+
         |                                       |
         v                                       v
+------------------+                   +------------------+
|   UI Thread      |                   |  Worker Thread   |
|                  |                   |                  |
|  let mut rx =    |                   |  let mut rx =    |
|    bus.receiver()|                   |    bus.receiver()|
|                  |                   |                  |
|  tokio::spawn(   |                   |  std::thread::   |
|    async move {  |                   |    spawn(move {  |
|      while let   |                   |      while let   |
|        Ok(e) =   |                   |        Ok(e) =   |
|        rx.recv() |                   |        rx.recv() |
|          .await  |                   |          .await  |
|      {           |                   |      {           |
|        update_ui |                   |        process   |
|      }           |                   |      }           |
|    }             |                   |    }             |
|  )               |                   |  )               |
+------------------+                   +------------------+
```

**Key Points:**
- Context is single-threaded
- EventBus is Clone + Send - can be shared across threads
- Receivers work in async or sync contexts
- Schema (Arc) can be shared across threads safely

---

## Type System Hierarchy

```
                        Parameter
                            |
          +-----------------+------------------+
          |                 |                  |
     Primitive          Composite          Special
          |                 |                  |
    +-----+-----+     +-----+-----+      +-----+-----+
    |     |     |     |     |     |      |     |     |
  Text Number Bool  Object List Mode   Enum Dynamic Action
    |     |                   |
    |     |                   |
    |  +--+--+           +----+----+
    |  |     |           |         |
    | i32  f64       Schema    Schema
    | i64  f32      (nested)  (per variant)
```

---

## Value Enum Variants

```
                       Value
                         |
     +-------------------+-------------------+
     |         |         |         |         |
   Null      Bool      Int      Float     Text
     |         |         |         |         |
     -       bool      i64       f64    SmartString
     
     +-------------------+-------------------+
     |         |         |         |         |
   Array    Object    Binary  Expression
     |         |         |         |
 Arc<[Value]> Arc<HashMap> Arc<[u8]>  template +
                                      compiled
```

---

## Event System Flow

```
+---------------+     emit()      +---------------+
|    Context    +---------------->|   EventBus    |
+---------------+                 +-------+-------+
                                          |
                    +---------------------+---------------------+
                    |                     |                     |
                    v                     v                     v
           +---------------+     +---------------+     +---------------+
           |   Observer 1  |     |   Observer 2  |     |   Observer 3  |
           | (Validation)  |     |   (Logging)   |     |     (UI)      |
           +---------------+     +---------------+     +---------------+
                    |
                    v
           +---------------+
           | DisplayObserver|
           | (Visibility)  |
           +---------------+
                    |
                    v
           +---------------+
           | VisibilityChanged
           | Events emitted
           +---------------+
```

---

## Undo/Redo Stack

```
                    HistoryManager
                          |
          +---------------+---------------+
          |                               |
    Undo Stack                      Redo Stack
          |                               |
    +-----+-----+                   +-----+-----+
    |     |     |                   |     |     |
  Cmd3  Cmd2  Cmd1               Cmd4  Cmd5   ...
    |
    v
+-------------------+
| SetValueCommand   |
|  - key: "email"   |
|  - old: "a@b.com" |
|  - new: "x@y.com" |
+-------------------+

         UNDO                           REDO
           |                              |
           v                              v
  Pop from Undo Stack            Pop from Redo Stack
  Execute cmd.undo()             Execute cmd.execute()
  Push to Redo Stack             Push to Undo Stack
```

---

## Schema vs Runtime Separation

```
+-----------------------------------------------------------+
|                    SCHEMA (Immutable)                      |
|                                                            |
|  +-------------+  +-------------+  +-------------+         |
|  |TextParameter|  |NumberParam  |  |BoolParameter|         |
|  |             |  |             |  |             |         |
|  | metadata    |  | metadata    |  | metadata    |         |
|  | subtype     |  | subtype     |  | default     |         |
|  | validators  |  | hard_min    |  |             |         |
|  | transformers|  | hard_max    |  |             |         |
|  +-------------+  | soft_min    |  +-------------+         |
|                   | soft_max    |                          |
|                   | unit        |                          |
|                   +-------------+                          |
|                                                            |
|  Shared via Arc - ONE copy in memory                       |
+-----------------------------------------------------------+
                           |
                           | wrapped by
                           v
+-----------------------------------------------------------+
|                   RUNTIME (Mutable)                        |
|                                                            |
|  +---------------------+  +---------------------+          |
|  | RuntimeParameter<T> |  | RuntimeParameter<T> |          |
|  |                     |  |                     |          |
|  | schema: Arc --------+--+-- schema: Arc       |          |
|  | value: Value        |  | value: Value        |          |
|  | state: ParamState   |  | state: ParamState   |          |
|  +---------------------+  +---------------------+          |
|                                                            |
|  Each form instance has own values + state                 |
+-----------------------------------------------------------+
```

---

## Feature Gates

```
+-----------------------------------------------+
|           Core (always available)              |
|  - Schema                                      |
|  - Parameters (Text, Number, Bool, etc.)       |
|  - Validation                                  |
|  - Transformers                                |
+-----------------------------------------------+
                      |
                      |
       +--------------+--------------+
       |              |              |
       v              v              v
  +--------+     +--------+     +---------+
  |datetime|     |  uuid  |     |  array  |
  |feature |     |feature |     | feature |
  +--------+     +--------+     +---------+
       |              |              |
       +--------------+--------------+
                      |
                      v
            +------------------+
            |    ui feature    |
            |  - UIHints       |
            |  - WidgetType    |
            |  - I18n metadata |
            +------------------+
                      |
             +--------+--------+
             v                 v
       +----------+      +----------+
       | ui-egui  |      | ui-iced  |
       +----------+      +----------+
```

---

## Display Conditions

```
                    DisplayRule
                         |
         +---------------+---------------+
         |               |               |
     show_when       hide_when     show_and_enable
         |               |               |
         v               v               v
     Condition       Condition       Condition
         |
    +----+----+----+----+
    |    |    |    |    |
 Equals  In  Greater And  Or
    |              Than  |    |
    v                    v    v
+--------+          +---+    +---+
|key:mode|          |...|    |...|
|val:adv |          +---+    +---+
+--------+
```

---

## Complete Example: HTTP Request Node

```
Schema::new()
    |
    +-- TextParameter::url("base_url")
    |       label: "Base URL"
    |       required: true
    |
    +-- EnumParameter::builder("method")
    |       options: [GET, POST, PUT, DELETE]
    |       default: GET
    |
    +-- ModeParameter::builder("auth")
    |       |
    |       +-- variant("none")
    |       |       Schema::empty()
    |       |
    |       +-- variant("basic")
    |       |       +-- TextParameter("username")
    |       |       +-- TextParameter::password("password")
    |       |
    |       +-- variant("bearer")
    |       |       +-- TextParameter::secret("token")
    |       |
    |       +-- variant("oauth2")
    |               +-- TextParameter("client_id")
    |               +-- TextParameter::secret("client_secret")
    |               +-- TextParameter::url("token_url")
    |
    +-- TextParameter::json("body")
    |       display_when: method IN [POST, PUT, PATCH]
    |
    +-- ListParameter("headers")
    |       item_schema:
    |           +-- TextParameter("key")
    |           +-- TextParameter("value")
    |
    +-- NumberParameter::duration_seconds("timeout")
            range: 1..300
            default: 30
```

---

## Industry Comparison

```
+------------+-------+-------+-----+----+--------+--------+
|  Feature   |Blender|Unreal | n8n | Qt |Houdini | Nebula |
+------------+-------+-------+-----+----+--------+--------+
|Type Safety |   -   |   ~   |  -  | ~  |   -    |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Subtype+Unit|  YES  |  YES  |  -  | -  |   ~    |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Soft/Hard   |  YES  |  YES  |  -  | -  |  YES   |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Mode/Branch |   -   |   -   | YES | -  |   -    |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Expressions |  YES  |   ~   | YES | ~  |  YES   |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Undo/Redo   |   -   |  YES  |  -  | -  |  YES   |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Events      |   -   |  YES  |  -  |YES |   -    |   YES  |
+------------+-------+-------+-----+----+--------+--------+
|Zero-Cost   |   -   |   ~   |  -  | ~  |   -    |   YES  |
+------------+-------+-------+-----+----+--------+--------+

Nebula combines ALL patterns from across the industry!
```

---

## Summary

### Architecture Layers:
1. **Schema Layer** (Immutable) - Parameter definitions
2. **Runtime Layer** (Mutable) - State + Value  
3. **Context Layer** (Orchestration) - Everything together

### Key Components:
- **Schema** - Parameter definitions (Arc, shareable)
- **RuntimeParameter<T>** - schema + state + value
- **Context** - Manages all parameters + history + events
- **EventBus** - tokio::broadcast wrapper
- **HistoryManager** - Command pattern for undo/redo

### Data Flow:
```
User -> Context -> HistoryManager -> Command -> RuntimeParameter -> EventBus -> Observers
```

### Threading:
- Context: Single-threaded
- EventBus: Multi-threaded (Clone + Send)
- Schema: Shareable (Arc)
