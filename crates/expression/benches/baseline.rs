// Baseline benchmarks for nebula-expression
// Run with: cargo bench --bench baseline

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use nebula_expression::{EvaluationContext, ExpressionEngine, Template};
use serde_json::Value;
use std::sync::Arc;
use std::thread;

// ================================
// Template Benchmarks
// ================================

fn benchmark_template_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("template/parse");

    // Simple template
    group.bench_function("simple", |b| {
        b.iter(|| Template::new(black_box("Hello {{ $input }}!")))
    });

    // Multiple expressions
    group.bench_function("multiple_expressions", |b| {
        b.iter(|| Template::new(black_box("{{ $a }} + {{ $b }} = {{ $a + $b }}")))
    });

    // Complex template
    group.bench_function("complex", |b| {
        let template = r#"
            <html>
                <title>{{ $workflow.name }}</title>
                <body>
                    <h1>{{ $execution.id }}</h1>
                    <p>Result: {{ $input | uppercase() }}</p>
                    <span>{{ $node.data.count * 2 }}</span>
                </body>
            </html>
        "#;
        b.iter(|| Template::new(black_box(template)))
    });

    group.finish();
}

fn benchmark_template_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("template/render");

    let engine = ExpressionEngine::new();
    let mut context = EvaluationContext::new();
    context.set_input(Value::String("World".to_string()));

    let simple = Template::new("Hello {{ $input }}!").unwrap();
    let complex = Template::new(
        r#"
        <html>
            <title>{{ $input | uppercase() }}</title>
            <p>Length: {{ length($input) }}</p>
        </html>
    "#,
    )
    .unwrap();

    group.bench_function("simple", |b| {
        b.iter(|| simple.render(black_box(&engine), black_box(&context)))
    });

    group.bench_function("complex", |b| {
        b.iter(|| complex.render(black_box(&engine), black_box(&context)))
    });

    group.finish();
}

fn benchmark_template_clone(c: &mut Criterion) {
    let template = Template::new("Hello {{ $input }}!").unwrap();

    c.bench_function("template/clone", |b| b.iter(|| black_box(template.clone())));
}

// ================================
// Engine Benchmarks
// ================================

fn benchmark_evaluate_no_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine/evaluate_no_cache");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let test_cases = vec![
        ("literal", "42"),
        ("arithmetic", "2 + 3 * 4"),
        ("comparison", "10 > 5"),
        ("string_concat", r#""hello" + " " + "world""#),
        ("function_call", "uppercase('hello')"),
        ("nested", "abs(min(-5, -10)) * 2"),
        ("conditional", "if true then 1 else 2"),
    ];

    for (name, expr) in test_cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, expr| {
            b.iter(|| engine.evaluate(black_box(expr), black_box(&context)))
        });
    }

    group.finish();
}

fn benchmark_evaluate_with_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine/evaluate_with_cache");

    let engine = ExpressionEngine::with_cache_size(1000);
    let context = EvaluationContext::new();

    let expr = "2 + 3 * 4";

    // Warm up cache
    let _ = engine.evaluate(expr, &context);

    group.bench_function("cache_hit", |b| {
        b.iter(|| engine.evaluate(black_box(expr), black_box(&context)))
    });

    // Cache miss
    group.bench_function("cache_miss", |b| {
        let mut counter = 0;
        b.iter(|| {
            counter += 1;
            let expr = format!("{} + {}", counter, counter + 1);
            engine.evaluate(black_box(&expr), black_box(&context))
        })
    });

    group.finish();
}

// ================================
// Context Benchmarks
// ================================

fn benchmark_context_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("context/operations");

    // Create context with many variables
    let mut context = EvaluationContext::new();
    for i in 0..100 {
        context.set_execution_var(format!("var_{}", i), Value::Number((i as i64).into()));
    }

    // Clone benchmark
    group.bench_function("clone_100_vars", |b| b.iter(|| black_box(context.clone())));

    // Lookup benchmark
    group.bench_function("lookup", |b| {
        b.iter(|| context.get_execution_var(black_box("var_50")))
    });

    group.finish();
}

// ================================
// Concurrent Benchmarks
// ================================

fn benchmark_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent/access");
    group.throughput(Throughput::Elements(1));

    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let expr = "2 + 2";

    // Warm up cache
    {
        let context = EvaluationContext::new();
        let _ = engine.evaluate(expr, &context);
    }

    // Single thread baseline
    group.bench_function("1_thread", |b| {
        let context = EvaluationContext::new();
        b.iter(|| engine.evaluate(black_box(expr), black_box(&context)))
    });

    // Multi-threaded
    for num_threads in [2, 4, 8] {
        group.bench_function(format!("{}_threads", num_threads), |b| {
            b.iter(|| {
                let handles: Vec<_> = (0..num_threads)
                    .map(|_| {
                        let engine = Arc::clone(&engine);
                        thread::spawn(move || {
                            let context = EvaluationContext::new();
                            for _ in 0..10 {
                                let _ = engine.evaluate(expr, &context);
                            }
                        })
                    })
                    .collect();

                for handle in handles {
                    handle.join().unwrap();
                }
            })
        });
    }

    group.finish();
}

fn benchmark_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent/throughput");
    group.throughput(Throughput::Elements(1));

    let engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let context = EvaluationContext::new();

    group.bench_function("ops_per_sec", |b| {
        b.iter(|| engine.evaluate(black_box("2 + 2"), black_box(&context)))
    });

    group.finish();
}

// ================================
// Builtin Functions
// ================================

fn benchmark_builtins(c: &mut Criterion) {
    let mut group = c.benchmark_group("builtins");

    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();

    let test_cases = vec![
        ("string/uppercase", "uppercase('hello world')"),
        ("string/length", "length('hello world')"),
        ("math/abs", "abs(-42)"),
        ("math/max", "max(1, 2, 3, 4, 5)"),
        ("array/first", "first([1, 2, 3, 4, 5])"),
        ("array/join", "join([1, 2, 3, 4, 5], ', ')"),
        ("array/concat", "concat([1, 2], [3, 4], [5, 6])"),
        ("array/flatten", "flatten([[1, 2], [3, 4], [5, 6]])"),
        ("object/keys", "keys({a: 1, b: 2, c: 3})"),
        ("object/values", "values({a: 1, b: 2, c: 3})"),
        ("conversion/to_string", "to_string(42)"),
    ];

    for (name, expr) in test_cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, expr| {
            b.iter(|| engine.evaluate(black_box(expr), black_box(&context)))
        });
    }

    group.finish();
}

// ================================
// Criterion Groups
// ================================

criterion_group!(
    template_benches,
    benchmark_template_parse,
    benchmark_template_render,
    benchmark_template_clone
);

criterion_group!(
    engine_benches,
    benchmark_evaluate_no_cache,
    benchmark_evaluate_with_cache
);

criterion_group!(context_benches, benchmark_context_operations);

criterion_group!(
    concurrent_benches,
    benchmark_concurrent_access,
    benchmark_throughput
);

criterion_group!(builtin_benches, benchmark_builtins);

criterion_main!(
    template_benches,
    engine_benches,
    context_benches,
    concurrent_benches,
    builtin_benches
);
