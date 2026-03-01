# nebula-expression

Expression and template engine for dynamic workflow data transformation.

## Scope

- In scope:
  - expression parsing, AST evaluation, and built-in function registry
  - template rendering with `{{ }}` expressions and whitespace control
  - evaluation context for node/execution/workflow/input variables
  - optional parsing/evaluation caches via `nebula-memory`
- Out of scope:
  - workflow orchestration and scheduling
  - storage of execution results
  - credential ownership and secret lifecycle

## Current State

- maturity: implemented and actively tested crate with parser/evaluator/template features.
- key strengths:
  - broad language surface (operators, conditionals, pipelines, lambdas)
  - clear `ExpressionEngine` API and `EvaluationContext` model
  - template engine with positional error formatting
  - security-conscious protections (regex ReDoS pattern checks, recursion depth limits)
- key risks:
  - large language surface increases long-term compatibility burden
  - context shape and function behavior drift can break downstream expectations if not governed

## Target State

- production criteria:
  - stable expression semantics across runtime/action usage
  - explicit compatibility policy for built-ins and syntax evolution
  - deterministic and observable failure modes in execution paths
- compatibility guarantees:
  - additive functions/features in minor releases
  - grammar/semantic breaks only in major releases with migration notes

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
