# ADR-0053: Two-struct DX — Action struct + `Self::Input` consolidation

**Status:** Proposed (2026-05-14)
**Tags:** action, dx, schema, derive, deferred-decision

## Context

ADR-0043 §Negative explicitly acknowledged: *"Two structs per action
(Self + Input) — verbose by single-line metric."* Current pattern:

```rust
#[derive(Action)]
struct SendTelegram {
    #[require("bot")]   bot:   Handle<TelegramBot>,
    #[require("auth")]  token: Handle<TelegramCredential>,
}

#[derive(Schema, Deserialize)]
struct SendMessageInput {
    chat_id: i64,
    text:    String,
}

impl StatelessAction for SendTelegram {
    type Input  = SendMessageInput;
    type Output = MessageId;
    async fn execute(&self, input: SendMessageInput, ctx: ...) -> ... { ... }
}
```

Author writes **two struct definitions** for what is conceptually one
action: slot-binding fields on `Self`, form-input fields on
`Self::Input`. withoutboats called this *"the original sin"* during
Day 2 conference.

## Options

### Option 1 — Status quo (two structs)

Pros: clear separation of concerns (`Self` = capabilities, `Input` =
data); slot binding lives where it belongs (struct that holds runtime
handles).
Cons: verbose for trivial actions; authors mix up which struct gets
which field; `#[derive]` boilerplate doubled.

### Option 2 — Single struct with field discriminators

```rust
#[derive(Action)]
#[action(key = "telegram.send")]
struct SendTelegram {
    #[require("bot")]   bot:   Handle<TelegramBot>,
    #[require("auth")]  token: Handle<TelegramCredential>,

    #[input]            chat_id: i64,
    #[input]            text:    String,
}
```

`#[require]` fields → slot bindings (acquired at instantiation).
`#[input]` fields → user form data (deserialized per execute).
Macro emits both `FromWorkflowNode` impl and synthetic `Input` struct.

Pros: one struct authored; clearer co-location.
Cons: struct represents two distinct lifetimes (slot fields persist
per-instance, input fields per-execution); macro complexity grows;
`Self::Input` becomes hidden derived type — worse error messages.

### Option 3 — Function-style for slot-less, two-struct for slot-bound

`#[action]` attribute macro on `async fn` covers slot-less case in
4 lines (already in ADR-0052). For slot-bound actions, two-struct
remains. **No consolidation attempted; problem split by case.**

## Decision

**Defer.** Insufficient evidence which trade-off authors prefer in
practice. After Concept A-modified ships and `#[action]` function-style
covers the slot-less majority, gather adoption data. Revisit when
slot-bound actions proven painful **with concrete user feedback**.

Until then: keep two-struct pattern (Option 1) for slot-bound actions;
function-style `#[action]` for slot-less. Document both clearly.

## Consequences

### Positive

- No premature optimization for cases we haven't measured.
- Function-style covers majority of slot-less actions (Maxim Fateev
  estimate: 60-70% of activities in production).

### Negative

- Pain documented in ADR-0043 not resolved.
- Slot-bound action authoring remains verbose.

### Neutral

- Future ADR may adopt Option 2 if data justifies.
- Migration path exists if Option 2 chosen later (additive macro
  feature).

## Status of related ADRs

- ADR-0043 (dependency declaration DX) — pain acknowledged here
  remains; this ADR refuses to resolve until evidence accumulates.
- ADR-0052 (action surface hybrid) — function-style `#[action]`
  partially mitigates by removing struct entirely for slot-less case.

## Out of scope

- Concrete syntax for Option 2 if eventually adopted (would be a new
  ADR).
- Migration tooling — only relevant if migration ever happens.

## References

- Conference Day 2 morning (CONFERENCE-NOTES.md) — withoutboats
  raised the issue; deferred there.
- Yoshua Wuyts: "fix one thing at a time. Land Concept A-modified;
  watch what hurts; iterate."
