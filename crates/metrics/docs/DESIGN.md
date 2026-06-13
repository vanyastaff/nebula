# nebula-metrics — design

| Field | Value |
|-------|-------|
| **Status** | Stable — observability foundation (leaf/core) |
| **Layer** | Cross-cutting (depends only on `nebula-error` + `nebula-eventbus`) |
| **Redesign role** | **Not touched structurally** — passive *name registry* for the post-0092 credential/resource model. Hot-swap/rotation работы пишут существующие `NEBULA_*` метрики; примитивы и экспортёры redesign не меняет. |
| **Related** | [ADR-0046](../../../docs/adr/0046-metrics-telemetry-boundary.md) (поглощение `nebula-telemetry`), [ADR-0092](../../../docs/adr/0092-credential-subsystem-consolidation.md) (как consumer имён) |

---

## 1. Назначение и границы

`nebula-metrics` — единый observability-крейт workspace: in-memory метрик-примитивы,
интернинг лейблов, стандартизованные имена `nebula_*`, cardinality-guard и два экспортёра.

**Владеет:** lock-free примитивы `Counter` / `Gauge` / `Histogram` на атомиках
(`#![forbid(unsafe_code)]`), конкурентный `MetricsRegistry` (DashMap), интернинг лейблов
поверх `lasso`, policy-слой имён (`naming`) и фильтр кардинальности (`LabelAllowlist`),
а также экспорт в Prometheus text-format и OTLP push. После ADR-0046 границы бывшего
`nebula-telemetry` стали модульными внутри этого крейта (`otlp.rs` — единственное место
типов OTel SDK).

**ЯВНО НЕ делает:** не хранит ничего durable (только in-memory, со staleness-вытеснением
через `retain_recent`); не хостит HTTP-эндпоинт `/metrics` (это `nebula-api`); не задаёт
доменную семантику подсистем — только держит их имена-константы; не enforce-ит имена линтом
(дисциплина ручная, см. §6).

## 2. Публичная поверхность

| Item | Where |
|------|-------|
| `MetricsRegistry` + `counter/gauge/histogram(_labeled)` | `src/registry.rs:105,145-215` |
| `MetricsRegistry::snapshot_counters/gauges/histograms` (seam обоих экспортёров) | `src/registry.rs:286-308` |
| `MetricsRegistry::retain_recent(max_age)` / `metric_count` / `interner_len` | `src/registry.rs:344-360` |
| `Counter` — `inc/inc_by/get/last_updated_ms` | `src/counter.rs:16` |
| `Gauge` — `inc/dec/set/get` | `src/gauge.rs:18` |
| `Histogram` + `HistogramSnapshot` — `observe/snapshot/percentile/try_with_buckets` | `src/histogram.rs:102,38` |
| `LabelInterner` / `LabelSet` / `MetricKey`; `LabelKey`/`LabelValue` = `lasso::Spur` | `src/labels.rs:138,59,288,35-38` |
| `LabelAllowlist` — `all()/only()/apply()` (cardinality-guard) | `src/filter.rs:41` |
| `naming` — ~80 pub const `NEBULA_*` + вложенные модули label-значений | `src/naming.rs` |
| `record_eventbus_stats(&MetricsRegistry, &EventBusStats)` → 4 гейджа | `src/eventbus.rs:44` |
| `snapshot(&MetricsRegistry) -> String` / `content_type()` / `PrometheusExporter` | `src/prometheus.rs:367,481,489` |
| `OtlpMetricsConfig` / `OtlpMetricsExporter::install{,_with_exporter}` / `OtlpMetricsGuard::shutdown` / `OtlpInitError` | `src/otlp.rs:86,224,237,158,144` |
| `MetricsError` / `MetricKind` / `MetricsResult` (thiserror + `Classify`) | `src/error.rs:18,6,70` |
| `prelude` — re-export БЕЗ otlp-типов и `LabelValue` | `src/prelude.rs` |

## 3. Зависимости и зависимые

- **Deps:** `nebula-error` (только derive `Classify`), `nebula-eventbus`; внешние —
  `dashmap`, `lasso`, `thiserror`, `tracing`, `opentelemetry{,-otlp,_sdk}`,
  `tokio` (time/rt/macros — для OTLP push-loop).
- **Dependents:** `nebula-engine` (`crates/engine/Cargo.toml:40`),
  `nebula-resource` (`crates/resource/Cargo.toml:27`),
  `nebula-credential` (`crates/credential/Cargo.toml:42`),
  `nebula-api` (`crates/api/Cargo.toml:26` — хостит `/metrics`).

## 4. Внутренняя архитектура

- `counter.rs` / `gauge.rs` / `histogram.rs` — lock-free примитивы на атомиках.
- `registry.rs` — DashMap-реестр, снапшот-seam, `retain_recent` staleness-вытеснение.
- `labels.rs` — lasso-interner, `LabelSet`, композитный `MetricKey`.
- `naming.rs` — policy-слой: все `NEBULA_*` имена (workflow / engine / action / api-auth /
  webhook / resource / eventbus / credential / cache).
- `filter.rs` — policy: `LabelAllowlist` (cardinality-guard).
- `prometheus.rs` — text-format экспорт (`# HELP` / `# TYPE`, per-bucket).
- `otlp.rs` — единственное место типов OTel SDK (ADR-0046 seam), push-экспортёр.
- `eventbus.rs` — инструментация `EventBusStats` → гейджи `NEBULA_EVENTBUS_*`.
- `error.rs` — `MetricsError`; `prelude.rs` — удобный импорт.

**Поток данных:** подсистема пишет в примитив через `MetricsRegistry` под именем-константой
из `naming` → значения копятся на атомиках → экспортёр (Prometheus или OTLP) читает их
через единый снапшот-seam (`snapshot_*`); `LabelAllowlist` отсекает high-cardinality лейблы
на записи, `retain_recent` вытесняет устаревшие серии.

## 5. Инварианты и контракты

- **Единый снапшот-seam.** Оба экспортёра читают состояние только через
  `snapshot_counters/gauges/histograms` (`registry.rs:286-308`) — экспортёр не имеет
  альтернативного доступа к внутренним структурам, формат экспорта изолирован by-construction.
- **OTel изолирован.** Типы OTel SDK живут только в `otlp.rs` (ADR-0046 seam) — остальной
  крейт от OTLP не зависит, что и позволяет prelude отдавать примитивы без OTel-поверхности.
- **Cardinality-guard на записи.** `LabelAllowlist::apply()` (`filter.rs:41`) ограничивает
  множество лейблов до взрыва кардинальности в реестре, не на экспорте.
- **Staleness-вытеснение.** `retain_recent(max_age)` (`registry.rs:344`) держит реестр
  ограниченным по времени; `Counter::last_updated_ms` даёт точку отсечения.
- **`forbid(unsafe_code)`.** Примитивы lock-free, но без `unsafe` — корректность на атомиках.

## 6. Известные напряжения / долг

1. **README отстал от кода.** `README.md:76-77` и `README.md:86` утверждают «Not an OTLP
   exporter… OTLP is planned», но `src/otlp.rs` полностью реализован и re-export в
   `lib.rs:60`; `AGENTS.md:23` сам фиксирует расхождение (README last-reviewed 2026-05-06).
2. **Глоб `pub use naming::*` (`lib.rs:59`).** ~80 констант + вложенные label-модули лежат
   плоско в корне крейта; рост `naming.rs` (791 строка) раздувает корневую поверхность.
3. **Асимметрия prelude.** `prelude.rs:7-14` не включает otlp-типы и `LabelValue` —
   расходится с плоским экспортом `lib.rs`.
4. **`naming.rs` как односторонняя свалка имён** всех подсистем (api auth, webhook,
   credential coordinator…); enforcement имён ручной (`README.md:87-88`: «no lint, drift
   possible»).
5. **TODO/FIXME/deprecated/shims** — нет.

## 7. Роль в пост-0092 credential/resource модели

Затронут **пассивно, как реестр имён**. Метрики, которые пишут `nebula-resource` и
`nebula-credential` в ходе hot-swap/rotation работ, уже присутствуют как `NEBULA_*`
константы: resource lifecycle + rotation/revoke dispatch + recycle outcomes
(`naming.rs:488-583`) и credential rotations + refresh-coordinator
claims/coalesced/sentinel/reclaim/hold + resolver reauth CAS (`naming.rs:634-777`).
Код самого крейта (примитивы / реестр / экспорт) redesign **не меняет** — правки сводятся
к добавлению/удалению `NEBULA_*` констант в policy-секции `naming.rs`. Это стабильный
фундамент: новые метрики per-slot rotation fan-out (nebula-resource) и
RefreshTransport/lease (nebula-credential) попадают сюда только именами.

## 8. Forward design / открытые вопросы

Крейт стабилен; структурных задач нет. Точечные открытые вопросы:

- **Структурировать `naming`** под namespaced-модули вместо глоб-реэкспорта (§6.2/§6.4),
  чтобы остановить рост корневой поверхности и снять ручной drift имён — кандидат на лёгкий
  не-breaking рефактор корня.
- **Выровнять prelude** с плоским экспортом или сознательно задокументировать асимметрию
  (§6.3).
- **Освежить README** под фактическую OTLP-реализацию (§6.1) — документационный долг, не код.
