# nebula-validator — design

| Field | Value |
|-------|-------|
| **Status** | `frontier` (ADR-0052/0080) — программный API стабилен, `Rule` wire-format недавно сменился |
| **Layer** | Core cross-cutting — rules-engine, к которому `nebula-schema` делегирует исполнение правил |
| **Redesign role** | **Не затронут напрямую** redesign'ом credential/resource (нет deps на эти крейты и нет обратных). Косвенный участник: единственный эмиттер `required`-ошибок (ADR-0052 P2) на write-path credential (P4) через `nebula-schema` |
| **Related** | [ADR-0052], ADR-0080, PRODUCT_CANON §3.5 / §4.5, [refactor/error-unify-validation] (несмёрженная ветка унификации ошибок) |

---

## 1. Назначение и границы

`nebula-validator` — это shared rules-engine уровня Core с двумя поверхностями:

1. **Программные валидаторы** — трейт `Validate<T>` (`src/foundation/traits.rs:97`) плюс
   комбинаторы `.and()/.or()/.not()` через `ValidateExt<T>` (`traits.rs:238`). Авторы
   интеграций компонуют проверки прямо в Rust-коде.
2. **Декларативный `Rule`** — JSON-сериализуемый typed sum-of-sums enum
   (`src/rule/mod.rs:47`), который несут поля схем. Движок исполняет его на
   lint / activation / runtime.

**Владеет:** трейтами валидации (`Validate`, `ValidateExt`, `Validatable`), type-erasure
(`AnyValidator<T>`), структурной `ValidationError` (<=80 байт, `Cow`, RFC 6901 пути),
декларативным `Rule` + его исполнителем (`engine.rs`), proof-token'ом `Validated<T>`
(canon §4.5), policy-движком visibility/required-условий (`policy/`), каталогом встроенных
валидаторов (length/pattern/content/range/size/boolean/nullable + network/temporal за
фичами) и derive-макросом `#[derive(Validator)]`.

**ЯВНО НЕ делает** (из non-goals README): это **не** schema-система — `Field`/`Schema` и
пайплайн `ValidValues → ResolvedValues` живут в `nebula-schema`; **не** вычислитель
выражений (`nebula-expression`); **не** resilience-пайплайн (`nebula-resilience`); **не**
форматтер API-ошибок — RFC 9457 `problem+json` маппинг делает `nebula-api`. KDF/hashing
здесь тоже нет (это `nebula-credential`/`nebula-crypto`).

## 2. Публичная поверхность

| Item | Где |
|------|-----|
| `Validate<T>` — core-трейт валидатора | `src/foundation/traits.rs:97` |
| `ValidateExt<T>` — комбинаторные методы `.and()/.or()/.not()` | `src/foundation/traits.rs:238` |
| `Validatable` | `src/foundation/traits.rs:192` |
| `ValidationError` — структурная ошибка (<=80 байт, `Cow`, RFC 6901) | `src/foundation/error/validation_error.rs:103` |
| `ValidationErrors` — мульти-ошибки | `src/foundation/error/validation_errors.rs:11` |
| `FieldPath` — RFC 6901 JSON-pointer, валидация при конструировании | `src/foundation/field_path.rs:39` |
| `AnyValidator<T>` — type-erased валидатор | `src/foundation/any.rs:91` |
| `Rule` — sum-of-sums, ручные `Serialize`/`Deserialize` | `src/rule/mod.rs:47` |
| `Rule::validate` / `RuleKind` | `src/rule/mod.rs:83` / `mod.rs:171` |
| `ValueRule` / `Predicate` / `Logic` / `DeferredRule` / `PredicateContext` | `src/rule/value.rs:28` / `predicate.rs:16` / `logic.rs:12` / `deferred.rs:15` / `context.rs:13` |
| `ExecutionMode` (`StaticOnly`/`Deferred`/`Full`) | `src/engine.rs:35` |
| `validate_rules` / `validate_rules_with_ctx` | `src/engine.rs:67` / `engine.rs:77` |
| `Validated<T>` — proof-token (`Serialize` есть, `Deserialize` намеренно НЕТ) | `src/proof.rs:54` |
| `Presence` / `Requiredness` / `VisibilityPolicy` / `RequiredPolicy` | `src/policy/mod.rs:14/24/34/46` |
| `resolve_field_policies` — единственная точка входа для `nebula-schema::validate` | `src/policy/mod.rs:218` |
| `FieldDirective` / `FieldPolicyDecl` / `FieldPlan` / `FieldPolicyResolution` | `src/policy/mod.rs:109/127/175/195` |
| `ValidatorError` — операционная ошибка (`#[derive(nebula_error::Classify)]`) | `src/error.rs:30` |
| `#[derive(Validator)]` proc-macro (feature `derive`, subcrate `macros/`) | re-export `src/lib.rs:82` |
| Built-in фабрики/типы: length/pattern/content/range/size/boolean/nullable (+network/temporal) | `src/validators/mod.rs:57-80` |
| `__private::regex` re-export для вывода derive-кода | `src/lib.rs:94` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-error` (features=`["derive"]`, используется **только** ради `Classify` в
  `src/error.rs:28`), `nebula-validator-macros` (path=`macros`, optional за фичей `derive`);
  внешние — `thiserror`, `smallvec`, `regex`, `serde`, `serde_json`.
- **Зависят от него:** `nebula-schema` (`crates/schema/Cargo.toml:24`), `nebula-sdk`
  (`crates/sdk/Cargo.toml:24`), `nebula-api` (`crates/api/Cargo.toml:25`).
- **Фичи:** `default = derive + network + temporal`.

## 4. Внутренняя архитектура

- `foundation/` — трейты `Validate`/`ValidateExt`/`Validatable`, `AnyValidator`,
  `ValidationError` (+ codes/severity/mode/pointer), `FieldPath`, собственный prelude.
- `combinators/` — `And`/`Or`/`Not`/`When`/`Unless`/`Each`/`Field`/`JsonField`/`Lazy`/
  `WithMessage`/`WithCode`/`Nested`/`Optional`/`AllOf`/`AnyOf`.
- `validators/` — встроенные по категориям: length, pattern, content, range, size, boolean,
  nullable, network (cfg), temporal (cfg).
- `rule/` — `Rule` + `value`/`predicate`/`logic`/`deferred`/`context` + ручной deserialize +
  конструкторы/хелперы.
- `engine.rs` — `validate_rules` / `validate_rules_with_ctx` + `ExecutionMode`.
- `policy/` — движок `When(Rule)`-условий visibility/required; типизированные вердикты вместо
  голого `bool`.
- `proof.rs` — `Validated<T>` proof-token (canon §4.5).
- `error.rs` — `ValidatorError` (операционная), отделённая от `ValidationError`-на-вход.
- `macros.rs` (приватный) — `validator!` + deprecated `compose!`/`any_of!`.
- `macros/` — subcrate `nebula-validator-macros`: `parse/` → `model.rs` → `emit/` для
  `#[derive(Validator)]`.

**Поток данных (декларативный путь):** схема несёт `Rule` → `validate_rules(_with_ctx)`
выбирает по `ExecutionMode`, какие категории исполнять → `Rule::validate` диспетчеризует по
`RuleKind` на нужную inner-поверхность → результат — `Result<_, ValidationError(s)>`.
Программный путь: `Validate<T>::validate` (+ комбинаторы) → опционально `Validated<T>`.

## 5. Инварианты и контракты

- **[L1-§4.5] proof-token by-construction.** `Validated<T>` нельзя получить, не вызвав
  `validate`; `Deserialize` для него **намеренно не реализован** (`src/proof.rs:54`) —
  десериализованные данные обязаны повторно валидироваться.
- **Rule cross-kind safety.** Каждый inner-kind (`ValueRule`/`Predicate`/`Logic`/
  `DeferredRule`) экспонирует только осмысленный для него метод; вызов value-only метода на
  predicate-несущем `Rule` — ошибка компиляции. Это by-construction замена «silent-pass»
  эргономики старого плоского enum'а. Seam: `src/rule/mod.rs`.
- **[L1-§3.5] делегирование схемы.** `nebula-schema` исполняет правила полей через этот
  крейт; `resolve_field_policies` (`src/policy/mod.rs:218`) — **единственная** точка входа
  для `nebula-schema::validate` (visibility/required), вердикты типизированы.
- **ADR-0052 P2 — единственный эмиттер `required`.** Required-ошибки эмитит только validator
  (через policy-движок), а не каждый слой по отдельности.
- **Wire-format заморожен.** `Rule` сериализуется externally-tagged tuple-compact;
  смена кодировки ломает сохранённые правила. Коды ошибок заморожены fixtures
  (`tests/fixtures/compat/error_registry_v1.json`), есть адверсариальные contract-тесты.

## 6. Известные напряжения / долг

1. **Дубль `ValidationError`.** Свой `ValidationError` (`src/foundation/error/validation_error.rs:103`)
   дублирует канонический `nebula-error::ValidationError`. Унификация **сделана** на ветке
   `refactor/error-unify-validation`, но **не смёржена** — в `main` крейт всё ещё определяет
   собственный тип, а `nebula-error` нужен только ради `Classify` (`src/error.rs:28`).
2. **Deprecated макросы живы.** `compose!` (`src/macros.rs:684`) и `any_of!`
   (`src/macros.rs:715`) ещё существуют; тесты явно глушат deprecation-warning
   (`macros.rs:868-884`) — кандидаты на удаление.
3. **Док-ложь про `Cached`.** `combinators/mod.rs:14,57` описывает комбинатор
   `Cached`/`cached()`, которого нет в exports (`mod.rs:93-110`).
4. **Три prelude.** `crate::prelude`, `foundation::prelude` (:84) и `combinators::prelude`
   (:127) — расползание поверхности импорта.
5. **Wire/коды как обязательство совместимости.** Externally-tagged tuple-compact + замороженный
   `error_registry_v1.json` означают, что любая правка сериализации/кодов — breaking для
   сохранённых правил.
6. **`#![allow(clippy::result_large_err)]` на весь крейт** (`src/lib.rs:50`) — осознанно,
   из-за 80-байтовой ошибки по значению.

## 7. Роль в пост-0092 credential/resource модели

Крейт **не является артефактом** consolidation'а ADR-0092: у него нет deps на
`nebula-credential`/`nebula-crypto`/`nebula-resource`, и эти крейты не зависят от него
напрямую. Его участие в новой модели — **косвенное, через `nebula-schema`** и неизменное:

- **Write-path credential (ADR-0052 P4).** Объединённый `nebula-credential` (контракт +
  runtime + `CredentialService` facade + builtin-типы в одном крейте) валидирует `data`
  **перед** persist. Этот вызов идёт `nebula-credential` → `nebula-schema::validate` →
  `nebula-validator`. Тем самым validator остаётся **единственным эмиттером `required`** (P2)
  и для credential-данных тоже — это шов, который redesign не двигает.
- **Values-only persistence + HasSchema.** В пост-0092 модели слоты (`slot_bindings`) и
  параметры разделены, а сами значения хранятся без схемы: схема восстанавливается из
  зарегистрированных типов через `HasSchema → nebula-metadata → API catalog`. Validator
  исполняет правила этой восстановленной схемы — то есть **исполнительная сторона того же
  seam'а**, без знания о credential/resource топологии (`OwnerScopedKey`, lease, rotation
  fan-out в `nebula-resource`, `RefreshTransport`-шов — всё это вне validator).
- **Что остаётся.** `Rule` cross-kind safety, proof-token `Validated<T>`, policy-движок
  visibility/required и wire-format `Rule` — всё это уже стабильно и переживает redesign
  без изменений. `nebula-resource` `SlotCell`/`Manager`/topology с validator не
  пересекаются.
- **Что меняется.** Только топология крейтов: при коллапсе за sole-public `nebula-sdk`
  validator становится **приватной impl-деталью** (sdk уже зависит от него,
  `crates/sdk/Cargo.toml:24`) — внешнего semver-обязательства у его API больше нет, что
  снимает часть давления с пунктов 1–5 §6 (унификация ошибок, удаление deprecated-макросов,
  схлопывание prelude'ов можно делать как внутренние breaking-правки).

## 8. Forward design / открытые вопросы

- **Смержить унификацию ошибок.** Завести `refactor/error-unify-validation` в `main`: убрать
  собственный `ValidationError` в пользу `nebula-error::ValidationError`. Это снимает
  единственную причину тянуть `nebula-error` (сейчас только ради `Classify`) и закрывает
  напряжение №1. Риск: затрагивает RFC 6901 пути и contract-fixtures — нужна сверка
  `error_registry_v1.json`.
- **Удалить deprecated `compose!`/`any_of!`.** Под прикрытием «validator = приватная
  impl-деталь за sdk» это уже не breaking для внешних — выпилить вместе с глушением
  warning'ов в тестах (`macros.rs:868-884`).
- **Починить док-ложь про `Cached`.** Либо реализовать `cached()` комбинатор, либо вычистить
  упоминания из `combinators/mod.rs:14,57`. Сейчас это прямое расхождение doc vs exports.
- **Сократить prelude'ы до одного.** Свести `foundation::prelude`/`combinators::prelude` к
  `crate::prelude`, чтобы убрать расползание импортов.
- **Решить судьбу wire-format до durable-роста.** Externally-tagged tuple-compact +
  замороженные коды — обязательство перед сохранёнными правилами; любую эволюцию `Rule`
  планировать как версионированную миграцию (по аналогии с versioned-envelope в
  `nebula-crypto`), а не молчаливую смену кодировки.
- **Открытый вопрос.** Нужно ли validator знать о `PredicateContext`-расширениях под
  credential-data (например, cross-field правила поверх восстановленной из `HasSchema`
  схемы), или это полностью остаётся ответственностью `nebula-schema`? Развязать до того,
  как credential write-path начнёт декларировать нетривиальные cross-field `Rule`.
