# nebula-metrics — fact sheet

## Назначение
Единый observability-крейт: in-memory метрик-примитивы (counter/gauge/histogram на атомиках,
`#![forbid(unsafe_code)]`), interning лейблов (lasso), стандартные имена `nebula_*`, cardinality-guard
и два экспортера — Prometheus text-format и OTLP push (ADR-0046: бывший `nebula-telemetry` поглощён,
границы теперь модульные внутри крейта).

## Публичная поверхность
- `MetricsRegistry` — конкурентный реестр; `counter/gauge/histogram(_labeled)` (src/registry.rs:105,145-215)
- `MetricsRegistry::snapshot_counters/gauges/histograms` — seam для обоих экспортеров (src/registry.rs:286-308)
- `MetricsRegistry::retain_recent(max_age)`, `metric_count`, `interner_len` (src/registry.rs:344-360)
- `Counter` — `inc/inc_by/get/last_updated_ms` (src/counter.rs:16)
- `Gauge` — `inc/dec/set/get` (src/gauge.rs:18)
- `Histogram` + `HistogramSnapshot` — `observe/snapshot/percentile/try_with_buckets` (src/histogram.rs:102,38)
- `LabelInterner` / `LabelSet` / `MetricKey`; `LabelKey`/`LabelValue` = `lasso::Spur` (src/labels.rs:138,59,288,35-38)
- `LabelAllowlist` — `all()/only()/apply()` cardinality-guard (src/filter.rs:41)
- `naming` — ~80 pub const `NEBULA_*` + вложенные модули label-значений (src/naming.rs)
- `record_eventbus_stats(&MetricsRegistry, &EventBusStats)` — 4 гейджа `NEBULA_EVENTBUS_*` (src/eventbus.rs:44)
- `snapshot(&MetricsRegistry) -> String`, `content_type()`, `PrometheusExporter` (src/prometheus.rs:367,481,489)
- `OtlpMetricsConfig` (builder), `OtlpMetricsExporter::install/install_with_exporter`, `OtlpMetricsGuard::shutdown`, `OtlpInitError` (src/otlp.rs:86,224,237,158,144)
- `MetricsError` / `MetricKind` / `MetricsResult` — thiserror + `nebula_error::Classify` (src/error.rs:18,6,70)
- `prelude` — re-exports БЕЗ otlp-типов и `LabelValue` (src/prelude.rs)

## Workspace-зависимости
- Deps: `nebula-error` (только derive `Classify`), `nebula-eventbus`; внешние: dashmap, lasso, thiserror,
  tracing, opentelemetry{,-otlp,_sdk}, tokio (time/rt/macros — для OTLP push loop).
- Зависят от него: `nebula-engine` (crates/engine/Cargo.toml:40), `nebula-resource` (crates/resource/Cargo.toml:27),
  `nebula-credential` (crates/credential/Cargo.toml:42), `nebula-api` (crates/api/Cargo.toml:26; хостит `/metrics`).

## Структура модулей
- `counter.rs` / `gauge.rs` / `histogram.rs` — lock-free примитивы на атомиках
- `registry.rs` — DashMap-реестр, снапшот-seam, `retain_recent` staleness-вытеснение
- `labels.rs` — lasso-interner, LabelSet, композитный MetricKey
- `naming.rs` — policy: все `NEBULA_*` константы (workflow/engine/action/api-auth/webhook/resource/eventbus/credential/cache)
- `filter.rs` — policy: `LabelAllowlist`
- `prometheus.rs` — text-format экспорт (# HELP/# TYPE, per-bucket)
- `otlp.rs` — ЕДИНСТВЕННОЕ место OTel SDK типов (ADR-0046 seam), push-экспортер
- `eventbus.rs` — инструментация EventBusStats → гейджи
- `error.rs` — `MetricsError`; `prelude.rs` — удобный импорт

## Напряжения
- README vs код: README.md:76-77 «Not an OTLP exporter… OTLP is planned» и README.md:86 «OTLP is planned» —
  но src/otlp.rs полностью реализован и re-export в lib.rs:60. AGENTS.md:23 сам фиксирует это расхождение. README устарел (last-reviewed 2026-05-06).
- `pub use naming::*` глоб (lib.rs:59) — ~80 констант + вложенные label-модули плоско в корне крейта; рост naming.rs (791 строка) раздувает корневую поверхность.
- prelude.rs не включает otlp-типы и `LabelValue` — асимметрия с плоским экспортом lib.rs (src/prelude.rs:7-14).
- naming.rs — однонаправленная свалка имён всех подсистем (api auth, webhook, credential coordinator…); enforcement имён ручной (README.md:87-88: «no lint, drift possible»).
- TODO/FIXME/deprecated/shims — нет.

## Роль в credential/resource redesign
Затронут пассивно, как реестр имён: naming.rs:488-583 (resource lifecycle + rotation/revoke dispatch,
recycle outcomes) и naming.rs:634-777 (credential rotations, refresh-coordinator claims/coalesced/sentinel/
reclaim/hold, resolver reauth CAS) — это метрики, которые пишут nebula-resource/nebula-credential в ходе
hot-swap/rotation работ. Сам код крейта (примитивы/экспорт) redesign не меняет; правки сводятся к
добавлению/удалению `NEBULA_*` констант в policy-секции.
