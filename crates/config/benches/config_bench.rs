use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde_json::json;
use std::hint::black_box;

fn bench_config_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/build");

    let small = json!({"app": {"name": "bench", "port": 8080}});
    let large = json!({
        "app": {"name": "bench", "port": 8080, "debug": false},
        "database": {"host": "localhost", "port": 5432, "name": "db", "ssl": true},
        "cache": {"ttl": 300, "max_size": 1024},
        "features": {"a": true, "b": false, "c": true},
        "logging": {"level": "info", "format": "json"},
        "server": {"workers": 4, "timeout": 30, "keep_alive": 60, "max_connections": 1000}
    });

    for (name, defaults) in [("small", small), ("large", large)] {
        group.bench_function(name, |b| {
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
    group.finish();
}

fn bench_key_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/lookup");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = rt.block_on(async {
        nebula_config::ConfigBuilder::new()
            .with_defaults(json!({
                "app": {"name": "bench", "port": 8080, "debug": false},
                "database": {"host": "localhost", "port": 5432,
                    "connection": {"pool_size": 10, "timeout": 5000}}
            }))
            .build().await.unwrap()
    });

    group.bench_function("shallow_string", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| { let cfg = &config; async move {
            black_box(cfg.get::<String>("app.name").await.unwrap());
        }});
    });
    group.bench_function("shallow_u16", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| { let cfg = &config; async move {
            black_box(cfg.get::<u16>("app.port").await.unwrap());
        }});
    });
    group.bench_function("deep_i64", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| { let cfg = &config; async move {
            black_box(cfg.get::<i64>("database.connection.pool_size").await.unwrap());
        }});
    });
    group.bench_function("get_value", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| { let cfg = &config; async move {
            black_box(cfg.get_value("database.connection").await.unwrap());
        }});
    });
    group.bench_function("missing_key", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| { let cfg = &config; async move {
            let _ = black_box(cfg.get::<String>("nonexistent.key").await);
        }});
    });
    group.finish();
}

fn bench_merge(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/merge");
    for &size in &[5, 20, 50] {
        group.bench_with_input(BenchmarkId::new("keys", size), &size, |b, &size| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let config = rt.block_on(async {
                nebula_config::ConfigBuilder::new()
                    .with_defaults(json!({"base": "value"}))
                    .build().await.unwrap()
            });
            let overlay: serde_json::Value = {
                let mut map = serde_json::Map::new();
                for i in 0..size { map.insert(format!("key_{i}"), json!(i)); }
                serde_json::Value::Object(map)
            };
            b.to_async(&rt).iter(|| { let cfg = &config; let ov = overlay.clone();
                async move { cfg.merge(ov).await.unwrap(); }
            });
        });
    }
    group.finish();
}

fn bench_flatten(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/flatten");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = rt.block_on(async {
        nebula_config::ConfigBuilder::new()
            .with_defaults(json!({
                "a": {"b": {"c": 1, "d": 2}, "e": 3},
                "f": {"g": {"h": {"i": 4}}},
                "j": 5, "k": [1, 2, 3]
            }))
            .build().await.unwrap()
    });
    group.bench_function("nested", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.to_async(&rt).iter(|| { let cfg = &config; async move {
            black_box(cfg.flatten().await);
        }});
    });
    group.finish();
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("config/parse");
    let json_s = r#"{"server":{"port":8080,"host":"localhost"},"debug":true}"#;
    #[cfg(feature = "toml")]
    let toml_s = "[server]\nport = 8080\nhost = \"localhost\"\ndebug = true\n";
    #[cfg(feature = "yaml")]
    let yaml_s = "server:\n  port: 8080\n  host: localhost\ndebug: true\n";

    group.bench_function("json", |b| {
        b.iter(|| black_box(
            nebula_config::utils::parse_config_string(json_s, nebula_config::ConfigFormat::Json).unwrap()
        ));
    });
    #[cfg(feature = "toml")]
    group.bench_function("toml", |b| {
        b.iter(|| black_box(
            nebula_config::utils::parse_config_string(toml_s, nebula_config::ConfigFormat::Toml).unwrap()
        ));
    });
    #[cfg(feature = "yaml")]
    group.bench_function("yaml", |b| {
        b.iter(|| black_box(
            nebula_config::utils::parse_config_string(yaml_s, nebula_config::ConfigFormat::Yaml).unwrap()
        ));
    });
    group.finish();
}

criterion_group!(benches, bench_config_build, bench_key_lookup, bench_merge, bench_flatten, bench_parse);
criterion_main!(benches);
