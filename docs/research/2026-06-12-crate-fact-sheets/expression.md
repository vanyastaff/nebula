# nebula-expression — fact sheet

## Назначение
Выражательный движок (n8n-совместимый синтаксис) для динамического разрешения полей workflow:
парсит и вычисляет `{{ expression }}`-шаблоны против execution-контекста (`$node`, `$execution`,
`$workflow`, `$input`), возвращает `serde_json::Value`. Бэкенд резолва для `nebula-schema`
(`ValidValues::resolve`, Canon §3.5). Maturity: `stable`.

## Публичная поверхность
- `ExpressionEngine` — engine.rs:154; `new`/`with_cache_size(s)`/`with_policy` (169–256), `evaluate` (287), `parse_template` (323), `render_template` (347), `cache_overview` (433)
- `CacheOverview` — engine.rs:32 (+ `CacheStats` engine.rs:23); LRU AST+template кэш на moka
- `EvaluationContext` — context.rs:22; `EvaluationContextBuilder` — context.rs:204
- `EvaluationPolicy` — policy.rs:10 (DoS-бюджет: step limit + recursion depth, дефолт 256)
- `Template` — template.rs:87; `MaybeTemplate` — template.rs:381; `TemplatePart`/`Position` (doc-hidden) — template.rs:25/50; whitespace-control `{{- -}}`
- `MaybeExpression<T>` — maybe.rs:88 (литерал-или-выражение serde-обёртка; `resolve_as_value/string/integer/float/bool` maybe.rs:203–280); `CachedExpression` — maybe.rs:27
- `ExpressionError` — error.rs:14 (thiserror + `nebula_error::Classify`, коды `EXPR:*`); `ExpressionResult` — error.rs:220; `ExpressionErrorExt` — error.rs:227
- `parse_expression(source)` — lib.rs:123: стабильный parse-only entrypoint; диспетчер template-vs-raw авторитетен через парсер шаблона, не substring
- `BuiltinFunction` (type alias) — builtins.rs:32; `BuiltinRegistry` — builtins.rs:37
- `BuiltinView<'_>` — eval.rs:141: policy-only handle для builtin'ов (type-enforced запрет рекурсии в eval, фикс обхода step-budget из issue #252)
- `Evaluator` — eval.rs:182 (doc-hidden); higher-order комбинаторы (`filter`/`map`/`reduce`/`group_by`/…) живут в eval.rs и идут через `eval_with_frame` с фреймом вызывающего
- `ErrorFormatter` — error_formatter.rs:28; `format_template_error` — error_formatter.rs:183
- value_utils.rs — pub-хелперы коэрции (`is_truthy`:48, `to_integer`:73, `char_count`:106 и др.)
- Re-export `serde_json::Value` — lib.rs:103; `prelude` — lib.rs:148
- doc-hidden, но pub: `ast` (Expr/BinaryOp), `lexer`, `parser`, `token`, `span`, `interner` — «advanced use, may change»

## Workspace-зависимости
Deps (Cargo.toml): nebula-log (path), nebula-error (workspace, feature `derive`), tracing, thiserror,
serde, serde_json, chrono, parking_lot, unicode-width; опциональные: moka (`cache`), regex (`regex`),
chrono-tz (`datetime`), uuid (`uuid`). default = cache,regex,datetime,uuid; `regex` тянет moka намеренно
(true-LRU кэш regex, ROADMAP #590).
Кто зависит: nebula-engine (crates/engine/Cargo.toml:29), nebula-schema (crates/schema/Cargo.toml:25),
nebula-action (crates/action/Cargo.toml:57, default-features=false, только `cache`),
nebula-resource (crates/resource/Cargo.toml:26, default-features=false, только `cache`),
examples (examples/Cargo.toml:20), nebula-expression-fuzz (crates/expression/fuzz, features=full).

## Структура модулей
- lib.rs — re-exports, `parse_expression`, prelude
- lexer.rs / token.rs / parser.rs / ast.rs / span.rs — фронтенд: токенизация → AST (+ позиции)
- eval.rs — AST-walker `Evaluator`/`EvalFrame`, `BuiltinView`, higher-order комбинаторы
- engine.rs — `ExpressionEngine`, два moka-LRU кэша (expr+template), статистика
- context.rs — `EvaluationContext` + builder (4 пространства переменных)
- policy.rs — `EvaluationPolicy` (DoS-бюджет)
- template.rs — `Template`/`MaybeTemplate`/`TemplatePart`, whitespace control
- maybe.rs — `MaybeExpression<T>`, `CachedExpression`
- builtins.rs + builtins/{array,string,math,object,datetime,conversion,util}.rs — реестр builtin-функций
- error.rs / error_formatter.rs — типизированные ошибки + красивый рендер с span
- interner.rs — `StringInterner` (doc-hidden)
- value_utils.rs — коэрция/предикаты над `Value`
- benches/baseline.rs (criterion), fuzz/ (отдельный крейт, 3 fuzz-таргета), tests/builtin_functions.rs

## Напряжения
- Doc-drift: README.md:33 и AGENTS.md:15 называют метод `evaluate_template(tmpl, ctx)` — в коде его нет, есть `render_template` (engine.rs:347).
- datetime.rs:101 — комментарий «legacy 2-arg shape»: эвристика разбора 3-го аргумента (tz vs format) с fallback; единственное упоминание legacy в крейте.
- Широкая doc-hidden pub-поверхность (lexer/parser/eval/ast/token/span/interner) — полупубличный API «may change»; под sole-public-sdk (только nebula-sdk публичен) можно сделать честно private.
- Ветка `refactor/error-unify-validation` (unmerged) трогает expression resolve seam (sync/single-parse + `From<ExpressionError>` на стороне потребителей) — в этом worktree не отражена.
- TODO/FIXME/deprecated: отсутствуют; явных дублей внутри крейта не найдено.

## Роль в credential/resource redesign
Напрямую не затронут: ни один credential-крейт от него не зависит, rewrite-планы его не упоминают.
Косвенно — потребитель nebula-resource (и nebula-action) использует его с урезанными фичами
(default-features=false, `cache`) для резолва `MaybeExpression`-конфигов; при коллапсе крейтов за
nebula-sdk его публичность станет внутренней деталью (re-export решает sdk).
