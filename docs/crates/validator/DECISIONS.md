# Decisions

## D-001: Trait-bound Driven Type Safety

Status: accepted

Decision:
- validators declare input constraints through trait bounds instead of runtime type checks.

Reason:
- fail invalid combinations at compile time.

## D-002: Combinator-first Composition Model

Status: accepted

Decision:
- compose validators via typed combinators and extension methods.

Reason:
- supports reusable and testable validation pipelines.

## D-003: Structured Error Model with Nesting

Status: accepted

Decision:
- keep `ValidationError` rich (code/message/field/params/nested/severity/help).

Reason:
- required for API-grade diagnostics and nested object validation.

## D-004: Context Support for Cross-field Rules

Status: accepted

Decision:
- provide `ValidationContext` + contextual validator trait.

Reason:
- many business rules depend on multiple fields or external context.

## D-005: Macro-based Ergonomics

Status: accepted

Decision:
- keep `validator!` for common validator definitions.

Reason:
- reduces boilerplate while preserving static typing and explicit generated behavior.
