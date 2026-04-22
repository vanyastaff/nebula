---
title: nebula-schema — execution plan (Phases 2–4 closeout)
status: draft
created: 2026-04-22
updated: 2026-04-22
preferred_execution: three-pull-request-stack
alternate_execution: one-pull-request-mega-diff-high-risk
canonical_roadmap: nebula-schema-roadmap.md
pr_stack:
  - nebula-schema-pr1-phase2-gap.md
  - nebula-schema-pr2-phase3-security.md
  - nebula-schema-pr3-phase4-json-schema-plus-docs.md
related_specs:
  - ../superpowers/specs/2026-04-16-nebula-schema-phase2-dx-layer-design.md
  - ../superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md
  - ../superpowers/specs/2026-04-16-nebula-schema-phase4-advanced-design.md
---

# nebula-schema — закрытие Ph2–Ph4 (план исполнения)

## Tech-lead override (2026-04-22) — вставить в kickoff / описание серии PR

1. **Стек из трёх PR, а не мега-PR.** Каноничный путь исполнения — отдельные документы: [PR-1](nebula-schema-pr1-phase2-gap.md) → [PR-2](nebula-schema-pr2-phase3-security.md) → [PR-3](nebula-schema-pr3-phase4-json-schema-plus-docs.md). Между PR: зелёные `clippy -D`, `nextest`, `deny` на `main` после merge предыдущего.

2. **Phase 4 в этой серии — максимум JSON Schema export (C1).** Остальное из Ph4 (реальный AST в build, infer, diff, i18n crate, `validate_async`) — **отдельные issues** с владельцем, пока нет потребителя «заблокирован сегодня».

3. **Зафиксировать API до кода.** Предлагаемая форма struct-level (проверить по факту с `nebula-validator` / deferred hooks):  
   `#[schema(custom(validate = "path::to::validate_fn"))]`  
   Колбэк в духе `fn(&serde_json::Value, &RuleContext) -> Result<(), ValidationError>` — **без** нового параллельного трейта валидации без повторного одобрения tech-lead.

4. **Serde defaults — только compile-time.** `#[serde(default = ...)]` / const-fn, **без** «допатчить JSON после deserialize».

5. **`SecretValue` в `nebula-schema`, граница с `nebula-credential` — через ADR.** Security-lead review **merge-blocking** на PR-2. Perf: `cargo bench -p nebula-schema` до/после PR-1 и PR-2, в описании PR — дельта, порог **~5%** на `bench_validate` / `bench_resolve` или обоснование.

6. **`Vec<Enum>`:** не раздувать derive без нужды — **явная ошибка** + дока/пример ручного `SelectField::extend_options(..)` (или текущий эквивалент в API).

**Ревьюеры:** security-lead (PR-2), rust-senior (макрос `#[schema(custom)]`).

---

## Каноничный путь: три PR

| PR | Документ | Кратко |
|----|----------|--------|
| **PR-1** | [nebula-schema-pr1-phase2-gap.md](nebula-schema-pr1-phase2-gap.md) | Остаток Phase 2 (только schema + macros) |
| **PR-2** | [nebula-schema-pr2-phase3-security.md](nebula-schema-pr2-phase3-security.md) | Phase 3 security + ADR + security-lead |
| **PR-3** | [nebula-schema-pr3-phase4-json-schema-plus-docs.md](nebula-schema-pr3-phase4-json-schema-plus-docs.md) | Phase 4 = JSON Schema (`schemars`) + CHANGELOG/MATURITY/roadmap |

Общий обзор «что уже сделано / что дальше» — в [nebula-schema-roadmap.md](nebula-schema-roadmap.md). Нормативные спеки — `docs/superpowers/specs/2026-04-16-nebula-schema-phase*-design.md`.

---

## Альтернатива: один мега-PR (не рекомендуется)

Один PR на **A+B+C+D** допустим только как осознанный компромисс по срокам, с тем же порядком шагов и **тем же scope cut для Ph4**, что и в PR-3. Риски: огромный diff, долгий review, сложный bisect, конфликты с параллельной работой.

Ниже — **тот же порядок работ**, что и в трёх PR, но объединённый в одну ветку (логические коммиты A→B→C→D обязательны).

### Шаг A — Phase 2 gap (schema + macros)

| # | Задача | Основные пути | Тесты |
|---|--------|---------------|--------|
| A1 | Struct-level `#[schema(...)]` в `#[derive(Schema)]` | `crates/schema/macros/src/attrs.rs`, `derive_schema.rs`, `lib.rs` | compile_fail + позитивный derive |
| A2 | Serde default alignment для `#[param(default)]` | `derive_schema.rs`, при необходимости helper в `crates/schema/src/` | integration: `{}` → значения по умолчанию |
| A3 | `tests/flow/derive_roundtrip.rs` | `crates/schema/tests/flow/` | validate/resolve в рамках API |
| A4 | `Vec<Enum>` — явная ошибка + дока / пример | макросы + docs | trybuild |
| A5 | Doctest «builder + derive» в `lib.rs` | `crates/schema/src/lib.rs` | `cargo test -p nebula-schema --doc` |

**Верификация шага A:**  
`cargo test -p nebula-schema`, `cargo test -p nebula-schema --test compile_fail`, `cargo clippy -p nebula-schema -p nebula-schema-macros -- -D warnings`, см. perf-правило tech-lead выше.

### Шаг B — Phase 3 (security)

| # | Задача | Основные пути | Тесты |
|---|--------|---------------|--------|
| B1 | Workspace deps (`zeroize`, при необходимости KDF, `tracing`) | root `Cargo.toml`, `crates/schema/Cargo.toml` | `cargo deny check` |
| B2 | `SecretValue` / redaction / expose API | `crates/schema/src/secret.rs`, `value.rs`, `validated.rs` | unit + serde redaction |
| B3 | Resolve-time путь + опциональный KDF | `validated.rs`, `field.rs` | integration |
| B4 | Миграции `nebula-credential` по спеке | `crates/credential/**` | `cargo test -p nebula-credential` |

**Верификация шага B:** шаг A + credential tests/clippy + **security-lead** + benches.

### Шаг C — Phase 4 (только согласованный поднабор)

Как в PR-3: **в мега-PR включаем только C1 (JSON Schema / `schemars`)**, если вы следуете рекомендации tech-lead. Остальные пункты Ph4 — таблица для follow-up issue, не для этого merge.

### Шаг D — Документация и метаданные

- `CHANGELOG.md`
- `docs/MATURITY.md`
- `docs/plans/nebula-schema-roadmap.md`
- `crates/schema/README.md`, ADR при изменении L2

## Единая финальная верификация (перед merge любого варианта)

```bash
cargo +nightly fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
cargo deny check
```

---

**Итог:** исполняйте **стек из трёх PR** по ссылкам вверху. Раздел «один мега-PR» оставлен только как запасной сценарий с теми же гейтами и урезанием Ph4.
