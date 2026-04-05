//! Benchmarks for nebula-config
//!
//! Measures:
//! - Config building from defaults
//! - Key lookups (typed and untyped)
//! - Nested key resolution
//! - Config merging
//! - Config flattening
//! - Format parsing (JSON, TOML, YAML)

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde_json::json;
use std::hint::black_box;

// ---------------------------------------------------------------------------
// Config build
// ---------------------------------------------------------------------------

fn bench_config_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/build");

    let small_defaults = json!({
        "app": { "name": "bench", "port": 8080 }
    });

    let large_defaults = json!({
        "app": { "name": "bench", "port": 8080, "debug": false },
        "database": { "host": "localhost", "port": 5432, "name": "db", "ssl": true },
        "cache": { "ttl": 300, "max_size": 1024 },
        "features": { "a": true, "b": false, "c": true },
        "logging": { "level": "info", "format": "json" },
        "server": {
            "workers": 4,
            "timeout": 30,
            "keep_alive": 60,
            "max_connections": 1000
        }
    });

    group.bench_function("small_defaults", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let defaults = small_defaults.clone();
        b.to_async(&rt).iter(|| {
            let d = defaults.clone();
            async move {
                let config = nebula_config::ConfigBuilder::new()
                    .with_defaults(d)
                    .build()
                    .await
                    .unwrap();
                black_box(config);
            }
        });
    });

    group.bench_function("large_defaults", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let defaults = large_defaults.clone();
        b.to_async(&rt).iter(|| {
            let d = defaults.clone();
            async move {
                let config = nebula_config::ConfigBuilder::new()
                    .with_defaults(d)
                    .build()
                    .await
                    .unwrap();
                black_box(config);
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Key lookup
// ---------------------------------------------------------------------------

fn bench_key_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/lookup");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = rt.block_on(async {
        nebula_config::ConfigBuilder::new()
            .with_defaults(json!({
                "app": {
                    "name": "bench",
                    "port": 8080,
                    "debug": false
                },
                "database": {
                    "host": "localhost",
                    "port": 5432,
                    "connection": {
                        "pool_size": 10,
                        "timeout": 5000
                    }
                }
            }))
            .build()
            .await
            .unwrap()
    });

    group.bench_function("shallow_typed_string", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let val: String = cfg.get("app.name").await.unwrap();
                black_box(val);
            }
        });
    });

    group.bench_function("shallow_typed_u16", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let val: u16 = cfg.get("app.port").await.unwrap();
                black_box(val);
            }
        });
    });

    group.bench_function("deep_typed_i64", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let val: i64 = cfg.get("database.connection.pool_size").await.unwrap();
                black_box(val);
            }
        });
    });

    group.bench_function("get_value_untyped", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let val = cfg.get_value("database.connection").await.unwrap();
                black_box(val);
            }
        });
    });

    group.bench_function("missing_key", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let val = cfg.get::<String>("nonexistent.key").await;
                let _ = black_box(val);
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

fn bench_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/merge");

    for &size in &[5, 20, 50] {
        group.bench_with_input(BenchmarkId::new("keys", size), &size, |b, &size| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let config = rt.block_on(async {
                nebula_config::ConfigBuilder::new()
                    .with_defaults(json!({"base": "value"}))
                    .build()
                    .await
                    .unwrap()
            });

            let overlay: serde_json::Value = {
                let mut map = serde_json::Map::new();
                for i in 0..size {
                    map.insert(format!("key_{i}"), json!(i));
                }
                serde_json::Value::Object(map)
            };

            b.to_async(&rt).iter(|| {
                let cfg = &config;
                let ov = overlay.clone();
                async move {
                    cfg.merge(ov).await.unwrap();
                }
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Flatten
// ---------------------------------------------------------------------------

fn bench_flatten(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/flatten");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = rt.block_on(async {
        nebula_config::ConfigBuilder::new()
            .with_defaults(json!({
                "a": { "b": { "c": 1, "d": 2 }, "e": 3 },
                "f": { "g": { "h": { "i": 4 } } },
                "j": 5,
                "k": [1, 2, 3]
            }))
            .build()
            .await
            .unwrap()
    });

    group.bench_function("nested_structure", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let flat = cfg.flatten().await;
                black_box(flat);
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Format parsing
// ---------------------------------------------------------------------------

fn bench_parse_formats(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/parse");

    let json_str = r#"{"server":{"port":8080,"host":"localhost"},"debug":true}"#;
    let toml_str = r#"
[server]
port = 8080
host = "localhost"
debug = true
"#;
    let yaml_str = r#"
server:
  port: 8080
  host: "localhost"
debug: true
"#;

    group.bench_function("json", |b| {
        b.iter(|| {
            let val = nebula_config::utils::parse_config_string(
                black_box(json_str),
                nebula_config::ConfigFormat::Json,
            )
            .unwrap();
            black_box(val);
        });
    });

    group.bench_function("toml", |b| {
        b.iter(|| {
            let val = nebula_config::utils::parse_config_string(
                black_box(toml_str),
                nebula_config::ConfigFormat::Toml,
            )
            .unwrap();
            black_box(val);
        });
    });

    group.bench_function("yaml", |b| {
        b.iter(|| {
            let val = nebula_config::utils::parse_config_string(
                black_box(yaml_str),
                nebula_config::ConfigFormat::Yaml,
            )
            .unwrap();
            black_box(val);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_config_build,
    bench_key_lookup,
    bench_merge,
    bench_flatten,
    bench_parse_formats,
);
criterion_main!(benches);
