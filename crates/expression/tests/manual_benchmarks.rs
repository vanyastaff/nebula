// Manual benchmarks (workaround for Rust 1.90 Windows bug)
// Run with: cargo test --release manual_benchmarks -- --ignored --nocapture --test-threads=1

use nebula_expression::{EvaluationContext, ExpressionEngine, Template};
use serde_json::Value;
use std::time::Instant;

const ITERATIONS: usize = 1000;

fn avg_time<F: FnMut()>(mut f: F) -> std::time::Duration {
    // Warmup
    for _ in 0..100 {
        f();
    }

    // Measure
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        f();
    }
    start.elapsed() / ITERATIONS as u32
}

#[test]
#[ignore]
fn benchmark_baseline() {
    println!("\n{:=^60}", " BASELINE BENCHMARKS ");
    println!("{:40} {:>15}", "Benchmark", "Avg Time");
    println!("{:-^60}", "");

    // ================================
    // Template Benchmarks
    // ================================
    println!("\n{} Template Benchmarks", "📝");

    let time = avg_time(|| {
        let _ = Template::new("Hello {{ $input }}!");
    });
    println!("{:40} {:>15.2?}", "template/parse/simple", time);

    let time = avg_time(|| {
        let _ = Template::new("{{ $a }} + {{ $b }} = {{ $a + $b }}");
    });
    println!("{:40} {:>15.2?}", "template/parse/multiple", time);

    let complex = r#"
        <html>
            <title>{{ $workflow.name }}</title>
            <body>
                <h1>{{ $execution.id }}</h1>
                <p>Result: {{ $input | uppercase() }}</p>
            </body>
        </html>
    "#;
    let time = avg_time(|| {
        let _ = Template::new(complex);
    });
    println!("{:40} {:>15.2?}", "template/parse/complex", time);

    let engine = ExpressionEngine::new();
    let mut ctx = EvaluationContext::new();
    ctx.set_input(Value::text("World"));
    let tmpl = Template::new("Hello {{ $input }}!").unwrap();

    let time = avg_time(|| {
        let _ = tmpl.render(&engine, &ctx);
    });
    println!("{:40} {:>15.2?}", "template/render/simple", time);

    let time = avg_time(|| {
        let _ = tmpl.clone();
    });
    println!("{:40} {:>15.2?}", "template/clone", time);

    // ================================
    // Engine Benchmarks
    // ================================
    println!("\n{} Engine Benchmarks", "⚙️");

    let ctx2 = EvaluationContext::new();

    let time = avg_time(|| {
        let _ = engine.evaluate("42", &ctx2);
    });
    println!("{:40} {:>15.2?}", "engine/eval/literal", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("2 + 3 * 4", &ctx2);
    });
    println!("{:40} {:>15.2?}", "engine/eval/arithmetic", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("10 > 5", &ctx2);
    });
    println!("{:40} {:>15.2?}", "engine/eval/comparison", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("uppercase('hello')", &ctx2);
    });
    println!("{:40} {:>15.2?}", "engine/eval/function", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("abs(min(-5, -10)) * 2", &ctx2);
    });
    println!("{:40} {:>15.2?}", "engine/eval/nested", time);

    // With cache
    let cached_engine = ExpressionEngine::with_cache_size(1000);
    let expr = "2 + 3 * 4";
    let _ = cached_engine.evaluate(expr, &ctx2); // Warm cache

    let time = avg_time(|| {
        let _ = cached_engine.evaluate(expr, &ctx2);
    });
    println!("{:40} {:>15.2?}", "engine/eval_cached/hit", time);

    // ================================
    // Context Benchmarks
    // ================================
    println!("\n{} Context Benchmarks", "📦");

    let mut big_ctx = EvaluationContext::new();
    for i in 0..100 {
        big_ctx.set_execution_var(format!("var_{}", i), Value::integer(i as i64));
    }

    let time = avg_time(|| {
        let _ = big_ctx.clone();
    });
    println!("{:40} {:>15.2?}", "context/clone_100_vars", time);

    let time = avg_time(|| {
        let _ = big_ctx.get_execution_var("var_50");
    });
    println!("{:40} {:>15.2?}", "context/lookup", time);

    // ================================
    // Concurrent Benchmarks
    // ================================
    println!("\n{} Concurrent Benchmarks", "🔀");

    use std::sync::Arc;
    use std::thread;

    let arc_engine = Arc::new(ExpressionEngine::with_cache_size(1000));
    let expr = "2 + 2";

    // Warm cache
    let _ = arc_engine.evaluate(expr, &ctx2);

    let time = avg_time(|| {
        let _ = arc_engine.evaluate(expr, &ctx2);
    });
    println!("{:40} {:>15.2?}", "concurrent/1_thread", time);

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let eng = Arc::clone(&arc_engine);
                thread::spawn(move || {
                    let c = EvaluationContext::new();
                    for _ in 0..10 {
                        let _ = eng.evaluate(expr, &c);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
    let time_2t = start.elapsed() / ITERATIONS as u32;
    println!("{:40} {:>15.2?}", "concurrent/2_threads", time_2t);

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let eng = Arc::clone(&arc_engine);
                thread::spawn(move || {
                    let c = EvaluationContext::new();
                    for _ in 0..10 {
                        let _ = eng.evaluate(expr, &c);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
    let time_8t = start.elapsed() / ITERATIONS as u32;
    println!("{:40} {:>15.2?}", "concurrent/8_threads", time_8t);

    // Throughput
    let start = Instant::now();
    for _ in 0..10000 {
        let _ = arc_engine.evaluate(expr, &ctx2);
    }
    let ops_per_sec = (10000.0 / start.elapsed().as_secs_f64()) as u64;
    println!("{:40} {:>12} ops/sec", "concurrent/throughput", ops_per_sec);

    // ================================
    // Builtin Benchmarks
    // ================================
    println!("\n{} Builtin Benchmarks", "🔧");

    let time = avg_time(|| {
        let _ = engine.evaluate("uppercase('hello world')", &ctx2);
    });
    println!("{:40} {:>15.2?}", "builtin/string/uppercase", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("length('hello world')", &ctx2);
    });
    println!("{:40} {:>15.2?}", "builtin/string/length", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("abs(-42)", &ctx2);
    });
    println!("{:40} {:>15.2?}", "builtin/math/abs", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("max(1, 2, 3, 4, 5)", &ctx2);
    });
    println!("{:40} {:>15.2?}", "builtin/math/max", time);

    let time = avg_time(|| {
        let _ = engine.evaluate("to_string(42)", &ctx2);
    });
    println!("{:40} {:>15.2?}", "builtin/conversion/to_string", time);

    println!("\n{:=^60}\n", "");
}
