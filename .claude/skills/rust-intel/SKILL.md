---
name: rust-intel
description: Hard rules for writing Rust that LLMs systematically get wrong. Load this BEFORE writing any Rust code. Defends against the full known taxonomy of LLM failure modes in Rust as of 2026.
---

# Rust Intel — Defense Against LLM Failure Modes

Based on (a) a published 6-month field report on LLM-generated Rust in production (~80k LOC, tokio + sqlx + unsafe hot paths, May 2026 — see [`docs/sources.md`](docs/sources.md)), (b) academic benchmarks RustEvo², SafeTrans, CRUST-Bench, SafeGenBench, Rust-SWE-Bench, AkiraRust, and (c) the empirical error distribution observed across Claude/GPT/Cursor through 2025–2026, plus real supply-chain incidents (CrateDepression 2022, `faster_log`/`async_println` 2025). The rules below cover **twenty-six categories** of bugs ranging from mass compilation failures (E0277/E0308/E0599) to subtle runtime failures that pass compilation, pass tests, and only manifest in production under load — including supply-chain risks (slopsquatting with documented attack cases), cryptographic insecurity, check-then-act races, backpressure leaks, and advanced async pitfalls (AFIT, Pin, Waker, block_on). Citations and URLs for every empirical claim live in [`docs/sources.md`](docs/sources.md). They are non-negotiable for any Rust I write.

Industry signal: per Faros AI and Lightrun studies (2026), AI-generated PRs show +242.7% incident rate and 43% require post-merge debugging. Zero surveyed senior engineers rated themselves "very confident" in AI-generated Rust. This is the empirical context this document defends against.

The categories split into three tiers, plus a meta-layer:
- **Self-monitoring**: a triggers table that maps user-request patterns to risk categories. Scanned before generating code.
- **Tier A — Mass compilation failures (§A1–§A4)**: highest frequency, caught by `rustc` but waste cycles. Address FIRST. Includes supply-chain attacks via crate-name hallucination (with real attack cases).
- **Tier B — Silent correctness bugs (§B1–§B15)**: pass compilation, sometimes pass tests, fail in production. These are the ones that hurt. Includes UB, async pitfalls (basic and advanced), lock ordering, memory leaks, silent task dropping, cryptographic insecurity, TOCTOU races, backpressure neglect, and Mutex poisoning cascades.
- **Tier C — Architecture and ergonomics (§C1–§C7)**: design-level mistakes that are expensive to undo, plus the reflexive `.clone()` pattern, procedural macro hygiene, and Cargo feature flag hygiene.

---

## Principle: prove, don't guess

When writing Rust under this command, my role is a **verifying engineer, not a code-completion engine**. The difference matters for every line:

- I generate code I can justify, not code that looks plausible.
- When I'm uncertain about an API, a lifetime, a trait bound, a Drop contract — I say so and ask, rather than producing something that compiles by luck.
- When the context is insufficient to prove correctness, I refuse to generate code and explicitly block (see "Blocking protocol" below).
- "Compiles" and "tests pass" are necessary but never sufficient. The bugs in this document specifically exist in the gap between those signals and actual correctness.

This principle is what activates every rule below. Without it, the rules become a checklist to game; with it, they become a method for catching mistakes I would otherwise make confidently.

The same principle applies to this document's own empirics: every percentage, rate, and sample-size figure cited in the categories below maps to a sourced entry in [`docs/sources.md`](docs/sources.md). Load that file alongside this one when statistical precision matters for your decision.

---

## Blocking protocol

If at any point I lack the context required to satisfy this command's rules, I do not "best-effort guess". I emit a blocking message in this exact format and stop:

```
⚠️ BLOCKED: <one-line reason — what I cannot verify>
NEEDED:
  - <specific item 1, e.g. "exact versions of tokio and sqlx from Cargo.toml">
  - <specific item 2, e.g. "definition of the `Database` trait this is implementing against">
  - <specific item 3, e.g. "expected behavior on commit failure: retry, propagate, or rollback to checkpoint?">
```

Cases where I block rather than guess:
- Crate versions are unknown for any dependency I'd need to call API on (§A1).
- Trait definitions are missing for a trait I'm asked to implement against (§C1).
- I would need to design a new trait hierarchy from scratch (operating mode rule 3).
- Drop semantics matter for the task and I don't know the library version (§B4).
- Cancel-safety is required and I cannot determine the cancellation context (§B3).
- The user asks for `unsafe` code but the invariants the caller will uphold are unstated (§B5).

A blocking message is not failure. Generating code that compiles, passes tests, and leaks resources in production *is* failure. Blocking is how that failure is prevented.

---

## Operating mode

Whenever this command is loaded, before generating any Rust code I will:

1. **Pin the world.** Read `Cargo.toml` (and `CLAUDE.md` if present) for exact crate versions of `tokio`, `axum`, `sqlx`, `reqwest`, `serde`, `hyper`, `clap`, and any other major dependency. State the assumed versions in a comment block at the top of the response. If versions are unknown and cannot be read, **ask** rather than guess. *RustEvo² shows pass@1 drops from 56.1% to 32.5% on post-cutoff APIs — guessing is the dominant source of API hallucinations.*

2. **Map the project idioms.** If `CLAUDE.md`, `README.md`, or top-level docs declare project conventions (error type, logging crate, runtime, lint level), follow those. Do not introduce a new error-handling style, a new async runtime, or a new logging crate without explicit permission.

3. **Refuse to design trait hierarchies blind.** For any new public trait, propose the signature in plain text first and wait for approval before writing impls. LLMs make strategic mistakes here (object safety, sealed vs open, blanket impls) that are expensive to undo. Drafting is fine; committing is not.

4. **Refuse `unsafe` without `// SAFETY:`.** Every `unsafe` block must be preceded by a `// SAFETY:` comment naming every invariant the operation relies on. No exceptions, including "obvious" cases.

5. **Annotate every `async fn` with cancel-safety.** See §B3. A doc comment line is mandatory: `/// cancel-safe: yes` or `/// cancel-safe: NO — <reason>`.

6. **Show the caller for non-trivial lifetimes.** Any function returning `&T` derived from inputs requires at least one example call site in a comment or test — two consecutive calls with disjoint inputs — before the signature is final. See §B1.

7. **Surface everything risky in the summary.** When work is complete, list every occurrence of: `unsafe`, `unwrap`, `expect`, `transmute`, `Arc<Mutex<_>>`, manual `Send`/`Sync` impl, blanket impl, `panic!`, `unimplemented!`, `todo!`. Line numbers and justification each.

---

# Self-monitoring: prompt triggers that activate failure modes

Before generating code, I scan the user's request for triggers below. If a trigger fires, the linked category is on heightened alert. This is the meta-rule: **knowing why I would make a mistake here is half the defense**.

| User request contains... | Activates category | Specific risk |
|---|---|---|
| "cache", "memoize", "store results" with returned `&T` | §B1 lifetime laundering | One `'a` for input and cache, collapsing lifetimes |
| "shared between threads", "concurrent", "from multiple tasks" | §B2 Mutex across .await; §A3 smart pointer misuse | Default to `std::sync::Mutex`, reflexive `Arc<Mutex<T>>` |
| "with timeout", "select!", "cancel", "race two futures" | §B3 cancel safety | Silent partial state, no cancel-safe annotation |
| "transaction", "rollback", "commit" | §B4 Drop and RAII | Library-specific Drop semantics on commit failure |
| "fast", "zero-copy", "performance", "parse bytes", "from network" | §B5 unsafe UB | `ptr::read` on unaligned buffers |
| "fix this borrow error", "make this compile", "lifetime issue" | §C5 reflexive clone | `.clone()` as silencer of real ownership problem |
| "implement trait for any T", "generic Display", "blanket impl" | §C1 semver hazard | Open blanket impl in public API |
| "buffer of size N" where N is large | §B7 stack overflow | `[u8; N]` by value or `Box::new([0u8; N])` |
| "parse this", "convert from string" | §C2 error handling | `.unwrap()` instead of typed error |
| "use the latest version of X", "modern Y" | §A1 API hallucinations | Memory of pre-cutoff API for fast-evolving crates |
| Code involves crate version 0.x | §A1 pre-1.0 churn | Breaking changes between minor versions |
| "send notification", "fire and forget", "log this event async" | §B8 silent task dropping | Forgotten `.await`, future never polled |
| "lock the X and the Y", "two shared resources", "atomic update across two" | §B9 ABBA deadlock | Locks acquired in opposite orders |
| "tree with parent links", "graph structure", "bidirectional", "scene graph", "DOM-like" | §B10 reference cycles | Symmetric `Rc<RefCell>` without `Weak` |
| "read a file", "make HTTP request", "sleep", "wait N seconds" in async context | §B11 blocking executor | `std::fs`/`std::thread::sleep` in `async fn` |
| "add this dependency", "use crate X for Y", "what crate should I use" | §A1 slopsquatting | Hallucinated crate name → supply-chain attack |
| "encrypt", "decrypt", "hash a password", "JWT", "TLS", "sign this", "AES", "AEAD" | §B12 crypto insecurity | Nonce reuse, weak primitives, hallucinated crypto API |
| "public API", "library", "publish to crates.io", "what should the signature be" | §B1 lifetime leaking; §C1 blanket impls | `'a` in public signatures, semver hazards |
| "lazy cache", "memoize", "compute if absent", "deduplicate concurrent requests", "ensure only once" | §B13 TOCTOU | `contains_key` + `insert` race; should be `entry().or_insert_with` |
| "background worker", "event queue", "log pipeline", "broadcast to subscribers", "producer-consumer" | §B14 unbounded queue | `unbounded_channel` instead of bounded + backpressure policy |
| "trait with async method", "trait Foo { async fn ... }", "trait object" | §B15 AFIT | Missing `+ Send` bound, not spawn-able |
| "implement Future manually", "custom Poll", "wake the task" | §B15 Waker | `Poll::Pending` without registering waker → hang forever |
| "block_on this from a helper", "synchronous wrapper for async" | §B15 nested runtime | `block_on` inside async context → panic |
| "Pin this", "self-referential struct", "Pin::new_unchecked" | §B15 Pin misuse | Unsafe Pin without proving non-movement |
| "procedural macro", "derive macro", "proc-macro2", "syn"/"quote" | §C6 macro hygiene | Bare `Option`/`Result` paths, `panic!` in macro errors |
| "feature flag", "conditional compilation", "cfg attribute" | §C7 feature hygiene | Typo'd feature names silently become dead code |

When two or more triggers fire in one request, treat it as a high-risk task and explicitly enumerate which categories I'm guarding against in my response.

---

# TIER A — Mass compilation failures

These are the bugs that show up in 18–30% of LLM-generated Rust per SafeTrans. They get caught by `rustc`, but only if I avoid the traps proactively.

**Empirical priority justification**: on Rust-SWE-bench, **76.3% of all compilation failures from LLM agents fall into just two categories** — failure to model project organization (43.7%, manifesting as E0433, E0432, E0425, E0412, E0405) and failure to respect Rust's type/trait semantics (32.6%, manifesting as E0599, E0308, E0277, E0407). §A1 and §A2 directly address these two categories. This is why Tier A is addressed first: these are not exotic bugs, they are the dominant failure mode by a wide margin.

## §A1. API hallucinations and stale APIs

**The trap**: Codestral and DeepSeek-Coder generate non-existent methods (`E0599`) in up to 22% of cases. RustEvo² shows that for crates whose API changed after the model's knowledge cutoff, pass-rate drops by ~24 percentage points. Typical compiler signatures of this category: **E0433** (unresolved crate/module/type), **E0432** (unresolved import), **E0425** (unresolved name), **E0412** (type not in scope), **E0405** (trait not in scope). Typical hallucinations: `axum::Server::bind` (removed in 0.7), `serde::Deserialize` without `#[serde(default)]` where required, `sqlx::query!` with the wrong macro signature, methods on `tokio` that exist in 0.2 but not 1.x.

**REQUIRED**:
- Before calling any method on a third-party type, check that it exists in the **exact version pinned in `Cargo.toml`**.
- For crates with high churn (`tokio`, `axum`, `hyper`, `reqwest`, `sqlx`, `serde`, `tonic`, `tower`, `clap`), if I'm uncertain about an API, **say so explicitly** and ask the user to confirm or run `cargo doc --open` rather than guess.
- Do not invent module paths. If `axum::extract::FromRequest` is what I need, I write that. If I'm unsure whether it's `FromRequest` or `FromRequestParts`, I say so.
- Do not invent crate names. `tokio-postgres-pool` does not exist; `deadpool-postgres` does.
- Pre-1.0 crates (any version with leading `0.`) have **breaking changes between minor versions**. Treat 0.6 → 0.7 with the same suspicion as 1.x → 2.x.

**BANNED**:
- Method calls on types where I have not internally verified the method exists in the pinned version.
- Mixing API styles from different major versions (e.g., axum 0.6 routers with axum 0.7 handlers).

**Security note: slopsquatting**. Hallucinated *crate names* (not just methods) are not only a compile error — they are a supply-chain attack vector. Adversaries monitor common LLM crate-name hallucinations and **register those names on crates.io with malicious payloads**. Published "package-import hallucination" studies (Lanyado / Spracklen line of work) report elevated hallucination rates for Rust crate names relative to other ecosystems, attributed to a smaller training corpus — verify the precise figure against the primary source before quoting.

**Real attack cases (2022–2026)** — these are not hypothetical:
- `rustdecimal` — typosquat of `rust_decimal` (the real crate has ~3.5M downloads). The malicious crate, documented in the CrateDepression incident (2022), targeted CI pipelines.
- `faster_log`, `async_println` — malicious crates designed to scan for and exfiltrate Solana/Ethereum private keys; reached thousands of downloads before takedown.
- Supply-chain attacks on Rust ecosystem rose ~130% in 2025 per industry reports.

Concrete defenses:
- I do not add a crate to `Cargo.toml` unless the user explicitly named it OR I verified its existence by reading the project's existing dependencies.
- For any new dependency I suggest, I flag it as a *suggestion to verify*, not a fait accompli: "I'd add `deadpool-postgres` for connection pooling — please verify on crates.io before adding."
- I never invent variations of well-known crate names (`tokio-utils` does not exist, `tokio-util` does; `serde-json` does not exist as a separate crate, `serde_json` does; `rust-decimal` does not exist, `rust_decimal` does — and the typo'd variant has been weaponized).
- Surface every newly-added `Cargo.toml` dependency in the post-flight summary so the user can audit it.

## §A2. Trait bounds and type mismatches (E0277 / E0308)

**The trap**: E0277 ("trait not implemented") and E0308 ("mismatched types") together account for >18% of all errors in LLM-generated Rust per SafeTrans, up to 30% for some models. Related compiler signatures: **E0599** (method not found), **E0407** (method is not a member of trait). The pattern is failing to track conversion chains (`Into<T>`/`From<T>`), missing `Send + 'static` for `tokio::spawn`, and confusion between owned and borrowed forms.

**REQUIRED**:
- For every `tokio::spawn(async move { ... })`, verify the future is `Send + 'static`: no `Rc`, no non-Send guard held across `.await`, no captured `&` reference to local data.
- For every `Box<dyn Trait>` returned from a public function, default to `Box<dyn Trait + Send + Sync + 'static>` unless there is a documented reason not to.
- For every generic function with bounds, prefer **stating bounds explicitly** over relying on inference. `fn foo<T: Serialize + DeserializeOwned + Send + Sync + 'static>(x: T)` is verbose but compiles; `fn foo<T>(x: T) where T: Serialize` will get caught downstream.
- When converting between types, prefer `.into()` only when the target type is unambiguous. If the target is generic, write `T::from(x)` explicitly.
- API surface: prefer `&str` over `String`, `&[T]` over `Vec<T>`, `impl AsRef<Path>` over `&Path` for filesystem APIs.

**BANNED**:
- `Box<dyn Future<Output = T>>` without `+ Send` (won't work with `tokio::spawn`).
- Using `?` across error types where `From` impl is not defined; either define the `From` impl or use `.map_err(...)` explicitly.

## §A3. Smart pointer misuse

**The trap**: LLMs default to `Arc<Mutex<T>>` for "shared state" reflexively, even when a `&mut T` would suffice. The reverse trap: using `Rc<RefCell<T>>` in code that will later cross threads, then having to refactor.

**REQUIRED**:
- `Arc` only when ownership is genuinely shared across threads or async tasks. Single-owner sharing → `&` or `&mut`.
- `Mutex` only when interior mutability is actually needed. Read-only shared data → `Arc<T>` is enough.
- `Rc` and `RefCell` are **forbidden** in any code path that may be called from `tokio::spawn` or any other multi-threaded executor. If unsure → use `Arc` and `Mutex`/`RwLock` from `tokio::sync` or `std::sync` per §B2.
- `Box<T>` for `T: Sized` of small size (≤ 2 × pointer size) is almost always wrong. Don't box `i64`, `Option<u32>`, or small enums.

**BANNED**:
- `Arc<Mutex<T>>` where `T` is only ever read after construction. Use `Arc<T>`.
- `Arc<RwLock<T>>` for write-heavy workloads. Profile first; `Mutex` is often faster.

## §A4. Module visibility and pub leaks

**The trap**: LLMs mark types `pub` reflexively to silence compiler errors, leaking internal types into the public API surface. Once shipped, removing them is a breaking change.

**REQUIRED**:
- New types default to private. Promote to `pub(crate)` only when needed across modules; promote to `pub` only when intended as part of the public API.
- Never re-export types via `pub use` from a public module without confirming they should be part of the public surface.
- For library crates: every `pub` item is a semver commitment. Treat `pub fn` as load-bearing.

---

# TIER B — Silent correctness bugs

These pass `cargo build`, often pass `cargo test`, and fail in production. The fifteen categories below are the ones that hurt.

**Why this tier exists**: high compilation rate is not correctness. The published 2026 field report on ~80k LOC of LLM-generated tokio/sqlx code (see [`docs/sources.md`](docs/sources.md)) shows that **§B2 alone (`Mutex` across `.await`) was responsible for failure in roughly half of async tasks** before defensive prompting cut it sharply; SafeGenBench shows static analyzers miss **~57% of vulnerabilities** in LLM-generated crypto Rust that *does* compile (§B12). The category list below is structured around this gap between `cargo test` green and actual correctness — see [`docs/sources.md`](docs/sources.md) for the full evidence trail.

## §B1. Lifetime laundering and lifetime leaking

Two distinct lifetime traps LLMs make with high frequency. They look similar from the outside (both involve `<'a>` in a signature where it shouldn't be) but the diagnostic and the fix are different. Treat them as separate sub-categories.

### §B1a. Lifetime laundering

**The trap**: one `'a` parameter binds both an input and a cached output, hiding a lifetime collapse from the local view. The signature compiles in isolation but the function becomes uncallable in practice.

**Why this happens**: the transformer's attention doesn't extend beyond the function body. Locally, `<'a>` looks elegant; the cross-function constraint is invisible.

**BANNED pattern (synthetic):**
```rust
fn lookup<'a>(s: &'a str, cache: &mut HashMap<String, &'a str>) -> &'a str { ... }
//                                                       ^^^ caller's `s` lifetime
//                                                       leaks into the cache type
```
Compiles in isolation; collapses to an empty lifetime when called twice with different inputs.

**BANNED pattern (realistic — typical LLM output for "add caching"):**
```rust
use std::collections::HashMap;

fn first_word<'a>(s: &'a str, cache: &mut HashMap<String, &'a str>) -> &'a str {
    if let Some(cached) = cache.get(s) {
        return cached;
    }
    let word = s.split_whitespace().next().unwrap_or("");
    cache.insert(s.to_string(), word);
    word
}
```
Compiles, passes unit tests with a single input. Fails the moment a second call site passes a `&str` with a different lifetime: the cache forces all entries to share one `'a`, which the borrow checker collapses to the empty intersection.

**Prompt triggers that produce this**: "add caching to this function", "memoize", "speed up by storing results", "build a lookup". Whenever the user mentions caching of returned references, this category activates.

**REQUIRED**:
- Separate input and output lifetimes (`<'input, 'cache>`) when they should be independent, OR store owned data (`HashMap<String, String>`).
- For any function returning `&T` derived from inputs, write a comment showing two consecutive calls with disjoint inputs before the signature is final.
- Higher-Ranked Trait Bounds (`for<'a> Fn(&'a T) -> &'a U`) deserve extra care: do not drop `for<'a>` when generalizing.

### §B1b. Lifetime leaking through public APIs

**The trap**: exposing `'a` in a *public* function signature when the lifetime is an implementation detail. The function compiles, the lifetime is genuine, and the signature is technically more "zero-copy" than the alternative — but every downstream caller is now forced to juggle that lifetime through their own code.

**Distinct from §B1a**: laundering is *one `'a` binding too many things inside one function*; leaking is *exposing an `'a` in a `pub` signature that should not have been part of the public API at all*. A function can suffer from leaking without any laundering, and vice versa.

**BANNED in published library APIs unless zero-copy is an explicitly documented design goal**:
```rust
// Forces every caller to track 'a through their own code:
pub fn parse<'a>(source: &'a str) -> Document<'a> { ... }
```

**REQUIRED**:
- Default to owned return types in public APIs: `pub fn parse(source: &str) -> Document { ... }` where `Document` owns its data.
- If zero-copy is a real design requirement, document it explicitly and consider exposing both variants (`parse` returning owned + `parse_borrowed` returning the lifetime-parameterized version) so callers opt in.
- Surface every `pub fn` with a non-`'static` output lifetime in the post-flight summary so the user can confirm the lifetime is intentional, not residual.

## §B2. `std::sync::Mutex` held across `.await`

**The trap**: LLMs default to `std::sync::Mutex` because it dominates training data. Holding it across `.await` violates tokio's contract and can deadlock under load. `clippy::await_holding_lock` catches only ~30% of cases (misses guards hidden in closures, `if let`, early-return blocks). Statistics: in the 2026 field report (~80k LOC), this single category was the proximate cause of failure in roughly half of async tasks; pinning crate versions in the prompt cut it sharply.

**BANNED** in any function annotated `async`, called from `tokio::spawn`, or used in a tokio runtime context:
- `std::sync::Mutex` / `parking_lot::Mutex` whose guard lives across a `.await`.
- `std::sync::RwLock` whose guard lives across a `.await`.
- `RefCell` or `Rc` anywhere reachable from async tasks crossing thread boundaries.

**REQUIRED**:
- For data shared across `.await` points → `tokio::sync::Mutex` / `tokio::sync::RwLock`.
- For data accessed only synchronously inside an async block → `std::sync::Mutex` is fine, but **the guard must be dropped before any `.await`**. Write the drop explicitly:
  ```rust
  let value = {
      let guard = mutex.lock().unwrap();
      guard.get(&key).cloned()
  };  // guard dropped here
  some_async_op(value).await
  ```
- Run `cargo clippy -- -W clippy::await_holding_lock` after writing async code touching locks.

**Related anti-pattern: Mutex poisoning cascade.** When a thread panics while holding a `Mutex`, the Mutex is "poisoned": all subsequent `.lock().unwrap()` calls panic too. LLMs copy `.lock().unwrap()` from std/serde examples without considering poisoning. One unrelated panic in production cascades into every code path that touches that Mutex.

- For non-trivial code, handle poison explicitly:
  ```rust
  let guard = mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
  ```
- Or use `parking_lot::Mutex` (no poisoning by design) if poison-aware recovery is not needed.
- Surface every `.lock().unwrap()` in the post-flight summary.

**Related anti-pattern: oversized critical section.** A `MutexGuard` held across I/O, heavy compute, logging, or any non-trivial operation creates contention even when it doesn't violate any rule. It compiles, tests pass, but production throughput collapses under load.

- The body of a `lock()` block should be: read/write a few fields, clone what's needed, drop the guard. Anything else (I/O, allocation, parsing, logging, format!) goes outside.
- If a critical section grows beyond ~10 lines, it's a candidate for restructuring.

## §B3. Async cancellation (THE BIG ONE)

**The trap**: futures in Rust are cancellable at every `.await` point. Cancel safety is **not visible in any signature**. Borrow checker doesn't help. Clippy doesn't help. Documentation for each tokio function must be read individually (`AsyncReadExt::read` is cancel-safe, `read_exact` is not). In the 2026 field report, **zero** models across the timeout-using benchmark tasks spontaneously mentioned cancel safety; when asked directly, they answered "yes, it's cancel-safe" confidently and incorrectly in ~50% of cases.

**Critical warning about my own reasoning**: in empirical testing, approximately half of LLM-generated assessments of cancel-safety were *confidently wrong* — the model labeled a not-cancel-safe function as "cancel-safe because all `.await` points are idempotent" or similar plausible-sounding justifications. This is a known failure mode: I am especially prone to overconfidence in this area. **When I annotate a function as cancel-safe, I must enumerate every `.await` point and prove cancel safety for each, not assert it.**

**REQUIRED for every async fn I write**:
- A doc comment line: `/// cancel-safe: yes` or `/// cancel-safe: NO — <reason>`.
- If not cancel-safe, justify by listing the await points where partial state would leak (DB write committed but ack not sent, file written but rename not done, etc.).
- If a function performs `db.write` then `network.send_ack`, it is **not cancel-safe**. Do not call it from `tokio::select!` or with `tokio::time::timeout` without wrapping in `tokio::spawn` to detach from the cancellation tree.

**Pattern for the not-cancel-safe boundary**:
```rust
/// cancel-safe: yes (read is cancel-safe, write+ack is detached via spawn)
async fn handle(stream: TcpStream, db: Arc<Db>) -> Result<()> {
    let data = read_message(&stream).await?;  // cancel-safe up to here
    // Critical section detached from caller cancellation:
    tokio::spawn(async move {
        db.insert(&data).await?;
        send_ack(&stream).await?;
        Ok::<_, Error>(())
    }).await?
}
```

**Specifically cancel-UNSAFE in tokio (memorize)**:
- `AsyncReadExt::read_exact`, `read_to_end`, `read_to_string`
- `AsyncWriteExt::write_all`, `write_buf`
- `tokio::io::copy`
- Anything that wraps the above

**BANNED**:
- Calling a function with `db.write().await; send_ack().await` directly under `tokio::select!` or `tokio::time::timeout`.
- Claiming a function is "cancel-safe because all `.await` points are idempotent" without proving each one (idempotence is necessary but not sufficient; you also need atomic recovery from any partial state).
- `stream.next().then(|x| async move { ... .await ... })` — if the inner async block contains any `.await`, the entire chain is not cancel-safe: cancellation between `next()` resolving and the inner await completing loses the item from the stream.

## §B4. Drop order and RAII contracts

**The trap**: implicit `Drop` for transactions, file handles, async resources has library-specific contracts. `sqlx` implicit-rolls-back inside the async runtime (blocking). `deadpool-postgres` sends rollback to a background task that may never run. The semantics live in library source, not signatures.

**REQUIRED** for any DB transaction / file handle / network resource:
- After the last fallible operation that might fail (e.g., `tx.commit().await`), **assume the resource's `Drop` runs in an undefined state**. Do not rely on it for correctness.
- For transactions: explicit `commit().await?` on success path, explicit `rollback().await?` on error path, **and** acknowledge the failure mode of `commit().await` itself failing (the tx is then in a library-specific state — check the docs).
- Read the version-specific `Drop` impl docs for the library being used. State the version you assumed in a comment.
- Be aware that holding multiple drop-significant guards (file + DB tx + lock) creates an ordering problem: Rust drops in reverse declaration order, but the *correct* order depends on the semantics. State which order matters.

## §B5. Unsafe that looks safe (high UB rate in small-N studies)

**The trap**: code passes review and tests because UB doesn't manifest on typical inputs. In the small-N audit cited in [`docs/sources.md`](docs/sources.md), out of 40 LLM-generated `unsafe` blocks: 13 were UB on any input, 9 were UB on specific inputs (alignment, OOB, Stacked Borrows violations), 18 were correct — i.e. 22/40 (~55%) exhibited UB. The *exact rate* is directional (small sample, not stratified by model or domain), but the *pattern* — that LLM-generated `unsafe` is significantly more dangerous than LLM-generated safe code — is consistent across every published audit to date. Treat any LLM-generated `unsafe` block as high-risk until proven otherwise via miri + manual invariant audit.

**BANNED**:
- `std::ptr::read(p)` / `*p` / `&*p` where the source pointer's alignment is not statically known to match `T`. Use `read_unaligned` / `write_unaligned` / `slice::align_to` instead.
- `transmute` between types whose layouts aren't both `#[repr(C)]` or otherwise pinned by documented invariants. `#[repr(Rust)]` is unstable and `transmute` between such types is UB.
- Any `unsafe` block without `// SAFETY:` preceding it that names every invariant.
- Creating a `&mut T` from a `*mut T` while another reference to the same data still exists (Stacked Borrows violation, caught by miri).
- Marking a public function `pub fn` when its contract actually requires invariants from the caller. If the caller must uphold something for safety, it is `pub unsafe fn`.

**REQUIRED**:
- `// SAFETY:` comment listing each invariant in the form `// SAFETY: ptr is valid for reads of size_of::<T>(), is properly aligned (allocated via Layout::new::<T>()), and outlives this borrow.`
- Add miri to CI for files containing `unsafe`: `cargo +nightly miri test`. Yes, 10× slower; the one UB caught pays for it.
- Default to safe abstractions (`zerocopy::FromBytes`, `bytemuck::Pod`/`bytemuck::cast_slice`, `slice::chunks_exact`, `slice::align_to`) before reaching for raw pointers. (`bytes::Bytes` is a refcounted *buffer container*, not a safe-transmute abstraction — don't confuse the two.)
- For FFI: every `extern "C"` function takes/returns `#[repr(C)]` types only. `String` and `Vec` cannot cross the FFI boundary; use `CString` / `*const c_char` / `Vec::into_raw_parts`.

## §B6. Pattern matching exhaustiveness drift

**The trap**: a `match` written today is exhaustive. After someone adds a new enum variant, it may silently become non-exhaustive only in `if let` form, or use a wildcard `_ => ...` that swallows the new case.

**REQUIRED**:
- For every `match` on an enum I do not own: assume the enum is `#[non_exhaustive]` and handle the fallback explicitly with a logged/typed error, not silent ignore.
- For every `match` on an enum I own: avoid wildcard arms unless I want adding-a-variant to compile silently. Use explicit arms.
- For every `if let Some(x) = ...` on a `Result` or option-chain that could grow new "interesting" failure modes, prefer `match` with explicit arms.

**BANNED**:
- `_ => unreachable!()` or `_ => panic!()` for enums where new variants could legitimately be added.
- `_ => Ok(())` swallowing an error case.

## §B7. Large stack allocations and arena pitfalls

**The trap**: `[u8; 1_048_576]` on the stack overflows in debug builds. `Box::new([0u8; N])` constructs on the stack first and may overflow before placement-new optimizations kick in (release-mode dependent, never reliable).

**BANNED**:
- `[T; N]` where `N * size_of::<T>() > 4096` returned by value or wrapped in `Box::new(...)`.
- Recursive functions with large local arrays.

**REQUIRED for heap-allocated buffers**:
- `vec![0u8; N].into_boxed_slice()` — guaranteed heap, stable Rust.
- Or on nightly: `Box::<[u8]>::new_uninit_slice(N)`.
- Or `bytes::BytesMut::zeroed(N)`.

## §B8. Silent task dropping (forgotten `.await`)

**The trap**: an `async fn` call without `.await` returns a `Future` that is never polled — meaning the work *never happens*. Compilation often passes (especially when the future is bound to `let _` or returned from a match arm where its `#[must_use]` is consumed), tests pass (the calling function returned without panicking), but the HTTP request was never sent, the database write never executed, the cache never updated. This is *uniquely silent* because nothing went wrong from the type system's perspective — the code is correct, the work simply wasn't performed.

**Why this happens**: LLMs sometimes generate `client.post(url).send()` instead of `client.post(url).send().await`. The reflex comes from sync-language patterns where calling the function executes it. In async Rust, the future is inert until polled.

**Prompt triggers**: "send a notification", "log this event", "fire and forget", "make an HTTP call after the response", any background-task framing.

**BANNED**:
- `let _ = some_async_fn(...);` — explicitly drops the future without polling.
- Calling an async function and not using the result, with no `.await` or `tokio::spawn`.
- `tokio::spawn(async_fn())` instead of `tokio::spawn(async move { async_fn().await })`. The first creates a future-of-future and spawns the outer one, which completes immediately and drops the inner without polling.

**REQUIRED**:
- Every async function call is followed by `.await`, OR wrapped in `tokio::spawn(async move { ... .await })` for fire-and-forget, OR explicitly stored in a `JoinHandle`/`FuturesUnordered` for later polling.
- For fire-and-forget, **always** use `tokio::spawn` rather than letting the future drop silently.
- Enable `#[warn(unused_must_use)]` at crate level. Verify the `#[must_use]` warning fires for ignored futures in clippy output.
- For functions that return `impl Future`, ensure callers `.await` them — surface uncalled futures in the post-flight summary.

## §B9. Lock ordering and ABBA deadlock

**The trap**: two locks (`Mutex<A>`, `Mutex<B>`) acquired in opposite orders in different code paths. Function `f1` locks A then B; function `f2` locks B then A. Single-threaded tests pass trivially. Multi-threaded production hits the classic deadlock: thread 1 holds A waiting for B, thread 2 holds B waiting for A, both wait forever.

**Why this happens**: LLMs treat lock acquisition as a local concern. The deadlock is a global property of the program's lock graph, invisible from any single function. No lint detects it.

**Prompt triggers**: "synchronize access to two shared resources", "lock the cache and the queue", "update state and metrics atomically", anything involving two `Arc<Mutex<_>>` in the same operation.

**REQUIRED**:
- For any code path that acquires more than one lock, **document the lock acquisition order** as a doc comment at the top of the module or function. State it in a comment LLM-readable enough that future generations of this file maintain it.
- Use a consistent lock ordering across the entire crate. Common conventions: alphabetical by name, by declaration order in the struct, by a numeric rank field.
- Prefer fine-grained immutable data + message passing (`mpsc`, `oneshot`) over multi-lock critical sections when possible.
- When two locks must be held, take them **in the documented order, every time, without exception**.
- For async code, prefer `tokio::sync::Mutex` (which detects some deadlock patterns under `tokio-console`).

**BANNED**:
- Holding two locks across a function call (the called function may acquire locks in another order).
- Acquiring a second lock while holding the first if the second one's acquisition can block on async work or I/O.
- "Just try locking" patterns with `try_lock` to escape suspected deadlocks — that hides the design problem.

**Detection**: add `tokio-console` for runtime visibility, or `parking_lot::deadlock` detection in dev builds. Surface every double-lock site in the post-flight summary.

## §B10. Reference cycles in `Rc`/`Arc` graphs

**The trap**: when LLMs build graph or tree structures with parent-child relationships, they reach for `Rc<RefCell<Node>>` (or `Arc<Mutex<Node>>`) and create *both* parent→child and child→parent strong references. This creates a reference cycle. Rust has no garbage collector. Memory leaks. Tests pass because functionality (insert, traverse, lookup) works correctly. Production hits OOM after days or weeks.

**Why this happens**: LLM training corpus has plenty of "graph in Rust" examples, but the `Weak` pattern is underrepresented. The model defaults to symmetric strong references.

**Prompt triggers**: "build a tree with parent links", "graph data structure", "linked list with previous pointer", "DOM-like structure", "scene graph", any bidirectional ownership.

**BANNED**:
- `Rc<RefCell<T>>` or `Arc<Mutex<T>>` on both sides of a bidirectional reference.
- "Parent owns children, children own parent" patterns.

**REQUIRED**:
- One direction is `Rc<T>` (or `Arc<T>`), the other is `Weak<T>`. Convention: parent owns children with `Rc`, children point to parent with `Weak`.
- For any graph structure with cycles, prefer arena-style storage: `Vec<Node>` + `NodeId(usize)` indices. No reference cycles possible, no `RefCell` overhead, better cache locality. Crates: `slotmap`, `id-arena`, `petgraph`.
- When `Weak::upgrade()` returns `None`, treat it as a normal case (parent has been dropped), not an error.

**Detection**: profile with `heaptrack` or `valgrind --tool=massif` for steady-state memory growth. In dev builds, periodically print `Rc::strong_count(&node)` for representative nodes.

## §B11. Blocking the async executor

**The trap**: LLM puts `std::thread::sleep`, `std::fs::*`, blocking HTTP clients, or synchronous DB drivers inside `async fn`. The compiler doesn't care — these are valid sync functions, and tests pass because they're single-threaded and short. Production hits the wall at ~N concurrent requests (N = tokio worker count, often the CPU core count): every worker is blocked, no other tasks make progress, latency spikes to seconds.

**Why this happens**: corpus statistics. `std::fs::read_to_string` is *vastly* more common in training data than `tokio::fs::read_to_string`.

**Prompt triggers**: "read a config file", "fetch from URL", "sleep for N seconds", "wait", "make an HTTP request", anything that does I/O.

**BANNED in any `async fn` or function called from `tokio::spawn`**:
- `std::thread::sleep`  →  `tokio::time::sleep`
- `std::fs::*` (read, write, metadata, etc.)  →  `tokio::fs::*`
- `std::io::Read` / `Write` on real files/sockets  →  `tokio::io::AsyncReadExt` / `AsyncWriteExt`
- `reqwest::blocking::*`  →  `reqwest::Client` (async)
- `rusqlite`, synchronous `postgres` crate  →  `sqlx`, `tokio-postgres`, or wrap in `tokio::task::spawn_blocking`
- CPU-bound work taking more than ~100µs — wrap in `tokio::task::spawn_blocking`. Do not substitute `yield_now` (see below for why).

**REQUIRED**:
- For genuinely CPU-bound work (compression, hashing, parsing large blobs, calling a sync C library, using a sync crate that has no async equivalent): wrap in `tokio::task::spawn_blocking(|| { ... }).await?`. This dispatches to a *separate* blocking-task thread pool, freeing the async worker thread for other tasks.
- `tokio::task::yield_now().await` is **not** an alternative to `spawn_blocking` for CPU-bound work. `yield_now` only gives *other tasks already on the same worker thread* a chance to make progress; when your task resumes, the worker is still occupied by you. It does not solve "starving the executor" because the worker count is fixed (typically the CPU core count). Use `yield_now` only for cooperative fairness inside an IO-bound task that occasionally does a small CPU burst.
- Verify with `tokio-console` or `tracing` spans that no task holds a worker thread longer than its budget.

## §B12. Cryptographic code (silent insecurity)

**The trap**: cryptographic code generated by LLMs has a unique failure profile. Studies report that only ~23% of LLM-generated crypto Rust code compiles at all, and of the code that *does* compile, static analyzers like CodeQL miss **~57% of the vulnerabilities** present. Crypto code looks right, runs, passes round-trip tests (encrypt → decrypt yields original) — and is still catastrophically insecure.

**Why this happens**: cryptography requires *protocol-level* reasoning the LLM does not do. Encrypt-then-decrypt round-trip is the canonical test, and it passes for any non-broken cipher regardless of whether the key, nonce, or mode is sound. The bugs live at a level orthogonal to functional correctness.

**Specifically dangerous patterns**:
- **Nonce reuse**: hardcoded nonce, nonce derived from a counter that resets, nonce equal to the message ID. Reusing a nonce with the same key in AES-GCM or ChaCha20-Poly1305 is catastrophic — recovers plaintext or forges authentication.
- **API hallucination in crypto crates**: invented methods on `ring`, `rust-crypto`, `aes-gcm`, `chacha20poly1305`. Crypto-API names look interchangeable to the LLM but have very different security properties.
- **Weak parameter choices**: ECB mode (which the LLM may select because it's "simpler"), 64-bit nonces with random generation (birthday-bound collision), insufficient PBKDF2 iterations.
- **Mixing primitives across security levels**: using SHA-1 alongside AES-256, or pairing a strong cipher with a weak MAC.
- **Custom crypto code**: hand-rolling any cryptographic primitive in Rust. Almost always wrong.

**REQUIRED**:
- I do not write cryptographic code beyond *direct calls to well-known high-level APIs* (e.g., `aes_gcm::Aes256Gcm::new(key).encrypt(nonce, plaintext)`).
- For any crypto-touching task, I propose the design in plain text first and ask the user to confirm the threat model before writing code.
- Nonces are generated via a CSPRNG — prefer `rand::rngs::OsRng` for keys and security-critical nonces, **never** hardcoded, never reused, never derived from a counter without explicit cryptographic justification.
- Default to high-level libraries (`age`, `ring`, `rustls`) over low-level primitives.
- For password hashing: `argon2`, not bare PBKDF2 or — under any circumstances — plain SHA-256.
- I surface every line of crypto code in the post-flight summary with extra prominence. Crypto code is the *one place* I recommend mandatory human cryptographer review.

**BANNED**:
- Writing custom encryption/decryption logic.
- Implementing cryptographic primitives (block ciphers, hash functions, KDFs) by hand.
- Using `SmallRng`, `StdRng`, or any seedable RNG for security-sensitive randomness — use `OsRng` (or `getrandom`) directly. The rule is "OS-backed entropy for keys, nonces, salts", not a literal call name: in `rand` 0.8.x the default RNG accessor is `thread_rng()`, in 0.9+ it is `rng()`. Both are CSPRNGs per the `rand` security guarantees, but `OsRng` is the safer default when seeding chains are part of the threat model. State the `rand` version assumed.
- Storing crypto keys in source code, environment variables read at compile time, or anywhere they end up in the binary.

## §B13. Check-then-act races in concurrent collections (TOCTOU)

**The trap**: LLMs port single-threaded patterns from Python/JS/Java into multi-threaded Rust. The canonical example is the "lazy cache":

```rust
// BANNED — race between contains_key and insert
if !cache.contains_key(&key) {
    let value = expensive_fetch(&key).await;
    cache.insert(key, value);
}
```

In a single-threaded test, this is correct. Under concurrent load, N threads simultaneously see "key is absent", N threads simultaneously call `expensive_fetch`, and only one write actually wins. The cache works *functionally* — every lookup returns a value — but the "expensive" function is called N times when it should have been called once. Variants of this pattern fail similarly: read-modify-write on a counter, "if absent insert default else update", lazy initialization with `bool` flag.

**Why this happens**: in single-threaded languages, check-then-act is sound. The model has a strong prior on it. The Time-of-Check-to-Time-of-Use (TOCTOU) gap is invisible from a single function's perspective.

**Prompt triggers**: "cache", "memoize", "lazy initialization", "ensure exactly one X", "deduplicate", "if not exists, create".

**BANNED**:
- `if !map.contains_key(k) { map.insert(k, v); }` and any variation where check and act are separate calls.
- `if map.contains_key(k) { let v = map.get(k).unwrap(); ... }` — between the check and the get, another thread could remove the entry, and `.unwrap()` panics.
- "Two-phase commit"-style patterns across separate operations on a concurrent collection.
- `let x = *counter.lock().unwrap(); *counter.lock().unwrap() = x + 1;` — read and write are separate critical sections, a thread can interleave.

**REQUIRED**:
- For "insert if absent": `map.entry(key).or_insert_with(|| compute_value())`. The `entry` API holds the relevant bucket lock across the check and act.
- For `DashMap`: `dashmap::DashMap::entry(key).or_insert_with(...)`.
- For async expensive computation that must run once: combine `entry` with `Arc<OnceCell<T>>` or `tokio::sync::OnceCell`. The pattern:
  ```rust
  let slot = cache.entry(key).or_insert_with(|| Arc::new(OnceCell::new()));
  let value = slot.get_or_init(|| async { expensive_fetch().await }).await;
  ```
- For atomic counters: `AtomicUsize::fetch_add(1, Ordering::Relaxed)`, not lock-load-add-store.
- For "compare and swap" patterns: `Atomic*::compare_exchange` or `Atomic*::fetch_update`.

**Detection**: this is invisible to type checking and almost always invisible to tests. The defense is recognizing the pattern at write time. If a function does two consecutive operations on a shared collection, it is a candidate.

## §B14. Unbounded channels and backpressure neglect

**The trap**: when the producer/consumer rate is unbalanced, an `mpsc::unbounded_channel` doesn't block the producer — it just lets the queue grow. Tests with 5–100 messages pass. Production with a producer that's 2× faster than the consumer accumulates millions of pending messages, RAM climbs steadily, and the OOM killer eventually terminates the process — usually under peak load when it hurts most.

**Why this happens**: bounded channels force the producer to handle "channel is full" via `try_send`/`send` errors; `unbounded` has the simpler API and is the LLM's path of least resistance — the §C5 reflexive-fix pattern applied to channel selection.

**Prompt triggers**: "send events to a worker", "background queue", "log messages to a task", "producer-consumer", "event bus", "websocket broadcast", "metrics pipeline".

**BANNED** in any non-trivial pipeline:
- `tokio::sync::mpsc::unbounded_channel()` without explicit justification that the producer rate is provably bounded by an external invariant.
- `flume::unbounded()`, `async_channel::unbounded()` for the same reason.
- A `Vec` that is `push`-ed in a hot loop with no consumer or cap — same failure shape as an unbounded channel, different surface. `Vec::push` itself is fine (amortized O(1)); the failure is the missing drain or bound.

**REQUIRED**:
- Default to **bounded** channels: `tokio::sync::mpsc::channel(N)`. Size `N` from the actual constraints, not from a folk number: large enough to absorb the *expected producer burst over one consumer cycle*, small enough that `N × sizeof(message)` fits the per-task memory budget. If the right `N` cannot be reasoned about, that is a signal that the backpressure policy itself needs design before the channel is written. Never `unbounded`.
- Decide the **backpressure policy** explicitly: block the producer (default `send().await`), drop oldest (`try_send` with explicit drop), drop newest (`try_send` returning error → log and discard), or apply rate limiting upstream. State the choice in a comment.
- For broadcast scenarios where slow consumers shouldn't slow producers: `tokio::sync::broadcast::channel(N)` with explicit handling of `RecvError::Lagged` (which indicates dropped messages).
- For any unbounded queue that *must* exist (e.g., legacy interop): expose its size as a metric and alert when it grows abnormally.

**Detection**: unbounded channel growth doesn't appear in tests. Defense is at write time (default to bounded) and via monitoring (track `Sender::capacity()` or queue length as a metric in production).

## §B15. Advanced async pitfalls (AFIT, Pin, Waker, block_on)

A cluster of narrow but high-impact traps that appear in non-trivial async code. Each compiles in isolation; each fails in production or under composition.

**AFIT vs RPITIT — terminology matters, they are not interchangeable:**

- **AFIT** (async fn in trait) — the syntax `trait Foo { async fn bar(&self) -> T; }`. Stabilized in Rust 1.75. Desugars to a method returning an opaque, anonymous `impl Future` whose `Send`-ness is **not bounded in the trait signature**. The trait compiles, implementations compile, but `tokio::spawn(x.bar())` fails with a non-obvious `Send` error because the returned future is not statically known to be `Send`. There is no syntactic way to add `+ Send` directly to an `async fn` in a trait.
- **RPITIT** (return-position impl trait in trait) — the syntax `trait Foo { fn bar(&self) -> impl Future<Output = T> + Send; }`. Lets you state bounds (including `+ Send`) on the returned `impl Future` directly. This is the construct you actually want when the trait's methods will be spawned onto `tokio`. AFIT and RPITIT share a desugar lineage — AFIT desugars into an RPITIT-shaped method internally — but as *written-down* syntactic forms they have materially different bound-expressing capabilities: AFIT cannot state `+ Send` on the return type at the trait definition site, RPITIT can. Treating them as interchangeable in source is the conflation to avoid.

**Decision table for async-returning trait methods**:

| Need | Use |
|---|---|
| Internal trait, no `tokio::spawn`, single executor | Plain **AFIT** (`async fn bar(&self) -> T`). |
| Method must be `Send` for `tokio::spawn` | **RPITIT** with explicit `+ Send`. |
| Library trait, want both Send-bounded and non-Send variants | `#[trait_variant::make(Send)]` from `trait-variant` — generates a Send-bounded variant alongside the original. |
| Need `dyn Trait` (trait objects) for async methods | `async-trait`. As of stable Rust through mid-2026, AFIT and RPITIT traits are not generally `dyn`-compatible without workarounds; stabilization of `dyn`-compatible RPITIT is an in-flight RFC, so verify the current status against your `rustc --version` before relying on a `dyn` async trait without `async-trait`. `async-trait` boxes every call (heap allocation per invocation) but remains the well-supported way to get `dyn` async traits today. |

**REQUIRED**:
- Pick the construct deliberately and state it in a comment on the trait: `// AFIT (no Send)`, `// RPITIT + Send`, `// trait-variant`, or `// async-trait (dyn)`.
- Surface every async-returning trait method in the post-flight summary, with the syntax used and whether `Send` is bounded.
- Never describe RPITIT as "AFIT with a Send bound" in source code. AFIT desugars into RPITIT internally, but the trait's *written* syntax determines what bounds you can express — pick the form deliberately.

**`Pin::new_unchecked` without justification**: `Pin::new_unchecked` is `unsafe` for a reason — it asserts that the pointee will never move again. LLMs reach for it when they don't understand `Pin` rather than as a justified low-level operation. If `Box::pin(...)`, `pin!` macro, or `pin-project` would work, use them.

- Default to `Box::pin(future)` (owning, heap-allocated, `Pin<Box<T>>`) or the `pin!` macro (borrowing, stack-allocated, `Pin<&mut T>`). LLMs frequently mix these up when adapting examples — they have different lifetimes and different ownership. State which one you mean.
- `Unpin` is an auto-trait. Most types implement it automatically, which makes `Pin<&mut T>` effectively free to use. Pinning discipline actually bites only for `!Unpin` types: hand-written futures with internal references, generator state machines, types explicitly opted out via `PhantomPinned`. The common LLM failure is conflating "this code involves a `Pin`" with "this type is `!Unpin`" — most of the time the `Pin` is incidental and Pinning rules add no real constraint.
- For projecting through `Pin`, use the `pin-project` or `pin-project-lite` crate, never manual `Pin::new_unchecked`.
- Every `Pin::new_unchecked` requires a `// SAFETY:` block proving the pointee is genuinely never moved (per §B5) — and the type must actually be `!Unpin` for the assertion to mean anything.

**Forgotten Waker in manual `Future::poll`**: when implementing `Future` by hand, returning `Poll::Pending` without first registering the current task's `Waker` causes the task to hang forever — nothing will ever wake it. The executor doesn't poll spontaneously.

- Before any `return Poll::Pending`, store `cx.waker().clone()` somewhere the underlying source will call on completion.
- Default to combinators (`async/.await`, `FutureExt`, `tokio_util::sync::PollSender`) rather than manual `Future` impls.
- If hand-rolling is unavoidable, write a comment naming who will call the stored waker and under what condition.

**`block_on` inside an async runtime**: `tokio::runtime::Handle::block_on` (or `futures::executor::block_on`) called from code already running inside a tokio runtime panics with "Cannot start a runtime from within a runtime". This happens when LLM writes a sync-looking helper that internally calls `block_on`, then invokes it from async code.

- Inside async code, use `.await`, not `block_on`.
- For sync-to-async bridges, use `tokio::task::spawn_blocking` and `block_in_place`, never nested `block_on`.
- If a helper function is shared between sync and async callers, prefer making the helper async and forcing sync callers to bridge explicitly.

---

# TIER C — Architecture and ergonomics

These are not bugs in the strict sense, but design choices the LLM makes that are expensive to reverse.

## §C1. Blanket impls in public APIs (semver hazard)

**The trap**: `impl<T: Display> Bar for T` in a published crate is a versioning landmine. Consumers may have `impl Bar for MyType` that becomes ambiguous when an upstream blanket impl is added or narrowed. The breakage surfaces months later on consumer CI, not the author's.

**REQUIRED in any `pub` API**:
- Blanket `impl<T: Bound>` only when the trait is **sealed** (private supertrait the crate controls):
  ```rust
  mod sealed { pub trait Sealed {} }
  pub trait MyTrait: sealed::Sealed { ... }
  ```
- Otherwise: write per-type impls or use a marker trait the crate exposes for opt-in.
- For any public trait being added, explicitly state in a comment whether it is sealed or open to external impl.
- Respect orphan rules: never `impl ForeignTrait for ForeignType`. Use the newtype pattern: `pub struct MyWrapper(pub Foreign);`.

## §C2. Error handling discipline

**The trap**: `anyhow::Error` in library crates poisons downstream error handling. `unwrap()` and `expect()` in non-test code is a runtime panic waiting to happen. The `?` operator silently loses context if `From` impls are too eager.

**REQUIRED**:
- In **published library crates** (anything shipped to crates.io with a `pub` API that other authors consume): use `thiserror` for typed errors, never `anyhow::Error` in public APIs. Each `pub fn` returning `Result` has a typed error. The cost of `anyhow` here is paid by every downstream caller who loses the ability to match on specific error variants.
- For **internal/workspace libraries** (not published, only used within the same workspace by the same team): `anyhow::Error` in `pub fn` is acceptable if the team agrees, but make it a deliberate choice — once a library moves toward publication, the migration to typed errors becomes painful.
- In **binary** crates (`main.rs` and friends): `anyhow::Error` is acceptable for top-level handlers and CLI surfaces.
- `unwrap()` is allowed only when (a) it is statically impossible to fail and I have a comment explaining why, or (b) in tests. Same for `expect()`.
- `?` is fine when the conversion is meaningful; if it loses context, use `.map_err(|e| MyError::Context { source: e, info: ... })` instead.
- `panic!`, `todo!`, `unimplemented!`, `unreachable!` are surfaced in the summary with justification.

**BANNED**:
- `anyhow::Result<T>` in a `pub` API of a published library crate.
- `.unwrap()` on `Mutex::lock()` in production code (the panic message is unhelpful; use `.expect("description")` minimum, or handle the poison case).
- Silent `let _ = result;` to discard errors. If discarding is intentional, comment why.

## §C3. Async runtime and ecosystem coherence

**The trap**: mixing `async-std` types with `tokio` dependencies, or generating code that uses `tokio::fs` on `wasm32-unknown-unknown`. The compilation may succeed if features align, but behavior at runtime is broken.

**REQUIRED**:
- Verify the runtime once at the start (read `Cargo.toml`). Do not mix `tokio` and `async-std` in the same crate without explicit reason.
- For `wasm32` targets: no threads, no blocking I/O, no `tokio::time::sleep` (use `gloo-timers` or equivalent).
- For `#![no_std]` crates: no `String` or `Vec` without `extern crate alloc`; no `std::*` paths.
- For embedded with `embassy` or `embedded-hal-async`: do not mix with `tokio`-flavored APIs.
- `Pin<Box<dyn Future>>` is rarely the right answer — usually `impl Future` works. When using `pin_project`, use it correctly (the macro, not manual `Pin::new_unchecked`).

## §C4. Iterator and allocation discipline

**The trap**: unnecessary `clone()` on `Copy` types, materializing collections mid-chain with `collect::<Vec<_>>()` only to iterate again, `format!` in hot paths, treating `String` as the default string type everywhere.

**REQUIRED**:
- Profile before defending these on micro grounds, but as defaults:
- Prefer `&str` and `&[T]` in function signatures over `String` and `Vec<T>`.
- Iterator chains stay lazy: avoid intermediate `.collect()` unless the next stage requires materialization.
- For hot paths, write to a `&mut impl io::Write` or `&mut String` via `write!`/`writeln!` rather than allocating with `format!`.
- `clone()` is fine when needed; surface it in the summary so the user can question it.

## §C5. Reflexive `.clone()` as a borrow-checker silencer

**The trap**: when borrow checker complains, the LLM's path of least resistance is to insert `.clone()` or `.to_string()` until errors disappear. The code compiles. The performance cost is invisible until profiling. This is a *different* failure mode from §C4 — it's not an idiom drift, it's a reflexive *fix-it strategy* that resolves a real borrow problem with a hidden allocation.

**Why this happens**: gradient descent rewards "compiles" heavily; the model learned that adding `.clone()` is a reliable way to make red squiggles go away. The cost (allocation, deep copy of `Vec<T>`, etc.) isn't penalized anywhere in training.

**Prompt triggers**: any prompt involving a borrow checker error in the conversation history; "fix the lifetime issue"; "make this compile"; refactoring sessions where the user is iterating on a function signature.

**REQUIRED**:
- Before inserting `.clone()`, ask: can this be solved by restructuring ownership (split borrows, borrow earlier-release later, take `&self` instead of `self`)?
- For `Copy` types (i32, bool, small struct of `Copy` fields), `.clone()` is a code smell — `clippy::clone_on_copy` exists for a reason. Never insert it.
- For `&str` → `String` conversions purely to escape a lifetime: re-examine the lifetime first. The String allocation is often masking the real problem from §B1.
- For `Vec<T>` clones in hot paths: consider `&[T]`, `Cow<'_, [T]>`, or `Arc<[T]>`.
- Every `.clone()` and `.to_string()` I introduce gets surfaced in the post-flight summary with a one-line justification, so the user can question it.

**BANNED**:
- `.clone()` on a `Copy` type.
- `String::from(s)` or `s.to_string()` immediately followed by use as `&str` (the original would have worked).
- Cloning inside a loop where the cloned value is only read.
- Replacing `&T` with `T` in a function signature just to make a call site compile.

## §C6. Procedural macro hygiene

**The trap**: proc-macros generate code that's pasted into the user's crate. If the macro writes `Option<T>`, it resolves at the call site — and if the user has `type Option = MyOption;`, the macro silently breaks. Hygiene violations in proc-macros are invisible at macro authoring time and only surface at user sites.

**REQUIRED in any proc-macro output**:
- Use absolute paths for every standard library item: `::core::option::Option<T>`, `::core::result::Result<T, E>`, `::std::vec::Vec<T>`, `::std::string::String`. Never bare `Option`, `Result`, `Vec`, `String`.
- For external traits: `::serde::Serialize`, not `Serialize` (and require the macro user to add `serde` as a dependency).
- For error reporting in macro expansion, use `syn::Error::to_compile_error()` returning `TokenStream`, which surfaces correctly at the user's call site. **Never `panic!`** in proc-macros — the user sees an opaque panic message without source location.
- For `#[derive]` macros that add bounds (e.g., `#[derive(Clone)]` adding implicit `T: Clone`), consider whether this matches user intent. For finer control, use `derive_more` or `derivative` and document the choice.

## §C7. Cargo feature flag hygiene

**The trap**: Cargo accepts unknown feature names silently. A typo like `#[cfg(feature = "widnows")]` becomes dead code that never compiles, never runs, and never warns — until production reveals a missing code path.

**REQUIRED**:
- Declare every feature in `[features]` in `Cargo.toml`. Rust 1.80+ automatically emits the `unexpected_cfgs` lint for any `#[cfg(feature = "...")]` whose name doesn't appear there — no extra flag needed. Treat the lint as `deny`, not `warn`, in CI.
- Every `feature` in `Cargo.toml` is mirrored exactly in every `#[cfg(feature = "...")]`. Names are case-sensitive and exact.
- Avoid feature-gated `pub` fields in structs — they break the public API between feature combinations. If a field is conditional, the whole struct or the whole module should be conditional.
- Test the full feature matrix in CI: `cargo hack --feature-powerset check` or equivalent, at least for libraries.
- For platform-conditional dependencies with features (`[target.'cfg(...)'.dependencies]`), be aware that `features = [...]` activates globally per Cargo's resolution, not per-target — this is a known Cargo gotcha (see cargo#2524).

---

# Pre-flight checklist (run mentally before any non-trivial Rust)

Before writing the code, answer all seven out loud:

1. **Versions**: which exact crate versions am I targeting? Did I read `Cargo.toml` and `CLAUDE.md`?
2. **APIs**: am I about to call any method I'm not 100% sure exists in the pinned version? If yes, flag it.
3. **Async or sync context**: will this run under tokio? Are there locks that could cross `.await`? Is this `Send + 'static`?
4. **Cancel-safety**: for every `async fn`, can it tolerate cancellation at every `.await`? If not, where do I detach via `spawn` or document the precondition?
5. **Unsafe**: do I have a stated `// SAFETY:` invariant for each block? Is miri in CI for this file?
6. **Lifetimes**: if I'm returning a reference, can I write two consecutive call sites with disjoint inputs?
7. **Public surface**: is anything I'm marking `pub` part of the intended public API? Any blanket impls? Any error types leaking through?

If I cannot answer any of these confidently, I ask the user before generating code rather than guessing.

---

# Post-flight checklist (run after generating Rust)

```bash
cargo build                                                   # baseline
cargo clippy -- -W clippy::pedantic \
                -W clippy::await_holding_lock \
                -W clippy::unwrap_used \
                -W clippy::expect_used \
                -W clippy::missing_safety_doc \
                -W clippy::undocumented_unsafe_blocks \
                -W clippy::clone_on_copy \
                -W clippy::redundant_clone \
                -W unused_must_use
cargo test
cargo +nightly miri test    # for any file touching unsafe
```

Optional but strongly recommended for production code:
- `tokio-console` to observe runtime task health and detect blocked workers (§B11) or stuck locks (§B9).
- `heaptrack` or `valgrind --tool=massif` for steady-state memory profiling (§B10).
- `loom` for concurrency model checking of multi-lock code (§B9).

For any of the following found in the generated code, surface it explicitly to the user in the summary with line numbers and justification:
- `unsafe`
- `unwrap`, `expect`
- `transmute`, `mem::transmute_copy`
- `Arc<Mutex<_>>`, `Arc<RwLock<_>>`
- two or more lock acquisitions in the same function or call chain (§B9)
- `Rc<RefCell<_>>` or `Arc<Mutex<_>>` in graph/tree structures (§B10)
- manual `Send` / `Sync` impl
- blanket impl (`impl<T: Bound>`)
- `panic!`, `unimplemented!`, `todo!`, `unreachable!`
- `pub` items added to the crate API
- `as` casts between integer types of different signedness or width
- any async function call without `.await` (§B8) — verify intentional fire-and-forget via `tokio::spawn`
- `std::thread::sleep`, `std::fs::*`, `reqwest::blocking::*` inside any `async` context (§B11)
- any cryptographic operation (§B12) — list every crypto call, library, and parameter explicitly
- every new dependency added to `Cargo.toml` (§A1) — list crate name, version, and a one-line justification for each, so the user can audit against slopsquatting
- every `pub fn` with a non-`'static` output lifetime (§B1) — flag as potential API lifetime leak
- every check-then-act pattern on a shared collection (§B13) — `contains_key` + `insert`, `get` + `set`, lock-load-modify-store
- every `unbounded_channel`, `flume::unbounded`, `async_channel::unbounded` (§B14) — require explicit justification or replacement with bounded variant
- every `async fn` in a trait definition (§B15) — flag missing `+ Send` bound for tokio spawn use
- every manual `Future::poll` implementation (§B15) — verify waker registration before `Poll::Pending`
- every `Pin::new_unchecked` or `mem::transmute` of pin-related types (§B15) — verify the SAFETY justification holds
- every `.lock().unwrap()` (§B2 poisoning) — recommend explicit poison handling for non-trivial code
- every proc-macro that emits bare `Option`/`Result`/`Vec` paths (§C6) — hygiene risk

---

# When this command is loaded

I will:
- Read `Cargo.toml` and `CLAUDE.md` to pin versions and idioms before writing code.
- Treat every rule above as a HARD constraint, not a guideline.
- Surface violations as blocking issues, not warnings.
- Refuse to write trait hierarchies blind; propose, then wait for approval.
- Refuse to write `unsafe` without `// SAFETY:` justification.
- Flag API calls I'm uncertain about rather than hallucinate them.
- Run the post-flight checklist mentally and report results before declaring work complete.

The principle: **if a category of bug exists where the compiler cannot help, the discipline must move from the type system into this checklist**. Rust gives me the strongest type system of any mainstream language, but cancel safety, semver, drop ordering, and UB in unsafe live outside it. This document is where that gap is filled.