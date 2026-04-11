# Architect Mode

When asked to review architecture or propose improvements, follow these principles:

## Stance

- **Break things to make them right.** Propose changes that improve correctness, safety, and simplicity — even if they break existing code.
- **No adapters, bridges, or shims.** If the old API is wrong, replace it. Don't wrap it.
- **No backward compatibility tax.** Don't keep old interfaces alive "just in case". Delete the old code.
- **Migration cost is acceptable.** If a change is the right design, the cost of updating callers is justified.

## What to Propose

- Type-level guarantees that eliminate runtime checks (newtypes, typestate, `Validated<T>`)
- Simpler APIs that make misuse impossible (builder pattern, session types)
- Removing abstraction layers that add indirection without value
- Merging or splitting crates when boundaries are wrong
- Replacing stringly-typed interfaces with enums or trait objects
- Eliminating `clone()` where ownership transfer or borrowing is correct

## What NOT to Propose

- "Future-proofing" abstractions for hypothetical requirements
- Adding traits/generics when a concrete type suffices
- Wrapping external crates in internal abstractions "for portability"
- Feature flags for things that should just be the default

## Output Format

When proposing architectural changes:

1. **Problem** — what's wrong now (with concrete code references)
2. **Proposal** — the new design (with type signatures, not just prose)
3. **Breaking changes** — exactly what callers need to update
4. **Risk** — what could go wrong, what's the rollback path

Do not implement unless asked. Present the proposal and wait for approval.

## Scope Awareness

- Changes to `nebula-core` traits affect 25+ crates — always quantify the blast radius
- Cross-cutting crates (resilience, eventbus, log) are safer to change — fewer dependents
- Business-layer crates (credential, resource, action) can be changed freely
