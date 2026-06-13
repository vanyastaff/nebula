# nebula-expression — design

| Field | Value |
|-------|-------|
| **Status** | Stable — leaf/core evaluation primitive |
| **Layer** | Core (зависит только от `nebula-log` + `nebula-error`; ни одного домен-крейта) |
| **Redesign role** | **Не затронут** post-ADR-0092 credential/resource переделкой — ни один credential-крейт от него не зависит, rewrite-планы его не упоминают. Косвенный потребитель: `nebula-resource`/`nebula-action` резолвят `MaybeExpression`-конфиги. |
| **Related** | PRODUCT_CANON §3.5 (`ValidValues::resolve`), ROADMAP #590 (regex-кэш), issue #252 (step-budget bypass fix), n8n expression-синтаксис |

---

## 1. Назначение и границы

`nebula-expression` — выражательный движок с n8n-совместимым синтаксисом для динамического
разрешения полей workflow. Парсит и вычисляет `{{ expression }}`-шаблоны против
execution-контекста и возвращает `serde_json::Value`. Служит бэкендом резолва для
`nebula-schema` (`ValidValues::resolve`, Canon §3.5).

**Владеет:** лексером/парсером/AST выражений, AST-walk вычислителем, реестром builtin-функций
(`array`/`string`/`math`/`object`/`datetime`/`conversion`/`util`), template-движком с
whitespace-control (`{{- -}}`), двумя LRU-кэшами (AST + template) на moka, DoS-бюджетом
(`EvaluationPolicy`), serde-обёртками литерал-или-выражение (`MaybeExpression<T>`,
`MaybeTemplate`) и типизированными ошибками с span-рендерингом.

**ЯВНО НЕ делает:** не хранит execution-state (контекст строит вызывающий), не знает про
credential/resource домен, не делает KDF/crypto, не валидирует схему (это `nebula-schema`,
которая лишь *вызывает* резолв), не выполняет I/O — вычисление чистое над переданным контекстом.

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `ExpressionEngine` (+ `new`/`with_cache_size`/`with_policy`) | `engine.rs:154` (169–256) |
| `evaluate` / `parse_template` / `render_template` / `cache_overview` | `engine.rs:287 / 323 / 347 / 433` |
| `CacheOverview` (+ `CacheStats`) | `engine.rs:32` (`:23`) |
| `EvaluationContext` (+ `EvaluationContextBuilder`) — `$node`/`$execution`/`$workflow`/`$input` | `context.rs:22` (`:204`) |
| `EvaluationPolicy` (DoS-бюджет: step limit + recursion depth, дефолт 256) | `policy.rs:10` |
| `Template` / `MaybeTemplate` (whitespace-control `{{- -}}`) | `template.rs:87 / 381` |
| `MaybeExpression<T>` (+ `resolve_as_value/string/integer/float/bool`); `CachedExpression` | `maybe.rs:88` (203–280); `:27` |
| `ExpressionError` (thiserror + `nebula_error::Classify`, коды `EXPR:*`); `ExpressionResult`; `ExpressionErrorExt` | `error.rs:14 / 220 / 227` |
| `parse_expression(source)` — стабильный parse-only entrypoint (template-vs-raw через парсер, не substring) | `lib.rs:123` |
| `BuiltinFunction` (alias); `BuiltinRegistry` | `builtins.rs:32 / 37` |
| `BuiltinView<'_>` — policy-only handle для builtin'ов (type-enforced запрет рекурсии в eval) | `eval.rs:141` |
| `ErrorFormatter` / `format_template_error` | `error_formatter.rs:28 / 183` |
| `value_utils` — pub-хелперы коэрции (`is_truthy:48`, `to_integer:73`, `char_count:106`, …) | `value_utils.rs` |
| Re-export `serde_json::Value`; `prelude` | `lib.rs:103 / 148` |

doc-hidden, но pub: `ast` (`Expr`/`BinaryOp`), `lexer`, `parser`, `token`, `span`, `interner`,
`Evaluator` (`eval.rs:182`) — помечены «advanced use, may change».

## 3. Зависимости и зависимые

- **Deps:** `nebula-log` (path), `nebula-error` (workspace, feature `derive`), `tracing`,
  `thiserror`, `serde`, `serde_json`, `chrono`, `parking_lot`, `unicode-width`. Опциональные:
  `moka` (`cache`), `regex` (`regex`, намеренно тянет moka — true-LRU regex-кэш, ROADMAP #590),
  `chrono-tz` (`datetime`), `uuid` (`uuid`). default = `cache,regex,datetime,uuid`.
- **Зависимые:** `nebula-engine`, `nebula-schema`, `nebula-action` (default-features=false,
  только `cache`), `nebula-resource` (default-features=false, только `cache`), `examples`,
  `nebula-expression-fuzz` (features=full).

## 4. Внутренняя архитектура

Фронтенд: `lexer.rs`/`token.rs` → `parser.rs` → `ast.rs` (+ `span.rs` для позиций),
`interner.rs` дедуплицирует идентификаторы. `eval.rs` — AST-walker `Evaluator`/`EvalFrame`;
higher-order комбинаторы (`filter`/`map`/`reduce`/`group_by`/…) идут через `eval_with_frame`
с фреймом вызывающего, builtin'ы получают только `BuiltinView` (без доступа к рекурсивному
eval). `engine.rs` оркестрирует два moka-LRU кэша (expr-AST + template) и статистику.
`context.rs` несёт 4 пространства переменных. `template.rs` склеивает literal/expr-части
с whitespace-control. `maybe.rs` — serde-слой литерал-или-выражение для конфигов.
`error.rs`/`error_formatter.rs` — типизированные ошибки + красивый span-рендер.
Поток: source → (`parse_expression`/`parse_template`) AST/`Template` → cache → `evaluate`
под `EvaluationPolicy` → `Value`.

## 5. Инварианты и контракты

- **Резолв-бэкенд Canon §3.5.** `ValidValues::resolve` в `nebula-schema` вызывает движок;
  выход всегда `serde_json::Value`.
- **DoS-бюджет by-construction.** `EvaluationPolicy` ограничивает шаги и глубину рекурсии
  (дефолт 256); бюджет общий на всё вычисление.
- **Step-budget нельзя обойти из builtin'а (issue #252).** Builtin'ы получают `BuiltinView`,
  а не `Evaluator` — тип запрещает рекурсивный вызов в обход счётчика шагов. Higher-order
  комбинаторы рекурсируют только через `eval_with_frame` под тем же бюджетом.
- **Диспетчер template-vs-raw авторитетен.** `parse_expression` решает «шаблон или сырое
  выражение» через парсер шаблона, не по substring `{{`.
- **Типизированные ошибки.** `ExpressionError` несёт `nebula_error::Classify` с кодами `EXPR:*`.

## 6. Известные напряжения / долг

1. **Doc-drift в имени метода.** `README.md:33` и `AGENTS.md:15` называют `evaluate_template(tmpl, ctx)`,
   которого в коде нет — фактический метод `render_template` (`engine.rs:347`). Чистая doc-правка.
2. **Legacy-эвристика datetime.** `datetime.rs:101` — «legacy 2-arg shape»: разбор 3-го
   аргумента (tz vs format) эвристикой с fallback; единственное упоминание legacy в крейте.
3. **Широкая doc-hidden pub-поверхность.** `lexer`/`parser`/`eval`/`ast`/`token`/`span`/`interner`
   — полупубличный «may change» API. Под sole-public-sdk (публичен только `nebula-sdk`) это можно
   честно перевести в `pub(crate)`.
4. **Unmerged resolve-seam рефактор.** Ветка `refactor/error-unify-validation` трогает
   expression resolve seam (sync/single-parse + `From<ExpressionError>` на стороне потребителей);
   в этом worktree не отражена.
5. TODO/FIXME/deprecated отсутствуют; явных внутрикрейтовых дублей не найдено.

## 7. Роль в пост-0092 credential/resource модели

Не затронут — стабильный фундамент. Ни `nebula-credential`, ни `nebula-resource`,
ни `nebula-storage` не зависят от него по credential-пути, и rewrite-планы (ADR-0088/0092)
его не упоминают. Единственная связь косвенная: consumer binding в `nebula-resource`/`nebula-action`
резолвит `MaybeExpression`-конфиги через урезанный (`default-features=false`, только `cache`)
вариант. При коллапсе крейтов за `nebula-sdk` публичность станет внутренней деталью — re-export
решает sdk.

## 8. Forward design / открытые вопросы

Крейт стабилен; зелёного-поля работы нет. Открытые пункты узкие: (а) свести doc-drift из §6.1;
(б) при переходе на sole-public-sdk сузить doc-hidden pub-модули до `pub(crate)` (§6.3);
(в) подобрать resolve-seam изменения из `refactor/error-unify-validation` при мердже (§6.4).
regex-кэш на moka — уже выбранное направление (ROADMAP #590), не открытый вопрос.
