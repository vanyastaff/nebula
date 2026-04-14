# nebula-expression

Expression and template engine for dynamic workflow data transformation.

## Scope

- In scope:
  - expression parsing, AST evaluation, and built-in function registry
  - template rendering with `{{ }}` expressions and whitespace control
  - evaluation context for node/execution/workflow/input variables
  - optional parsing/evaluation caches via built-in `nebula-expression` caches
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

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [MIGRATION.md](./MIGRATION.md)


