//! CPU profiling benchmark for nebula-config using pprof
//!
//! Generates text-based profiling reports (flamegraph, report)
//! without requiring root/perf — uses ITIMER_PROF signal sampling.
//!
//! Run: cargo bench -p nebula-config --bench config_profile -- --profile-time=5

use criterion::{Criterion, criterion_group, criterion_main};
use pprof::criterion::{PProfProfiler, Output};
use serde_json::json;
use std::hint::black_box;

fn bench_config_build_profile(c: &mut Criterion) {
    let defaults = json!({
        "app": { "name": "bench", "port": 8080, "debug": false },
        "database": { "host": "localhost", "port": 5432, "name": "db", "ssl": true },
        "cache": { "ttl": 300, "max_size": 1024 },
        "features": { "a": true, "b": false, "c": true },
        "logging": { "level": "info", "format": "json" },
        "server": {
            "workers": 4, "timeout": 30, "keep_alive": 60, "max_connections": 1000
        }
    });

    c.bench_function("profile/build_large", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = defaults.clone();
        b.to_async(&rt).iter(|| {
            let d = d.clone();
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
}

fn bench_config_lookup_profile(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = rt.block_on(async {
        nebula_config::ConfigBuilder::new()
            .with_defaults(json!({
                "app": { "name": "bench", "port": 8080 },
                "database": {
                    "host": "localhost",
                    "port": 5432,
                    "connection": { "pool_size": 10, "timeout": 5000 }
                }
            }))
            .build()
            .await
            .unwrap()
    });

    c.bench_function("profile/lookup_deep", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            async move {
                let val: i64 = cfg.get("database.connection.pool_size").await.unwrap();
                black_box(val);
            }
        });
    });
}

fn bench_config_merge_profile(c: &mut Criterion) {
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
        for i in 0..50 {
            map.insert(format!("key_{i}"), json!({"nested": i, "data": format!("val_{i}")}));
        }
        serde_json::Value::Object(map)
    };

    c.bench_function("profile/merge_50_nested", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| {
            let cfg = &config;
            let ov = overlay.clone();
            async move {
                cfg.merge(ov).await.unwrap();
            }
        });
    });
}

fn bench_config_parse_profile(c: &mut Criterion) {
    let yaml_str = r#"
server:
  port: 8080
  host: "localhost"
  workers: 4
  timeouts:
    read: 30
    write: 60
    idle: 120
database:
  primary:
    host: "db-primary.example.com"
    port: 5432
    pool_size: 20
  replica:
    host: "db-replica.example.com"
    port: 5432
    pool_size: 10
features:
  - logging
  - metrics
  - tracing
debug: false
"#;

    c.bench_function("profile/parse_yaml_large", |b| {
        b.iter(|| {
            let val = nebula_config::utils::parse_config_string(
                black_box(yaml_str),
                nebula_config::ConfigFormat::Yaml,
            )
            .unwrap();
            black_box(val);
        });
    });
}

criterion_group! {
    name = profiled;
    config = Criterion::default()
        .with_profiler(PProfProfiler::new(1000, Output::Flamegraph(None)));
    targets =
        bench_config_build_profile,
        bench_config_lookup_profile,
        bench_config_merge_profile,
        bench_config_parse_profile,
}
criterion_main!(profiled);
