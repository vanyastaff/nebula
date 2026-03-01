# nebula-parameter

Node input schema definition for Nebula workflow platform.

## Scope

- **In scope:**
  - **def** — `ParameterDef` (tagged enum, 19 variants: Text, Textarea, Code, Secret, Number, Checkbox, Select, MultiSelect, Color, DateTime, Date, Time, Hidden, Notice, Object, List, Mode, Group, Expirable); delegation to metadata/display/validation_rules/children.
  - **kind** — `ParameterKind`, `ParameterCapability`; value_type(), capabilities(), predicates (is_editable, is_validatable, etc.).
  - **types/** — one struct per kind (e.g. `TextParameter`, `NumberParameter`, `SelectParameter`, `ObjectParameter`, `ListParameter`, `ModeParameter`, `GroupParameter`, `ExpirableParameter`); metadata, display, validation, type-specific options.
  - **metadata** — `ParameterMetadata` (key, name, description, required, placeholder, hint, sensitive).
  - **validation** — `ValidationRule` (MinLength, MaxLength, Pattern, Min, Max, OneOf, MinItems, MaxItems, Custom); declarative only; evaluation via nebula-validator.
  - **display** — `ParameterDisplay`, `DisplayCondition`, `DisplayRuleSet`, `DisplayContext`; show_when/hide_when.
  - **collection** — `ParameterCollection`; validate(&ParameterValues) pipeline.
  - **values** — `ParameterValues`, `ParameterSnapshot`, `ParameterDiff`.
  - **option** — `SelectOption`, `OptionsSource`.
  - **error** — `ParameterError` (variants, category(), code(), is_retryable()).
- **Out of scope:** UI widget dimensions, styling, layout; expression resolution (nebula-expression); credential resolution (nebula-credential); workflow orchestration.

## Current State

- **Maturity:** Stable schema layer; used by action, credential, engine, macros, sdk
- **Key strengths:** JSON-serializable schema, capability-based kind semantics, error aggregation, recursive containers
- **Key risks:** Raw JSON values push type mismatch detection late; display rule cycles not preflighted

## Target State

- **Production criteria:** Stable error codes, deterministic validation order, schema lint pass, typed value layer (optional)
- **Compatibility guarantees:** Patch/minor preserve API; breaking changes in MIGRATION.md

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
