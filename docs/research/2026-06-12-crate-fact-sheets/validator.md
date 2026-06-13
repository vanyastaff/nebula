# nebula-validator — fact sheet

## Назначение
Валидационный rules-engine уровня Core с двумя поверхностями: (1) композируемые программные
валидаторы через трейт `Validate<T>` + комбинаторы `.and()/.or()/.not()`; (2) JSON-сериализуемый
декларативный enum `Rule` (typed sum-of-sums: Value/Predicate/Logic/Deferred/Described), который
несут поля схем — движок исполняет его на lint/activation/runtime. Maturity: `frontier` (ADR-0052/0080).

## Публичная поверхность
- `Validate<T>` — core-трейт валидатора — src/foundation/traits.rs:97
- `ValidateExt<T>` — комбинаторные методы — src/foundation/traits.rs:238; `Validatable` — traits.rs:192
- `ValidationError` — структурная ошибка, <=80 байт, Cow, RFC 6901 пути — src/foundation/error/validation_error.rs:103
- `ValidationErrors` — мульти-ошибки — src/foundation/error/validation_errors.rs:11
- `FieldPath` — RFC 6901 JSON-pointer с валидацией при конструировании — src/foundation/field_path.rs:39
- `AnyValidator<T>` — type-erased валидатор — src/foundation/any.rs:91
- `Rule` — sum-of-sums enum, manual Serialize/Deserialize — src/rule/mod.rs:47; `Rule::validate` — mod.rs:83; `RuleKind` — mod.rs:171
- `ValueRule` — src/rule/value.rs:28; `Predicate` — predicate.rs:16; `Logic` — logic.rs:12; `DeferredRule` — deferred.rs:15; `PredicateContext` — context.rs:13
- `ExecutionMode` (`StaticOnly`/`Deferred`/`Full`) — src/engine.rs:35; `validate_rules` — engine.rs:67; `validate_rules_with_ctx` — engine.rs:77
- `Validated<T>` — proof-token, Serialize есть, Deserialize намеренно НЕТ — src/proof.rs:54
- policy: `Presence`/`Requiredness`/`VisibilityPolicy`/`RequiredPolicy` — src/policy/mod.rs:14/24/34/46; `resolve_field_policies` (единственная точка входа для nebula-schema::validate) — mod.rs:218; `FieldDirective`/`FieldPolicyDecl`/`FieldPlan`/`FieldPolicyResolution` — mod.rs:109/127/175/195
- `ValidatorError` (операционная, `#[derive(nebula_error::Classify)]`) — src/error.rs:30
- `#[derive(Validator)]` proc-macro (feature `derive`, subcrate macros/) — re-export src/lib.rs:82
- Built-ins (фабрики+типы): length/pattern/content/range/size/boolean/nullable + network/temporal (фичи) — src/validators/mod.rs:57-80
- `__private::regex` re-export для derive-вывода — src/lib.rs:94

## Workspace-зависимости
- Deps: `nebula-error` (features=["derive"], используется ТОЛЬКО для Classify в error.rs:28), `nebula-validator-macros` (path=macros, optional за фичей derive); внешние: thiserror, smallvec, regex, serde, serde_json.
- Зависят от него: `nebula-schema` (crates/schema/Cargo.toml:24), `nebula-sdk` (crates/sdk/Cargo.toml:24), `nebula-api` (crates/api/Cargo.toml:25).
- Фичи: default = derive + network + temporal.

## Структура модулей
- `foundation/` — трейты Validate/ValidateExt/Validatable, AnyValidator, ValidationError(+codes/severity/mode/pointer), FieldPath, свой prelude
- `combinators/` — And/Or/Not/When/Unless/Each/Field/JsonField/Lazy/WithMessage/WithCode/Nested/Optional/AllOf/AnyOf
- `validators/` — built-ins по категориям: length, pattern, content, range, size, boolean, nullable, network (cfg), temporal (cfg)
- `rule/` — Rule + value/predicate/logic/deferred/context + manual deserialize + constructors/helpers
- `engine.rs` — validate_rules / validate_rules_with_ctx + ExecutionMode
- `policy/` — движок When(Rule)-условий visibility/required; типизированные вердикты вместо bool
- `proof.rs` — Validated<T> proof-token (canon §4.5)
- `error.rs` — ValidatorError (операционная vs ValidationError-на-вход)
- `macros.rs` (приватный) — `validator!` + deprecated `compose!`/`any_of!`
- `macros/` — subcrate nebula-validator-macros: parse/ → model.rs → emit/ для #[derive(Validator)]
- tests/: contract/ (адверсариальные, wire-format compat fixtures, error-registry v1), integration/, derive_tests; 6 бенчей

## Напряжения
- Свой `ValidationError` дублирует канонический `nebula-error::ValidationError` — унификация СДЕЛАНА на ветке `refactor/error-unify-validation`, но НЕ смержена; в main крейт всё ещё определяет собственный тип (foundation/error/validation_error.rs:103), nebula-error нужен только ради Classify (error.rs:28)
- Deprecated макросы `compose!` (src/macros.rs:684) и `any_of!` (src/macros.rs:715) живы, тесты явно глушат предупреждение (macros.rs:868-884) — кандидаты на удаление
- Док-ложь: combinators/mod.rs:14,57 описывает комбинатор `Cached`/`cached()`, которого в exports нет (mod.rs:93-110)
- Три prelude (crate::prelude, foundation::prelude:84, combinators::prelude:127) — расползание поверхности импорта
- Wire-format `Rule` externally-tagged tuple-compact: смена сериализации ломает сохранённые правила; коды ошибок заморожены fixtures (tests/fixtures/compat/error_registry_v1.json)
- `#![allow(clippy::result_large_err)]` весь крейт (lib.rs:50) — осознанно, 80-байтовая ошибка по значению

## Роль в credential/resource redesign
Крейт напрямую НЕ затронут redesign'ом credential/resource (нет deps на эти крейты, они не зависят от него напрямую). Косвенная роль: по ADR-0052 P2 validator — единственный эмиттер `required`-ошибок, и credential write-path (P4) валидирует `data` перед persist через nebula-schema → nebula-validator. При коллапсе крейтов за sole-public `nebula-sdk` validator остаётся приватной impl-деталью (sdk уже зависит от него).
