# Cluster 08 — Rust Cookbook + Rustlings + Comprehensive Rust (Google)

> Dense expert-level notes distilled from three canonical practice-oriented resources.
> Audience: coding LLMs (Claude, Cursor) using this as a knowledge base.
> Source 1: Rust Cookbook — https://rust-lang-nursery.github.io/rust-cookbook/ (recipe-per-task, curated crate picks)
> Source 2: Rustlings — https://github.com/rust-lang/rustlings (exercise-driven onboarding)
> Source 3: Comprehensive Rust (Google) — https://google.github.io/comprehensive-rust/ (4-day course for experienced engineers)
>
> **Research provenance (fetched April 2026):** Content is aligned to the live sites — [Rust Cookbook index](https://rust-lang-nursery.github.io/rust-cookbook/), [Cookbook About](https://rust-lang-nursery.github.io/rust-cookbook/about.html), [Rustlings `README.md` / `exercises/README.md`](https://github.com/rust-lang/rustlings) (main branch layout via GitHub API), [Comprehensive Rust Welcome](https://google.github.io/comprehensive-rust/), [Course Structure](https://google.github.io/comprehensive-rust/running-the-course/course-structure.html), [Concurrency welcome](https://google.github.io/comprehensive-rust/concurrency/welcome.html), and day welcome pages (`welcome-day-1` … `welcome-day-4`). Spot-check individual cookbook recipe pages for exact `Cargo.toml` crate versions before production use.

---

## Taxonomy mapping

| Section | Taxonomy bucket | Primary source |
|---|---|---|
| 1. Meta-principles / "Why Rust" | 01-meta-principles | Comprehensive Rust Welcome + course assumptions |
| 2. Idioms & canonical patterns | 03-idioms | Cookbook recipes |
| 3. Error handling | 06-error-handling | Cookbook About (`anyhow`/`thiserror`) + Comprehensive Rust **Day 4 PM** (Fundamentals) |
| 4. Async & concurrency | 07-async-concurrency | Comprehensive Rust **Concurrency** deep-dive (separate track; not Day 4 Fundamentals) |
| 5. Performance | 09-performance | Cookbook (data-processing, parallel) + scattered |
| 6. Testing & tooling | 10-testing-and-tooling | Rustlings clippy/tests + Comprehensive Rust testing day |
| 7. Ecosystem crate picks (canonical) | 11-ecosystem-crate-picks | Cookbook (entire book is this) |
| 8. Learning-path pedagogy | (cross-cutting) | Rustlings category order |
| 9. Cargo / workspace / CI (operational) | (cross-cutting) | §18 — practical addendum (not in Cookbook course) |

---

# 1. Meta-principles — Comprehensive Rust's framing

## 1.1 Why Google wrote Comprehensive Rust

Google authored the course specifically to **onboard experienced engineers (C++, Java, Go, Kotlin) to Rust in ~4 intensive days**. It was first built for Android engineers; now covers bare-metal, Chromium, and generic concurrency tracks.

Key framing choices:

- **No hand-holding for basic programming concepts.** Assumes readers already know loops, functions, polymorphism, recursion. Spends its budget on what is *distinctive* about Rust.
- **Memory management is the spine.** In **Rust Fundamentals** (four days), stack/heap, moves, `Drop`, smart pointers, borrowing, and lifetimes are concentrated on **Day 3** per the official schedule—not on Day 1. Day 1 establishes syntax, types, control flow, tuples/arrays, references, and user-defined types. Everything later is framed relative to the ownership model.
- **Zero-cost abstractions demonstrated not just asserted.** Uses iterator vs. loop compilation, `Box<dyn Trait>` vs. `impl Trait`, and `async` state machines to show abstraction has no runtime tax.
- **Type-driven design as the Rust mental model.** "Make invalid states unrepresentable." Newtypes, enum variants carrying data, phantom types — taught as the primary way of encoding invariants.
- **Prefer shared-xor-mutable (XOR).** The aliasing/mutability rule is repeated across modules: `&T` aliasable or `&mut T` unique, never both. Most soundness errors collapse to this invariant.

## 1.2 Course-wide principles worth memorizing

| Principle | Short-form |
|---|---|
| **Ownership has exactly one owner** | Moving transfers; cloning duplicates; no shared ownership without `Rc`/`Arc`. |
| **Shared XOR mutable** | `&T` = any number; `&mut T` = exactly one; never simultaneously. |
| **Borrow checker is a static aliasing analysis** | The checker exists to prove no data races and no use-after-free at compile time. |
| **Lifetimes are descriptive not prescriptive** | `'a` tells the compiler what is *already true* about reference validity; it does not create lifetime. |
| **Drop is deterministic** | RAII at function/scope boundaries. No GC pause; no finalizer queue. |
| **Traits > inheritance** | Polymorphism through traits: static dispatch (`impl Trait`) by default; dyn dispatch only where required. |
| **`Option<T>` not null; `Result<T, E>` not exceptions** | No null pointer, no hidden control flow. |
| **Send + Sync encode thread safety in the type system** | You cannot compile a data race. |
| **`unsafe` is a contract not a bypass** | Inside an `unsafe` block, *you* must uphold invariants that the compiler normally verifies; outside must remain safe. |
| **Cargo is the whole toolchain** | Build, test, doc, bench, lint, format, publish — single command tree. |

## 1.3 "Why Rust" one-liners for experienced devs

- **For C++ devs:** "Same level of control; the borrow checker gives you the aliasing invariants you were already trying to maintain by hand, and catches the ones you missed."
- **For Go devs:** "Same 'one language, one build tool' ergonomics. Trades GC for deterministic drops; trades channels-first for channels-or-locks with `Send`/`Sync` enforcing correctness."
- **For Java/Kotlin devs:** "Traits replace interfaces + generics. Enums replace sealed classes and are way more expressive. No null; no checked exceptions — `Result` covers both."
- **For Python/Ruby/JS devs:** "Static types all the way down, but with type inference so you rarely write them. Errors are values, so control flow is always visible in the signature."

## 1.4 Canonical "you cannot do this in Rust" signals

When the compiler complains about one of the following, the answer is almost always a refactor, not a hack:

- Two `&mut` to the same value — restructure so only one active borrow at a time, or use interior mutability (`Cell`, `RefCell`, `Mutex`).
- A self-referential struct — use `Rc`/`Arc`, an arena (`typed-arena`, `bumpalo`, `slotmap`), or `Pin<Box<...>>` with `Pin` projection.
- Returning a borrow of a local — return owned data, or let the caller own the backing storage.
- Mutating a `HashMap` while iterating — collect indices first, or use the `drain_filter`/`retain` API.
- Implementing a trait for a foreign type on a foreign trait — use newtype pattern.

---

# 2. Comprehensive Rust — official schedule + expert notes

The **Rust Fundamentals** track is **four days** with fixed segments ([Course Structure](https://google.github.io/comprehensive-rust/running-the-course/course-structure.html)). **Threads + async** are *not* Day 4 of Fundamentals; they appear in the separate **Concurrency** deep-dive. Day 4 Fundamentals ends with **Error Handling** and **Unsafe Rust**.

## 2.1 Official Rust Fundamentals timetable (segment titles)

| Day | Segment | Approx. duration (incl. breaks) |
|-----|---------|----------------------------------|
| **1 AM** | Welcome; Hello, World; Types and Values; Control Flow Basics | ~2h10 |
| **1 PM** | Tuples and Arrays; References; User-Defined Types | ~2h45 |
| **2 AM** | Pattern Matching; Methods and Traits; Generics | ~2h50 |
| **2 PM** | Closures; Standard Library Types; Standard Library Traits | ~2h50 |
| **3 AM** | Memory Management; Smart Pointers | ~2h20 |
| **3 PM** | Borrowing; Lifetimes | ~2h30 |
| **4 AM** | Iterators; Modules; Testing | ~2h50 |
| **4 PM** | Error Handling; Unsafe Rust | ~2h20 |

**Welcome pages (what each day says you already know):** [Day 2](https://google.github.io/comprehensive-rust/welcome-day-2.html) recaps Day 1 (basic types, control flow, functions, structs/enums, references). [Day 3](https://google.github.io/comprehensive-rust/welcome-day-3.html) recaps pattern matching, polymorphism (methods/traits/generics), std types, closures — then pivots to memory. [Day 4](https://google.github.io/comprehensive-rust/welcome-day-4.html) recaps ownership, borrowing, lifetimes, smart pointers — then pivots to "large-scale" topics (iterators, modules, tests, errors, unsafe).

**Non-goals (course Welcome):** macros are *out of scope* — defer to the Rust Book / Rust by Example.

## 2.2 Day 1 — Syntax, types, control flow, first user-defined types

**AM —** Hello World, `cargo`, scalar/compound types, type inference, `if`/loops/`match`, functions/methods (intro). **PM —** tuples/arrays/slices/`str`, shared vs exclusive references (`&T` / `&mut T`), structs (named/tuple/unit), enums, `const`/`static`/aliases.

**Expert notes:** `char` is a Unicode scalar (4 bytes). Arrays carry size in the type `[T; N]`. `match` is exhaustive. Last expression in a block returns (no `;`). Rust does not overload functions — use traits or different names.

## 2.3 Day 2 — Pattern matching, traits/generics, closures, std library

**AM —** Destructuring, `match`/`if let`/guards, methods and traits, generics/`where`/`impl Trait`. **PM —** closures (`Fn`/`FnMut`/`FnOnce`), `Option`/`Result`/`Vec`/`String`/`HashMap` and key traits (`Iterator`, `Drop`, `Default`, `From`/`Into`, etc.).

**Important std traits (carry into all later days):**

| Trait | Role | Derive? |
|-------|------|--------|
| `Debug` / `Display` | `{:?}` vs `{}` | `Debug` yes; `Display` hand |
| `Clone` / `Copy` | Deep vs bitwise copy | Often derive |
| `PartialEq`/`Eq`, `PartialOrd`/`Ord`, `Hash` | Collections, sorting | Often derive |
| `Default`, `From`/`Into`, `TryFrom`/`TryInto` | Construction, conversions | Varies |
| `Deref`/`DerefMut` | Smart-pointer ergonomics | Rare |
| `Iterator`/`IntoIterator`/`FromIterator` | Loops and `.collect()` | Impl as needed |
| `Send`/`Sync` | Thread-safety (preview for Concurrency track) | Auto |
| `Error` | Error reporting | Libraries often custom |

## 2.4 Day 3 — Memory model: stack/heap, moves, smart pointers, borrowing, lifetimes

**AM —** Stack vs heap, ownership, move/`Copy`/`Clone`/`Drop`, `Box`, `Rc`/`Arc` (intro to shared ownership). **PM —** Borrow checker rules, `&` vs `&mut`, NLL (non-lexical lifetimes), interior mutability (`Cell`/`RefCell`/`Mutex`), explicit lifetimes, elision, lifetimes on structs.

**Expert notes:**

- **Shared XOR mutable** — many `&T` *or* one `&mut T`, never both.
- **NLL** — a reference ends at its last use, not necessarily at scope end.
- **Reborrowing** — `&mut *x` can temporarily split borrows.
- **Elision** — three rules; methods often need no written lifetimes.
- **`'static`** — literals, leaked data, or "lives forever" bounds.

## 2.5 Day 4 — Iterators, modules, tests, errors, unsafe

**AM —** `Iterator` and combinators, module layout and visibility (`mod`/`pub`/`use`), unit/integration/doc tests. **PM —** `Result`/`?`, custom errors, `panic!` vs recoverable errors, **Unsafe Rust** (raw pointers, `unsafe` blocks/traits, FFI sketch).

**Error handling (same themes as Cookbook About):** libraries favor structured errors (often `thiserror`); applications often use `anyhow` for context chains; `?` + `From` for propagation.

**Testing (Day 4 AM):** `#[test]`, `#[cfg(test)]`, `tests/` integration crate, doc tests, `should_panic`, `ignore`; **`criterion`** for stable micro-benchmarks.

> **Where concurrency lives:** Classical threads/channels/`Send`/`Sync`/`Mutex`/`Arc` and `async`/`.await`/executors are covered in the **Concurrency** deep-dive ([welcome](https://google.github.io/comprehensive-rust/concurrency/welcome.html)), not in Fundamentals Day 4. See **§6** below for the consolidated async/thread digest.

## Comprehensive Rust — supplementary tracks

### Android track
- **AIDL in Rust** — Binder IPC; crate `binder` provides safe wrappers.
- **Logging** — `log` crate facade + `android_logger` backend.
- **NDK bindings** — via `jni` / `ndk` / `ndk-context`.
- **Build** — `cargo-ndk`, integration with Soong / Android.bp.
- **Safe abstractions over unsafe FFI** — pattern reinforcement: expose `#[repr(C)]` ABI, wrap raw pointers, RAII the handles.

### Bare-metal / embedded track
- **`no_std`** — disable std via `#![no_std]`; rely on `core` and `alloc`.
- **Memory-mapped registers** — `volatile_register`, `svd2rust`-generated PACs.
- **`cortex-m` / `cortex-m-rt`** — Cortex-M runtime and abstractions.
- **HAL pattern** — `embedded-hal` traits (SPI, I2C, UART, GPIO) ⇒ portable drivers.
- **`embassy`** — async for embedded; cooperative tasks, interrupt-driven.
- **RTIC (Real-Time Interrupt-driven Concurrency)** — static analysis of priority-based preemption.
- **Critical sections** — `critical-section` crate for portable atomic sections.
- **Useful abstractions** — `heapless` (fixed-capacity collections without alloc), `fixed` (fixed-point math), `nb` (non-blocking I/O).

### Concurrency deep-dive (full day; separate from Fundamentals)

Official segments ([Course Structure](https://google.github.io/comprehensive-rust/running-the-course/course-structure.html)): **Morning** — Threads (~30m), Channels (~20m), Send and Sync (~15m), Shared State (~30m), Exercises (~1h10). **Afternoon** — Async Basics (~40m), Channels and Control Flow (~20m), Pitfalls (~55m), Exercises (~1h10). Prereq: `cargo init` + `cargo add tokio --features full` to run examples.

**Classical concurrency (threads):** `std::thread::spawn` (`FnOnce + Send + 'static`), `JoinHandle`, **`thread::scope`** (borrow stack data safely). **`Send`** / **`Sync`**. **`Mutex`/`RwLock`**, **`Arc`**, **`mpsc`**; MPMC → **`crossbeam-channel`** or **`flume`**. **`Ordering`** on atomics (`Relaxed`/`Acquire`/`Release`/`AcqRel`/`SeqCst`). Condvar: wait in a loop (spurious wakeups).

**Async half:** `async fn` → state machine + **`Future::poll`**, **`Pin`**, **`Waker`**. Executors: **`tokio`** (default for I/O). **`.await`** yields; futures lazy until polled. **`tokio::spawn`**, **`select!`**, **`join!`**. Prefer **`tokio::sync::Mutex`** only if the guard must span `.await`; otherwise **`std::sync::Mutex`** or restructure. **Pitfalls:** blocking in async (`std::thread::sleep` → `tokio::time::sleep`), **`Send` future** errors (`Rc`/`RefCell` across `.await`), cancellation at `.await`, lock ordering deadlocks.

**Ecosystem patterns (beyond slides):** **`rayon`** data parallelism; lock-free **`crossbeam-epoch`** / queues; actor crates (`actix`, `kameo`, …); niche stackful coroutines (`may`, `bastion`).

### Chromium track (half day)

Rust inside Chromium’s **`gn`** build, third-party crates policy, **C++ interop** — see [Chromium](https://google.github.io/comprehensive-rust/chromium.html) (requires a local Chromium build).

### Idiomatic Rust (2-day deep dive, post-Fundamentals)

**Foundations of API design**, **Leveraging the type system**, **Polymorphism** — see [Idiomatic Rust welcome](https://google.github.io/comprehensive-rust/idiomatic/welcome.html). Assumes Fundamentals complete.

### Unsafe deep-dive (work in progress)

Two-day **unsafe** course: safety guarantees, review process, FFI — [Unsafe welcome](https://google.github.io/comprehensive-rust/unsafe-deep-dive/welcome.html).

---

# 3. Rust Cookbook — categories and canonical recipe → crate mapping

> The Cookbook is organized as recipes. Each recipe is a short idiomatic example answering "how do I do X". The crate choices are **canonical** — the community treats these picks as the default way to solve the problem.

## 3.0 Cookbook conventions ([About](https://rust-lang-nursery.github.io/rust-cookbook/about.html))

- **Audience:** newcomers *and* experienced Rust devs (quick reminders).
- **Recipes are full programs** — copy into `cargo new` + add crates from the badge (`cargo add <crate>`; historically via **cargo-edit**).
- **Errors in examples:** prefer **`anyhow`** for application-style propagation; library authors should prefer **`thiserror`** (Cookbook explicitly migrated off legacy `error-chain`).
- **Scope:** std + **Libz Blitz**-style foundational ecosystem crates — quality-bar alignment, not every niche crate.
- **Rand API note:** upstream `rand` evolves; verify `Rng` / `gen` vs `random()` against current docs (Cookbook About shows `rand::rng()`-style snippets in recent revisions).

## 3.1 Master crate-per-task table (the core artifact)

| Task | Canonical crate | Notes |
|---|---|---|
| Random numbers (any distribution) | `rand` | `thread_rng()` for most uses; `rand::distributions::*` for non-uniform |
| Random distributions (normal, beta, etc.) | `rand_distr` | Companion to `rand` |
| Regex | `regex` | RE2-style, guaranteed linear-time, no backtracking |
| Simple logging (app) | `env_logger` + `log` | `log` is the facade, `env_logger` the default backend |
| Structured logging | `tracing` + `tracing-subscriber` | Modern successor; span-aware, async-aware |
| Date/time (calendar, TZ) | `chrono` | The historic standard; timezones via `chrono-tz` |
| Date/time (modern, cleaner API) | `time` | Post-2020 rewrite; many projects prefer it now |
| Argument parsing (simple) | `clap` | Derive API (`#[derive(Parser)]`) is canonical |
| Argument parsing (minimal) | `pico-args` | When you want zero deps |
| Terminal coloring | `ansi_term` / `termcolor` / `owo-colors` | `owo-colors` is the modern pick |
| TUI | `ratatui` (fork of `tui`) | Full-screen terminal apps |
| CSV | `csv` | Serde-integrated |
| JSON | `serde_json` | With `serde` derive |
| TOML | `toml` | With `serde` derive |
| YAML | `serde_yaml` | With `serde` derive |
| XML | `quick-xml` | Fast pull parser |
| MessagePack | `rmp-serde` | Serde-integrated |
| Bincode | `bincode` | Rust-native binary serde |
| Protobuf | `prost` / `rust-protobuf` | `prost` is tokio-ecosystem's choice |
| Base64 | `base64` | |
| Hex | `hex` | |
| URL-encoding | `percent-encoding` or `urlencoding` | |
| URL parsing | `url` | |
| Hashing (non-crypto) | std `DefaultHasher` or `ahash`/`fxhash` | |
| Cryptographic hash | `sha2`, `sha3`, `blake3`, `md-5` | `ring` bundles many |
| HMAC / KDF | `hmac`, `pbkdf2`, `argon2`, `scrypt` | `argon2` is current password-hashing default |
| Symmetric crypto | `aes-gcm`, `chacha20poly1305` | Authenticated encryption; `RustCrypto` org |
| General crypto | `ring` | Curated, foot-gun-free; Go-inspired API |
| TLS | `rustls` | Pure Rust; `tokio-rustls` for async |
| TLS (OpenSSL-backed) | `native-tls`, `openssl` | Use when you need system TLS |
| HTTP client | `reqwest` | Sync + async; most popular |
| HTTP client (minimal) | `ureq` | Blocking, no async runtime |
| HTTP server (batteries-included) | `axum` | Tokio-based, ergonomic |
| HTTP server (actor model) | `actix-web` | Very fast, actor-based |
| HTTP server (minimal) | `warp`, `tide`, `rouille` | Various |
| HTTP server (explicit / Hyper-level) | `hyper` | Low-level; most others built on it |
| WebSockets | `tokio-tungstenite` / `tungstenite` | |
| gRPC | `tonic` | Tokio + prost |
| Compression (gzip/deflate) | `flate2` | Multiple backends: miniz_oxide, zlib |
| Compression (zstd) | `zstd` | |
| Compression (bzip2) | `bzip2` | |
| Compression (lz4) | `lz4_flex` | Pure Rust |
| Compression (xz/lzma) | `xz2` | |
| Tar archives | `tar` | |
| Zip archives | `zip` | |
| Linear algebra / ndarray | `ndarray` | NumPy-like |
| Stats | `statrs` | Distributions, tests |
| Plotting | `plotters` | |
| Parallel iteration | `rayon` | `par_iter()` is the one-liner |
| Async runtime | `tokio` | Multi-threaded, work-stealing |
| Async runtime (minimal) | `smol` | |
| Async runtime (embedded) | `embassy` | |
| Channels (sync, MPMC, bounded/unbounded) | `crossbeam-channel` | Upgrade from `std::sync::mpsc` |
| Channels (sync + async unified) | `flume` | |
| Lock-free data structures | `crossbeam` | Epoch-based GC, queues, deque |
| Atomics beyond std | `atomic` | |
| Mutex (faster, no poisoning) | `parking_lot` | Mutex, RwLock, Condvar, Once |
| SQLite | `rusqlite` | Sync; safe wrapper |
| Postgres | `postgres` / `tokio-postgres` | Sync / async variants |
| MySQL | `mysql` / `mysql_async` | |
| DB ORM | `diesel` | Sync, compile-time-checked SQL |
| DB ORM (async) | `sqlx` | Async, compile-time-checked raw SQL |
| DB ORM (active-record) | `sea-orm` | |
| Email (SMTP) | `lettre` | |
| Config | `config`, `figment` | |
| Filesystem walking | `walkdir` | |
| Glob matching | `glob`, `globset` | |
| `find`-like | `ignore` (from ripgrep) | Also respects `.gitignore` |
| Temp files/dirs | `tempfile` | |
| Memory-mapped I/O | `memmap2` | (not `memmap` which is unmaintained) |
| Path normalization | `dunce` (on Windows) / `path-absolutize` | |
| UUID | `uuid` | v4 / v7 most common |
| Error (library) | `thiserror` | Enum-based, derive |
| Error (application) | `anyhow` | `Result<T>` alias, with `.context()` |
| Error (enhanced reports) | `color-eyre` / `eyre` | Color, backtraces, section reports |
| Env vars | `dotenvy` | Successor to `dotenv` |
| Retry / backoff | `backoff` / `tokio-retry` | |
| Progress bar | `indicatif` | |
| Terminal prompts | `dialoguer` | |
| Debug printing with colors | `pretty_env_logger` | |
| Async trait | `async-trait` | Until native async fn in traits; now GAT/RPITIT exists but compat crate still used |
| Serialization (custom) | `serde` | Derive `Serialize`/`Deserialize` |
| Bit manipulation | `bitflags` | |
| Binary reader/writer | `byteorder` / `bytes` | |
| Streaming bytes | `bytes` | Standard Tokio ecosystem type |
| Iterator extensions | `itertools` | `chunks`, `sorted`, `tuples`, `group_by`, etc. |
| Once-init (thread-safe, no std-1.70+ path) | `once_cell` | Superseded by std::sync::OnceLock for many uses |
| Lazy static | `lazy_static` | Superseded by `once_cell` and std |
| Command-line spinners | `spinoff` / `indicatif` | |
| Enum utilities | `strum` | `EnumIter`, `Display`, `FromStr` derive |
| Numeric traits | `num-traits` | `Zero`, `One`, `Signed`, generic numeric bounds |
| Big integers | `num-bigint` | |
| Complex numbers | `num-complex` | |
| Rational numbers | `num-rational` | |
| Fixed-point | `fixed` | |
| Time duration (human) | `humantime` / `humantime-serde` | `"5m"`, `"1h30m"` parse/format |
| File format sniffing | `infer` / `mime_guess` | |
| Image | `image` | Decode/encode many formats |
| SVG | `resvg` / `usvg` | |
| Audio | `cpal` (device I/O), `rodio` (playback) | |
| Hardware (CPU info, disks, etc.) | `sysinfo` | |
| Serial port | `serialport` | |
| USB | `rusb` | libusb binding |
| Bluetooth | `btleplug` | BLE cross-platform |
| Web scraping | `scraper` (CSS selectors) + `reqwest` | |
| HTML manipulation | `kuchiki` / `scraper` | |
| Template rendering | `tera` (Jinja-like), `handlebars`, `askama` (compile-time) | |
| Markdown | `pulldown-cmark` | |
| Syntax highlighting | `syntect` | |
| FFI / C bindings gen | `bindgen` | |
| FFI / C headers from Rust | `cbindgen` | |
| Python from Rust | `pyo3` | |
| Node from Rust | `napi-rs`, `neon` | |
| WebAssembly (browser) | `wasm-bindgen`, `web-sys`, `js-sys` | |
| WASI | `wasmtime`, `wasmer` | Host-side |
| Macros (proc) | `syn`, `quote`, `proc-macro2` | |

## 3.2 Chapter-by-chapter recipe digest

### Algorithms

- **Generate random numbers** — `rand::thread_rng().gen::<u8>()`; for a range, `rng.gen_range(0..10)`.
- **Generate numbers within a range** — `rng.gen_range(low..high)` (exclusive) or `low..=high` (inclusive).
- **Generate random numbers with a given distribution** — `rand_distr::{Normal, Exp, Poisson, Binomial, LogNormal, Weibull, StandardNormal}`.
- **Generate random values of a custom type** — `impl Distribution<MyType> for Standard`.
- **Create random passwords from a set of alphanumeric characters** — `rand::distributions::Alphanumeric` + `.sample_iter`.
- **Sort a vector of integers** — `v.sort()`; unstable `v.sort_unstable()` is faster.
- **Sort a vector of floats** — `v.sort_by(|a, b| a.partial_cmp(b).unwrap())`; floats are `PartialOrd`, not `Ord`.
- **Sort a vector of structs** — `v.sort_by(|a, b| a.field.cmp(&b.field))` or derive `Ord` and call `.sort()`.

### Command-line

- **Parse command-line arguments** — `clap` with derive API:
  ```rust
  #[derive(Parser)] struct Args { #[arg(short, long)] name: String }
  let args = Args::parse();
  ```
- **ANSI Terminal** — `ansi_term::Color::Red.paint("error")`; modern: `owo-colors` `"text".red()`.

### Compression

- **Decompress a tarball** — `flate2::read::GzDecoder` + `tar::Archive`:
  ```rust
  let tar = flate2::read::GzDecoder::new(File::open("a.tar.gz")?);
  tar::Archive::new(tar).unpack(".")?;
  ```
- **Compress a directory into tarball** — `tar::Builder::new(flate2::write::GzEncoder::new(File::create("a.tar.gz")?, Compression::default()))`.
- **Decompress a tarball while removing a prefix from the paths** — iterate `Archive::entries()`, `strip_prefix` path, unpack manually.

### Concurrency

- **Spawn a short-lived thread** — `std::thread::scope(|s| { s.spawn(|| ...); })` — borrow from stack without `'static`.
- **Create a parallel pipeline** — `crossbeam_channel::unbounded::<T>()`, thread::scope, workers send/recv.
- **Pass data between two threads** — `std::sync::mpsc::channel()`; `tx.send(value)?`, `rx.recv()?`.
- **Maintain global mutable state** — `once_cell::sync::Lazy<Mutex<T>>` or std `OnceLock<Mutex<T>>`.
- **Calculate SHA1 sum of .iso files concurrently** — `rayon::prelude::*`, `files.par_iter().map(|p| sha1(p)).collect()`.
- **Draw fractal dispatching work to a thread pool** — `rayon::scope`, each pixel computed in parallel.
- **Mutate the elements of an array in parallel** — `arr.par_iter_mut().for_each(|x| *x = f(*x))`.
- **Test in parallel if any or all elements of a collection match a given predicate** — `v.par_iter().any(p)` / `.all(p)`.
- **Search items using given predicate in parallel** — `.par_iter().find_any(p)` / `find_first(p)`.
- **Sort a vector in parallel** — `v.par_sort()`.
- **Map-reduce in parallel** — `v.par_iter().map(f).reduce(|| id, g)`.
- **Generate jpg thumbnails in parallel** — open each image in `par_iter`, resize, save.

### Cryptography

- **Calculate the SHA-256 digest of a file** — `ring::digest::digest(&SHA256, &bytes)` or `sha2::Sha256::digest(&bytes)`.
- **Sign and verify a message with HMAC digest** — `ring::hmac::sign(&key, msg)` or `hmac::Hmac::<Sha256>::new_from_slice`.
- **Salt and hash a password with PBKDF2** — `ring::pbkdf2::derive(...)` or `argon2` crate (preferred modern default).

### Database

- **SQLite: create a table / insert-query / transaction** — `rusqlite::Connection::open`, `.execute`, `.prepare`, `.query_map`.
- **Postgres: create tables / insert / aggregate** — `postgres::Client::connect` (sync) or `tokio-postgres` (async).

### Date/time

- **Measure elapsed time** — `let t = Instant::now(); ... ; let d = t.elapsed();` (monotonic).
- **Perform checked date/time calculations** — `chrono::NaiveDate::from_ymd_opt`, `.checked_add_days(Days::new(n))`.
- **Convert local time to another timezone** — `chrono::Local::now().with_timezone(&Utc)` or `chrono_tz::US::Pacific`.
- **Examine the date and time** — `.year()`, `.month()`, `.day()`, `.hour()`, `.minute()`, `.second()`.
- **Convert date to UNIX timestamp and vice versa** — `.timestamp()`, `DateTime::from_timestamp(secs, nsec)`.
- **Display formatted date/time** — `.format("%Y-%m-%d %H:%M:%S")`; RFC 3339: `.to_rfc3339()`.
- **Parse string into DateTime struct** — `DateTime::parse_from_rfc3339(s)` / `NaiveDateTime::parse_from_str(s, fmt)`.

### Development tools

- **Debug / log messages** — `log::info!`, `log::error!`; init with `env_logger::init()`.
- **Enable log levels per module** — `RUST_LOG=mycrate::mod=debug,other=info`.
- **Log to a file / custom output** — `log4rs` YAML config, or `fern` fluent config, or `tracing-appender`.
- **Include configuration files in binary** — `include_str!("config.toml")` / `include_bytes!`.
- **Check for external dependencies** — version queries via `pkg-config` in `build.rs`.
- **Link C/C++ libraries** — `build.rs` with `cc::Build::new().file("src.c").compile("foo")`.

### Encoding

- **Character sets — percent-encode/decode a URL** — `percent_encoding::utf8_percent_encode(s, NON_ALPHANUMERIC)`.
- **Encode a string as application/x-www-form-urlencoded** — `form_urlencoded::Serializer`.
- **Encode and decode hex** — `hex::encode(bytes)` / `hex::decode(s)`.
- **Encode and decode base64** — `base64::engine::general_purpose::STANDARD.encode(bytes)`.
- **Read CSV records / Read CSV with different delimiter** — `csv::ReaderBuilder::new().delimiter(b';').from_reader(r)`.
- **Filter CSV records matching a predicate** — iterate `.records()`, filter, write with `csv::Writer`.
- **Handle invalid CSV data with Serde** — derive `Deserialize`, handle `Result` per row; `#[serde(default)]`.
- **Serialize records to CSV** — derive `Serialize`, `csv::Writer::serialize`.
- **Serialize records to CSV using Serde** — same; works with struct fields.
- **Transform CSV column** — read, mutate, write.
- **Serialize and deserialize unstructured JSON** — `serde_json::Value`, `json!({ ... })` macro.
- **Deserialize a TOML configuration file** — `toml::from_str::<Config>(&contents)?`.
- **Read and write integers in little-endian byte order** — `byteorder::LittleEndian`, `.read_u32::<LittleEndian>()`.

### Error handling

- **Handle errors correctly in main** — use `fn main() -> Result<(), Box<dyn Error>>` or `anyhow::Result<()>`.
- **Avoid discarding errors during error conversions** — impl `From<InnerError> for MyError`; `?` does the conversion.
- **Obtain backtrace of complex error scenarios** — `std::backtrace::Backtrace::capture()` + `RUST_BACKTRACE=1`; `anyhow` / `color-eyre` capture automatically.

### File system

- **Read lines of strings from a file** — `BufReader::new(File::open(p)?).lines()`; iterator of `io::Result<String>`.
- **Avoid writing and reading from a same file** — use `tempfile::tempfile()` for scratch.
- **Access a file randomly using a memory map** — `memmap2::MmapOptions::new().map(&file)?`.
- **File names that have been modified in the last 24 hours** — `walkdir` + filter by `metadata().modified()`.
- **Recursively find duplicate file names** — `walkdir::WalkDir::new(".")`, keyed by filename.
- **Recursively find all files with given predicate** — `walkdir` + filter.
- **Traverse directories while skipping dotfiles** — `walkdir` with `.filter_entry(|e| !is_hidden(e))`.
- **Recursively calculate file sizes** — `walkdir` + `.metadata().len()` accumulation.
- **Find loops for a given path** — follow `read_link` with a visited-set.

### Hardware support

- **Check number of logical CPU cores** — `num_cpus::get()` (or std `std::thread::available_parallelism`).

### Memory management

- **Bit-field of multiple enum values** — `bitflags!` macro from `bitflags` crate.

### Network

- **Listen on unused port TCP/IP** — bind to `127.0.0.1:0`, read `listener.local_addr()?.port()`.
- **Make a HTTP GET request** — `reqwest::blocking::get(url)?.text()?` (sync) or `reqwest::get(url).await?.text().await?` (async).
- **Download a file to a temporary directory** — `reqwest::blocking::get`, `tempfile::NamedTempFile`, stream to file.
- **Query GitHub API** — `reqwest` + auth header + `serde_json`.
- **Parse URL from string** — `url::Url::parse("https://...")?`; query `.scheme()`, `.host_str()`, `.path()`.
- **Create new URL from a base and relative** — `base.join("relative")?`.
- **Extract the URL origin** — `url.origin()`.
- **Remove fragment from URL** — `url.set_fragment(None)`.
- **Use HTTP Basic Auth** — `reqwest::Client::builder()`; `.basic_auth(user, pass)`.
- **Download a file with HTTPS** — `reqwest` with `rustls-tls` feature (`reqwest = { default-features = false, features = ["rustls-tls"] }`).

### Operating system

- **Run piped external commands** — `std::process::Command::new("sh").arg("-c").arg("ls | wc -l").output()`.
- **Redirect stdout / stderr of child to file** — `Command::stdout(File::create("out.txt")?)`.
- **Continuously process child process' outputs** — `Command::spawn()`, `.stdout.take()`, wrap in `BufReader`, iterate lines.
- **Read environment variables** — `std::env::var("PATH")?`.

### Science

- **Vector Mathematics** — `ndarray::Array1`, arithmetic via operator overloading.
- **Matrix computations** — `ndarray` (`Array2`, `.dot(&b)`, `.t()`), `nalgebra` for linear algebra.

### Text

- **Collect unicode graphemes** — `unicode_segmentation::UnicodeSegmentation::graphemes(s, true)`.
- **Verify and extract login from email** — `regex::Regex::new(r"^(?P<login>[^@]+)@...")`.
- **Extract a list of unique #hashtags** — `regex` + `HashSet`.
- **Extract phone numbers** — `regex` with capture groups.
- **Filter log file by matching regex** — `BufReader::new(f).lines()` + `re.is_match(&line)`.
- **Replace all occurrences of one text pattern with another** — `re.replace_all(&s, "$capture")`.

### Web programming

- **Check if an API resource exists** — `reqwest::blocking::Client::new().head(url).send()?.status()`.
- **Set custom User-Agent** — `.header(USER_AGENT, "mycrate/1.0")`.
- **Make a partial download with HTTP range** — `header(RANGE, "bytes=0-1023")`.
- **Handle a rate-limited API** — detect 429; read `Retry-After` header; `tokio::time::sleep`.
- **POST a file to paste-rs** — `client.post(url).body(contents).send()?`.
- **Extract all links from a webpage HTML** — `scraper::Html::parse_document(&html)` + selector `"a"`.
- **Check a webpage for broken links** — recursive extract + HEAD each; use `reqwest::Client::head`.
- **Extract all unique links from a MediaWiki article** — `regex` for `\[\[Link\]\]` + `HashSet`.

---

# 4. Rustlings — exercise categories and what each teaches

> Rustlings is the practice-first onboarding. Each category is a directory of small failing programs (sometimes deliberately with `// I AM NOT DONE`) that compile once the learner fixes them. The order is progressive.

**Upstream README:** [rustlings.rust-lang.org](https://rustlings.rust-lang.org); main repo [README](https://github.com/rust-lang/rustlings/blob/main/README.md) points to the site for setup. **Directory layout (main branch):** `exercises/00_intro` … `exercises/23_conversions` plus `exercises/quizzes/`.

## 4.0 Exercise → *The Rust Programming Language* chapter mapping

From [exercises/README.md](https://github.com/rust-lang/rustlings/blob/main/exercises/README.md) (authoritative pairing with the book):

| Exercise dir | Book chapter |
|--------------|----------------|
| variables | §3.1 |
| functions | §3.3 |
| if | §3.5 |
| primitive_types | §3.2, §4.3 |
| vecs | §8.1 |
| move_semantics | §4.1–2 |
| structs | §5.1, §5.3 |
| enums | §6, §19.3 |
| strings | §8.2 |
| modules | §7 |
| hashmaps | §8.3 |
| options | §10.1 |
| error_handling | §9 |
| generics | §10 |
| traits | §10.2 |
| lifetimes | §10.3 |
| tests | §11.1 |
| iterators | §13.2–4 |
| smart_pointers | §15, §16.3 |
| threads | §16.1–3 |
| macros | §20.5 |
| clippy | Appendix D |
| conversions | *(no single chapter — synthesis)* |

## 4.1 Category list with pedagogical purpose

### `intro`
- First taste: `println!` formatting, code compiles, how to run `cargo run` / `rustlings run`.
- **Concepts**: where the compiler error lives, how to read diagnostics.

### `variables`
- `let` vs `let mut`, shadowing, type annotations, constants (`const`), statics (`static`).
- **Key takeaway**: immutability is the default; shadowing lets you "reuse" names with different types safely.
- **Gotcha drilled**: `let` creates new binding each time; does not re-assign.

### `functions`
- Declaration, parameters with types, return types, expression-vs-statement distinction.
- **Key takeaway**: functions always require type annotations; trailing expression is return.

### `if`
- `if`/`else`/`else if` as expressions — `let x = if cond { 1 } else { 2 };`.
- **Key takeaway**: all arms must produce same type.

### `primitive_types`
- `bool`, `char`, integer/float types, tuples, arrays, slices (`&[T]`), string literals (`&str`).
- **Teaches**: char is 4 bytes (USV), not 1 byte; arrays have size in their type.

### `vecs`
- `Vec<T>`: construction (`vec![]`, `Vec::new()`, `Vec::with_capacity`), push, iter, index, slice.
- **Teaches**: growable heap-allocated array; owned; moves/borrows like any value.

### `move_semantics`
- The central Rust concept. Move vs. borrow; what happens to `Vec`/`String` on assignment; `Clone`, `Copy`.
- **Exercises drill**: function taking `Vec<T>` vs `&Vec<T>` vs `&mut Vec<T>`; returning ownership; building up functions that consume-transform-return.
- **Key mental model**: every value has exactly one owner; transfer or borrow.

### `structs`
- `struct`, tuple structs, unit structs, `impl` blocks, methods (`&self` / `&mut self` / `self`), associated functions (no `self`).
- **Teaches**: builder patterns, method chaining, when to take `self` vs. `&self`.

### `enums`
- Basic enums, enums carrying data, `match`, `if let`, pattern matching on variants.
- **Teaches**: sum types — the feature ML-family languages have that mainstream OO languages don't. Essential for idiomatic Rust.

### `strings`
- `String` vs `&str`; `to_string()` / `.to_owned()` / `String::from()`; slicing; concatenation (`+`, `format!`, `push_str`).
- **Teaches**: ownership model for text; `&str` is a borrow, `String` is owned; UTF-8 byte slicing rules.

### `modules`
- `mod`, `pub`, `use`, `pub use`, crate hierarchy, `mod.rs` vs. `foo.rs` + `foo/`.
- **Teaches**: visibility is per-item; modules form a tree; `use` creates local aliases.

### `hashmaps`
- `HashMap::new`, `.insert`, `.get`, `.entry().or_insert()`, iteration, removal.
- **Teaches**: the `entry` API (insert-or-update in one pass); hash map is unordered; key must implement `Hash` + `Eq`.

### `options`
- `Option<T>`: `Some`/`None`; `match`, `if let`, `?`, `.unwrap_or`, `.map`, `.and_then`.
- **Teaches**: no null — every nullable value is explicit in the type.

### `error_handling`
- `Result<T, E>`; custom error types; `?` operator; `Box<dyn Error>`; `From`/`Into` for error conversion.
- **Teaches**: errors are values; `?` is early-return sugar; implementing `std::error::Error`.
- Exercises culminate in a boxed-error return from `main` and custom error enum.

### `generics`
- Generic functions, generic structs, bounds (`T: Ord`, `where` clauses).
- **Teaches**: monomorphization — each instantiation compiled separately (zero-cost abstractions).

### `traits`
- Defining traits, implementing traits, default methods, trait objects (`dyn Trait`), trait bounds.
- **Teaches**: Rust's interface-polymorphism story; the orphan rule; static vs dynamic dispatch.

### `lifetimes`
- Explicit lifetime annotations on functions and structs; `'static`; when the compiler needs help vs. when elision suffices.
- **Teaches**: lifetime is a compile-time scope; annotations describe relationships between borrow lifetimes.
- **Gotcha**: lifetime is a *description* of what is already true, not a way to *make* references live longer.

### `tests`
- `#[test]`, `assert!`, `assert_eq!`, `assert_ne!`, `#[should_panic]`, `#[cfg(test)]`.
- **Teaches**: tests live next to code; each test is an isolated `fn`; `cargo test`.

### `iterators`
- `Iterator` trait, `iter()` vs `into_iter()` vs `iter_mut()`, `map`, `filter`, `collect`, `fold`, `sum`, `product`, closures, combinator chains.
- **Teaches**: iterator chains are zero-cost (monomorphized and inlined); preferred over manual index loops.
- **Advanced**: implementing `Iterator` for your own type.

### `threads`
- `std::thread::spawn`, `JoinHandle`, `Arc<Mutex<T>>` sharing, channels, `Send`/`Sync`.
- **Teaches**: "fearless concurrency" — the compiler prevents data races; `Arc` + `Mutex` is the bread-and-butter pattern.

### `smart_pointers`
- `Box<T>`, `Rc<T>`, `Arc<T>`, `RefCell<T>`, `Cow<T>`.
- **Teaches**: when heap allocation is needed (recursive types, trait objects); shared ownership; interior mutability; clone-on-write.

### `macros`
- Using existing macros (`vec!`, `println!`); introducing `macro_rules!` declarative macros.
- **Teaches**: macros are compile-time code generation; `$e:expr` / `$t:ty` matchers; hygienic expansion.

### `clippy`
- A set of exercises that only compile after you clean up idiomatic-lint warnings from `clippy`.
- **Teaches**: canonical idioms as enforced by the community's linter — e.g., `if let Some(x) = opt` instead of matching, `.is_empty()` instead of `.len() == 0`.
- **Side effect**: learners pick up the "Rust way" of writing things because clippy tells them.

### `conversions`
- `From`/`Into`, `TryFrom`/`TryInto`, `AsRef`/`AsMut`, `as` casts, string conversions (`parse::<T>()`).
- **Teaches**: the canonical conversion traits. Impl `From`, get `Into` for free; `TryFrom` for fallible; `?` + `From` for error bubbling.

### `quiz1` .. `quiz4`
- Mini integration exercises combining the previous categories.
- **quiz1** — variables, if, functions, primitive types.
- **quiz2** — vecs, strings, move semantics, iterators.
- **quiz3** — error handling + generics + traits.
- **quiz4** — lifetimes + smart pointers + closures.
- **Purpose**: force cross-category synthesis.

## 4.2 Rustlings pedagogical signals worth copying

- **"Failing compile" as onboarding.** The learner is forced to read the compiler error, understand it, and fix it. This is how real Rust development feels.
- **`// I AM NOT DONE` marker.** Exercise file starts with this comment; removing it signals "grade this".
- **Hints command (`rustlings hint`)** gives progressive help without giving away the answer.
- **Watch mode (`rustlings watch`)** live-reloads on save; next exercise advances automatically.
- **Order matters.** The `00_`…`23_` folder order on [main](https://github.com/rust-lang/rustlings/tree/main/exercises) encodes dependency order (e.g. **smart_pointers** before **threads**). Teaching far out of this order usually fails for new learners.

---

# 5. Error handling — combined digest

Pulled from Comprehensive Rust **Day 4 PM** (Fundamentals) + Cookbook About (`anyhow`/`thiserror`) + Cookbook error recipes.

## 5.1 The layered decision tree

```
Is this error a *bug* (programmer mistake, invariant violation)?
    YES -> panic! / unreachable! / assert!
    NO -> Is this a library?
        YES (library) -> Return Result<T, MyError>
                         Implement std::error::Error for MyError
                         Use thiserror to cut boilerplate.
        NO (application) -> Return Result<T, anyhow::Error>
                            Use .context("what you were doing") at every layer.
                            Optionally color-eyre for prettier reports.
```

## 5.2 `thiserror` template (library)

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found at {path}")]
    NotFound { path: std::path::PathBuf },

    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid value for field `{field}`: {reason}")]
    Validation { field: &'static str, reason: String },
}
```

`#[from]` gives you free `From<toml::de::Error>` and `From<std::io::Error>` — `?` now bubbles those up into `ConfigError`.

## 5.3 `anyhow` template (application)

```rust
use anyhow::{Context, Result};

fn load_config(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    let cfg: Config = toml::from_str(&text)
        .with_context(|| "parsing config as TOML")?;
    Ok(cfg)
}

fn main() -> Result<()> {
    let cfg = load_config(Path::new("app.toml"))?;
    run(cfg)
}
```

On failure you get:

```
Error: reading config at app.toml

Caused by:
    0: parsing config as TOML
    1: expected `=`, found `{` at line 3 column 12
```

## 5.4 `?` operator mechanics

- Desugars to roughly `match expr { Ok(v) => v, Err(e) => return Err(From::from(e)) }`.
- Works with `Result<T, E>` and `Option<T>` (but not across the two in one expression; use `.ok_or(err)?` to bridge).
- Requires the function return type to be the target type.
- In closures: `?` inside `|| -> Result<..>` works fine; inside `|| -> Option<_>` works fine; but in non-Result-returning closures, use `.ok()` / `.map_err`.

## 5.5 `Result` combinators worth memorizing

```
map         T -> U            transform Ok
map_err     E -> F            transform Err
and_then    T -> Result<U, E> flat-map Ok
or_else     E -> Result<T, F> flat-map Err
unwrap_or(default)            extract or fallback
unwrap_or_else(|e| ...)       extract or compute fallback
unwrap_or_default()           extract or Default
ok()                          Result -> Option
err()                         Result -> Option<E>
transpose()                   Result<Option<T>> <-> Option<Result<T>>
?                             early return / From-convert
```

## 5.6 `main` signatures

```rust
fn main() {}                                         // panics not caught specially
fn main() -> Result<(), Box<dyn Error>> { ... }      // basic
fn main() -> anyhow::Result<()> { ... }              // library-friendly
fn main() -> ExitCode { ... }                        // custom exit code
```

Returned `Err` from `main` prints via `Debug` (that's why `anyhow::Error`'s `Debug` impl shows chain+context).

## 5.7 Panics you should keep

- `unreachable!()` — a branch the type system cannot eliminate but you know can't happen.
- `panic!("invariant violated: ...")` — truly exceptional; your data structure is corrupted.
- `assert!` / `debug_assert!` — sanity checks; `debug_assert!` is no-op in release.
- `.unwrap()` on `Option::Some`/`Ok` after you've *proved* the invariant (prefer `.expect("justification")`).

## 5.8 Error-handling antipatterns

- Stringly-typed errors (`Result<T, String>`) in libraries — caller loses all structured info.
- Dropping error chains via `.map_err(|_| MyErr::Generic)` — loses the cause.
- Catching-all with `Result<T, Box<dyn Error>>` in libraries — callers can't pattern-match; use a typed enum.
- `unwrap()` in libraries on user input — library panics on API caller's mistake.
- Mixing `anyhow` into library API — forces downstream to use `anyhow` too.

---

# 6. Async & concurrency — digest

## 6.1 Decision matrix: concurrency primitive selection

| Situation | Use |
|---|---|
| Pure CPU-bound, data-parallel | `rayon` `par_iter` |
| Task-parallel, CPU-bound | `std::thread::scope` + `crossbeam-channel` |
| Many I/O tasks, I/O-bound | `tokio` + `async/await` |
| Network server | `tokio` + `axum` / `hyper` |
| Embedded, no std | `embassy` |
| One-shot background job from sync code | `std::thread::spawn` |
| Pipeline (stage1 → stage2 → stage3) | `crossbeam-channel` between thread::scope workers |
| Producer/consumer in async | `tokio::sync::mpsc::channel` |
| Shared state, many readers | `Arc<RwLock<T>>` or `Arc<parking_lot::RwLock<T>>` |
| Shared state, many writers | `Arc<Mutex<T>>` |
| Shared state, atomic counter | `Arc<AtomicUsize>` |
| Lazy global init (thread-safe) | `std::sync::OnceLock<T>` |
| Write-once after init, read-many | `std::sync::OnceLock<T>` or `arc-swap::ArcSwap<T>` |
| Channel with async + sync sides | `flume` |

## 6.2 Async mental model (experienced-dev framing)

- An `async fn` is a **state-machine factory**: calling it returns a `Future` struct whose fields are the locals live across `.await` points.
- `.await` does not call a function — it *suspends* the current state machine when a sub-future returns `Poll::Pending`.
- Futures are **lazy**: nothing runs until polled by an executor. `async { ... }` not awaited = no-op.
- `tokio::spawn` polls the future to completion on the runtime; detached if `JoinHandle` dropped.
- **Cooperative scheduling**: no `.await` ⇒ no yield. CPU-bound tight loop inside async = starves other tasks. Fix: `tokio::task::yield_now().await` or `spawn_blocking`.
- **`Send` propagation**: a future is `Send` iff every local held across `.await` is `Send`. That's why `Rc`, `RefCell`, non-Send guards are toxic in multi-threaded runtimes.
- **Pin is an immovability guarantee.** Address stability matters because the future holds self-references between the state-machine fields. You rarely write `Pin` by hand; `tokio::spawn`, `Box::pin`, and `pin!` hide it.

## 6.3 Tokio core API cheat-sheet

```rust
#[tokio::main] async fn main() { ... }                          // multi-threaded runtime
#[tokio::main(flavor = "current_thread")] async fn main() {}    // single-thread
tokio::spawn(async { ... });                                    // new task
tokio::task::spawn_blocking(|| heavy_sync_work());              // blocking pool
tokio::time::sleep(Duration::from_millis(100)).await;
tokio::time::timeout(dur, fut).await                            // Result<T, Elapsed>
tokio::select! { a = fut1 => ..., b = fut2 => ... };            // race
let (tx, mut rx) = tokio::sync::mpsc::channel::<T>(100);        // bounded MPSC
let (tx, _) = tokio::sync::broadcast::channel::<T>(100);        // broadcast
let (tx, rx) = tokio::sync::oneshot::channel::<T>();            // one-shot
let (tx, rx) = tokio::sync::watch::channel::<T>(initial);       // latest-value
let mut s = TcpListener::bind("0.0.0.0:8080").await?;
```

## 6.4 Common async pitfalls (Comprehensive Rust emphasizes)

1. **Blocking in async.** `std::thread::sleep` in async = stalls worker. Use `tokio::time::sleep`.
2. **`std::sync::Mutex` held across `.await`.** Future becomes non-`Send`; on multi-thread runtime it won't even compile if spawned. Use `tokio::sync::Mutex` only when lock must span an await; otherwise drop guard before await.
3. **Cancellation leaves partial state.** `.await` can be cancelled at drop; your `Drop` must restore invariants. This is why persistent DB transactions inside cancellable futures are risky.
4. **Cooperative starvation.** Long CPU loops starve other tasks. Either `spawn_blocking` or `yield_now`.
5. **`select!` drops losing futures.** Losers are cancelled mid-flight. If you need re-entry, use `futures::future::FutureExt::fuse` + `select!` with `&mut`-biased.
6. **Async trait methods** — before Rust 1.75 only via `async-trait` crate (box + heap alloc per call). Now native, but trait objects (`dyn MyAsyncTrait`) still need the crate or manual boxing.

## 6.5 Threads without async — canonical patterns

```rust
// Scoped threads: borrow from stack
std::thread::scope(|s| {
    let data = &shared;
    s.spawn(|| work(data));
    s.spawn(|| work(data));
});

// Arc + Mutex for shared mutable state
let state = Arc::new(Mutex::new(0));
let handles: Vec<_> = (0..N).map(|_| {
    let st = Arc::clone(&state);
    std::thread::spawn(move || { *st.lock().unwrap() += 1; })
}).collect();
for h in handles { h.join().unwrap(); }

// Channel pipeline
use crossbeam_channel as cc;
let (tx1, rx1) = cc::unbounded();
let (tx2, rx2) = cc::unbounded();
std::thread::scope(|s| {
    s.spawn(|| for item in source { tx1.send(item).unwrap(); });
    s.spawn(|| for item in rx1 { tx2.send(transform(item)).unwrap(); });
    s.spawn(|| for item in rx2 { sink(item); });
});
```

## 6.6 Send/Sync rules of thumb

- `&T: Send` iff `T: Sync`. ⇒ `T: Sync` means "safe to share by reference across threads".
- `Rc<T>`: not `Send`, not `Sync`.
- `Arc<T>`: `Send + Sync` iff `T: Send + Sync`.
- `RefCell<T>`: not `Sync`. (It's `Send` if `T: Send`.)
- `Mutex<T>`: `Sync` (even for `!Send` `T`? No — `T: Send` still required).
- Raw pointers: not `Send`, not `Sync`.
- `MutexGuard<'_, T>`: not `Send` on some platforms; treat as single-thread.

---

# 7. Performance — tips culled from the three sources

## 7.1 Cookbook-implied perf patterns

- **Prefer `sort_unstable` / `sort_unstable_by` over `sort`** when stable ordering is not required. Faster and less memory.
- **`par_iter` for CPU-bound map/filter/reduce**. Cookbook's "SHA1 of ISO files" and "parallel thumbnails" recipes are the canonical examples.
- **Use `BufReader` / `BufWriter`** when reading/writing files line-by-line; unbuffered `File` issues a syscall per op.
- **`Vec::with_capacity(n)` when size is known.** Avoids repeated realloc.
- **`String::with_capacity(n)`** likewise.
- **`collect::<Vec<_>>()` eagerly** — laziness is a feature of iterators but eager collect amortizes allocation.
- **`HashMap::with_capacity(n)`** — avoid rehashes.
- **Prefer `&str` over `String` in function signatures** (accept `impl AsRef<str>` or `&str`; caller chooses ownership).
- **Pass by `&[T]` not `&Vec<T>`**; slice is the right abstraction.

## 7.2 Comprehensive Rust perf notes

- **Monomorphization** — generic code becomes specialized; no virtual call overhead; pays code-size cost.
- **`impl Trait` in return** — monomorphized; cheap. `Box<dyn Trait>` — virtual call; still cheap but not inlined.
- **Iterator chains** optimize as well as hand-written loops in practice (often better, because LLVM sees the whole chain).
- **Inlining** — `#[inline]` suggests; `#[inline(always)]` forces; `#[inline(never)]` suppresses. Use sparingly.
- **Ownership eliminates GC cost** — allocations are explicit, drops are at scope boundaries; predictable latency.
- **Bounds checks** — Rust inserts them; LLVM eliminates in loops where bounds are provable; use `iter()` over indexing for guaranteed elimination.

## 7.3 Async perf considerations (Concurrency deep-dive / Tokio ecosystem)

- Tokio multi-threaded runtime defaults to `num_cpus` workers.
- Each task has small per-task memory (~state machine size).
- `spawn` is cheap; don't be afraid to spawn thousands.
- Heavy work blocks the worker — use `spawn_blocking` (separate blocking pool, default 512 threads).
- `Arc<AtomicUsize>` is cheaper than `Arc<Mutex<usize>>` for counters.
- `parking_lot::Mutex` is smaller (1 byte) and faster than `std::sync::Mutex` in uncontended cases.

## 7.4 Profiling (implicit best practices)

- `cargo build --release` for any benchmark.
- `criterion` for microbenchmarks — statistical, warmup, HTML reports.
- `flamegraph` (`cargo flamegraph`) — on Linux/macOS via perf.
- `samply` — perf profiler producing Firefox Profiler JSON.
- `tokio-console` for async task inspection.
- `heaptrack` / `dhat` for allocation profiling.

---

# 8. Testing & tooling — digest

## 8.1 Rustlings `tests` + `clippy` categories

**Rustlings `tests` teaches:**
- `#[test]` fn; `#[cfg(test)] mod tests { ... }` colocation; `cargo test`.
- `assert!` / `assert_eq!` / `assert_ne!` and the optional format-message second arg.
- `#[should_panic(expected = "substring")]`.
- `-- --test-threads=1` for serial tests.
- Test discovery via path pattern: `cargo test partial_name`.

**Rustlings `clippy` teaches:**
- Idiomatic idioms via lints:
  - `if let Some(x) = opt` instead of `match opt { Some(x) => ..., None => () }`.
  - `iter().any(|x| ...)` instead of `.iter().filter(|x| ...).count() > 0`.
  - `.is_empty()` over `.len() == 0`.
  - `.clone()` only when necessary (avoid `x.clone()` if `x` is Copy).
  - `.to_owned()` vs `.to_string()` vs `String::from` — contextual.
  - Avoid `unwrap()` — prefer `expect` with justification or `?`.
  - `#[derive(Default)]` when you have a zero-state constructor.

## 8.2 `cargo test` ecosystem

- **Unit tests** colocated; **integration tests** in `tests/`; **doc tests** in `///`.
- **`#[cfg(test)]`** for test-only code paths.
- **Test harness customization** — `harness = false` in `Cargo.toml` + write your own `main`.
- **Mockall** — canonical mocking crate (`#[automock]` on a trait).
- **`proptest` / `quickcheck`** — property-based testing.
- **`insta`** — snapshot testing.
- **`assert_cmd` + `predicates`** — CLI integration testing.
- **`rstest`** — parametrized tests (fixtures + table-driven).
- **`wiremock`** — HTTP mock server for integration testing.

## 8.3 Coverage

- `cargo-tarpaulin` (Linux-leaning) or `cargo-llvm-cov` (cross-platform, LLVM source-based).

## 8.4 Linting and formatting

- `cargo fmt` — `rustfmt`, zero-configuration by default (use `rustfmt.toml` to override); canonical code style.
- `cargo clippy` — lints across 5 groups: correctness (error), suspicious (warn), style (warn), complexity (warn), perf (warn), pedantic (allow), restriction (allow), nursery (allow).
- **Enable pedantic in new projects cautiously**: `#![warn(clippy::pedantic)]` at crate root.
- **Deny warnings in CI**: `cargo clippy -- -D warnings`.

## 8.5 Cargo tooling ecosystem

| Tool | Purpose |
|---|---|
| `cargo fmt` | Format (rustfmt) |
| `cargo clippy` | Lint |
| `cargo test` | Tests |
| `cargo doc --open` | Build + open docs |
| `cargo bench` | Nightly bench harness |
| `cargo tree` | Dep graph |
| `cargo expand` | Macro expansion viewer |
| `cargo watch` | Rebuild on change |
| `cargo edit` (`cargo add`/`rm`/`upgrade`) | Dep management (also native now) |
| `cargo audit` | Security advisory scanner |
| `cargo deny` | License / advisory / version policy |
| `cargo outdated` | Check for newer deps |
| `cargo nextest` | Faster test runner |
| `cargo criterion` | Criterion-integrated bench |
| `cargo flamegraph` | Profiler |
| `cargo udeps` | Unused deps detector (nightly) |
| `cargo machete` | Unused deps (stable alternative) |
| `cargo-semver-checks` | Semver compatibility checker |
| `cargo-msrv` | Minimum Supported Rust Version finder |
| `cargo-release` | Release workflow |
| `cargo-workspaces` | Workspace management |

---

# 9. Ecosystem crate picks — canonical one-liners (derived from Cookbook choices)

This is the crate-per-task list, deduplicated and tightened for quick reference by an LLM.

## 9.1 By category

### Serialization / parsing
- Universal trait: `serde` + derive.
- JSON: `serde_json`. TOML: `toml`. YAML: `serde_yaml`. MessagePack: `rmp-serde`. Bincode: `bincode`. XML: `quick-xml`. CSV: `csv`.
- Protobuf: `prost` (tokio-style) or `rust-protobuf`.
- For config: `config` (merges sources) or `figment` (Rocket's config).

### Error handling
- Library: `thiserror`. Application: `anyhow` (or `color-eyre` for pretty reports).

### Logging / tracing
- Simple: `log` + `env_logger`.
- Structured + async: `tracing` + `tracing-subscriber`.
- Production JSON: `tracing-subscriber` with `fmt::json()` layer.

### HTTP
- Client: `reqwest` (default). Minimal sync client: `ureq`.
- Server: `axum` (default ergonomic) or `actix-web` (performance-forward) or `rocket`.
- Lower-level: `hyper` (both client and server building block).
- WebSocket: `tokio-tungstenite`.
- gRPC: `tonic`.

### Async runtime
- Default: `tokio` (full feature set).
- Lightweight: `smol` or `async-std`.
- Embedded: `embassy`.

### Concurrency primitives
- Parallel iteration: `rayon`.
- Channels: `crossbeam-channel` (sync MPMC) or `flume` (sync+async).
- Locks: `parking_lot::{Mutex, RwLock}` (faster, no poisoning).
- Shared lazy state: `once_cell::sync::Lazy` (or std `OnceLock`).
- Atomics: std.

### Database
- SQLite (sync): `rusqlite`.
- Postgres: `postgres` (sync) / `tokio-postgres` (async).
- MySQL: `mysql_async`.
- ORM (sync, compile-time SQL): `diesel`.
- ORM (async, compile-time raw SQL): `sqlx`.
- Active-record ORM: `sea-orm`.
- Migrations: `refinery`, `sea-orm-migration`, `sqlx migrate`.

### CLI
- Arg parsing: `clap` (derive API).
- Terminal coloring: `owo-colors`.
- Progress: `indicatif`.
- Prompts: `dialoguer`.
- TUI: `ratatui`.

### Date/time
- Default: `chrono` (mature, timezones via `chrono-tz`).
- Modern: `time` (cleaner API, no TZ-on-by-default).
- Humanized: `humantime`.

### Random
- `rand` for RNG. `rand_distr` for non-uniform. `uuid` for UUIDs.

### Crypto
- Hashes: `sha2` / `blake3` / `md-5`.
- Password: `argon2` (current best); `bcrypt` legacy; `scrypt` also acceptable.
- HMAC: `hmac`.
- Symmetric: `aes-gcm`, `chacha20poly1305`.
- TLS: `rustls` (pure Rust); `native-tls` (system backing).
- General curated: `ring`.
- X.509: `x509-parser`, `rustls-pemfile`.

### Text
- Regex: `regex`.
- Unicode: `unicode-segmentation`, `unicode-normalization`.
- String fuzzy matching: `strsim`.

### FS / paths
- Walk: `walkdir`.
- Glob: `glob` / `globset`.
- Temp: `tempfile`.
- Memmap: `memmap2`.
- Ignore-respecting walk: `ignore`.

### Numerics
- `ndarray` (NumPy-like), `nalgebra` (linear algebra), `num-bigint`, `num-complex`, `num-rational`, `num-traits`.

### Utility traits / macros
- `itertools`, `strum` (enum utilities), `bitflags`, `derive_more`, `smart-default`.

### Testing
- `proptest`, `quickcheck`, `mockall`, `insta`, `assert_cmd`, `rstest`, `wiremock`, `criterion`, `nextest`.

### FFI
- C consumption: `bindgen` + `libc`.
- C emission: `cbindgen`.
- Python: `pyo3`.
- Node: `napi-rs`.
- JVM: `jni`.
- WASM/browser: `wasm-bindgen` + `js-sys` + `web-sys`.

## 9.2 "If someone says X, reach for Y" (compressed intuition)

| User need | Default crate pick |
|---|---|
| "Parse JSON" | serde_json |
| "Parse YAML" | serde_yaml |
| "Parse TOML config" | toml + serde |
| "Read CSV" | csv + serde |
| "Regex match" | regex |
| "HTTP client" | reqwest |
| "Web server" | axum |
| "Database (Postgres)" | sqlx (async) or diesel (sync) |
| "Write async code" | tokio |
| "Parallel computation" | rayon |
| "Logging" | tracing |
| "CLI args" | clap |
| "Errors in lib" | thiserror |
| "Errors in app" | anyhow |
| "Date/time" | chrono or time |
| "Random numbers" | rand |
| "Hash passwords" | argon2 |
| "UUID" | uuid |
| "Temp file" | tempfile |
| "Walk directory" | walkdir |
| "Read PDF/image/audio" | pdf/image/symphonia |
| "Fast mutex" | parking_lot |
| "Shared channel" | crossbeam-channel |
| "Global lazy" | once_cell (or std OnceLock) |
| "Numeric array" | ndarray |
| "Linear algebra" | nalgebra |
| "Bitflags" | bitflags |
| "Iterator tools" | itertools |
| "Enum derive Display/FromStr" | strum |
| "CSV parse into struct" | csv + serde |
| "Markdown to HTML" | pulldown-cmark |
| "Template HTML" | askama (compile-time) or tera (runtime) |
| "Postgres migrations" | sqlx migrate or refinery |
| "HTTP mock in tests" | wiremock |
| "Property-based tests" | proptest |
| "Snapshot tests" | insta |
| "Mock trait in tests" | mockall |
| "Benchmark" | criterion |
| "Semver-check my API" | cargo-semver-checks |
| "Minimum supported Rust version" | cargo-msrv |

---

# 10. Cross-cutting: "Rust-ness" principles the three resources reinforce

These are the meta-level lessons that appear repeatedly across all three sources. Memorize these as the prior-before-writing-Rust-code.

1. **Ownership > everything.** Before writing a function, ask: who owns the output? Who owns the inputs after? Return owned `Vec<T>`/`String` when caller needs to keep it; take `&[T]`/`&str` otherwise.

2. **Prefer borrows in function signatures.** `fn f(s: &str)` beats `fn f(s: String)` because callers with `String` can pass `&s` via deref coercion, and callers with `&str` are not forced to allocate.

3. **`impl AsRef<Path>` / `impl AsRef<str>`** in APIs that accept paths/strings — lets callers pass `&str`, `String`, `Path`, `PathBuf`, `&Path` interchangeably.

4. **Newtype for invariants.** `struct NonEmpty<T>(Vec<T>)` + smart constructor; exposes only operations that maintain non-emptiness.

5. **Make invalid states unrepresentable.** Replace `struct Req { body: String, is_json: bool }` with `enum Body { Json(Value), Text(String), Binary(Vec<u8>) }`.

6. **Type-state pattern for protocols.** `Connection<Connected>` vs `Connection<Disconnected>` — `send` only implemented on the former. `Builder<Missing, Missing>` vs `Builder<Set, Set>` — `.build()` only on the latter.

7. **Prefer iterators to loops.** Faster, safer, more declarative, and composable. Use `collect::<Vec<_>>()` or `into_iter().fold(...)` to terminate.

8. **Prefer `match` to `if`.** Exhaustive, compiler-checked. `if let` when you only care about one arm.

9. **Use `?` liberally.** The prettier the error path, the more you'll actually handle errors right.

10. **Accept references, return owned values.** Standard function-signature shape: `fn process(input: &InputType) -> OutputType`.

11. **Small traits over big ones.** `Iterator` is 1 method; everything else is a default. Follow the pattern: 1-5 required methods max.

12. **Avoid `unsafe` unless required.** When you must, wrap in a safe-API module and document the invariants the caller doesn't need to know.

13. **`Cargo.lock` is part of your binary crate; *not* your library crate.** For apps, commit it; for libs, gitignore (or not — modern guidance says commit either way for reproducibility).

14. **Use features sparingly.** Cargo feature flags multiply combinatorially; test at least `--no-default-features` and `--all-features`.

15. **Don't write an async trait from scratch** — use `async-trait` crate or recent native `async fn in Trait` if MSRV permits.

16. **Don't hold a `Mutex` across `.await`.** Either finish the work under the lock, or use `tokio::sync::Mutex`.

17. **`Rc`/`RefCell` are not `Send`.** Anywhere you want thread-safe shared mutable state: `Arc<Mutex<T>>` (thread) or `Arc<tokio::sync::Mutex<T>>` (async).

18. **`String` is always UTF-8.** You can't build an invalid `String`. Byte-slicing into a `String` will panic at a non-char boundary.

19. **`Path` / `PathBuf` are OS-native**, not UTF-8. To round-trip via string, use `.to_string_lossy()` or handle `Option<&str>` from `.to_str()`.

20. **Code that compiles is probably correct.** The joke is old but the rate of "compiled and worked first time" is unusually high in Rust — because the compiler forces you to think about ownership, error paths, and aliasing up-front.

---

# 11. Rust language-feature "atlas" induced from the three sources

A single-page mental model of the Rust-specific surface area covered across all three:

```
OWNERSHIP
  owned T
  &T (shared, Copy, many)
  &mut T (exclusive, one)
  Box<T> (heap-owned)
  Rc<T> / Arc<T> (refcounted)
  Cell<T> / RefCell<T> / Mutex<T> / RwLock<T> (interior mutability)

TYPES
  scalar (i*/u*/f*/bool/char)
  compound (tuple, array, struct, enum)
  collection (Vec, VecDeque, HashMap, BTreeMap, HashSet, BTreeSet, String)
  pointer (&T, &mut T, *const T, *mut T, Box<T>, Rc<T>, Arc<T>)
  trait object (dyn Trait)
  slice (&[T], &mut [T], &str)
  fn pointer (fn(T) -> U)
  closure (Fn, FnMut, FnOnce)
  future (impl Future<Output = T>)
  iterator (impl Iterator<Item = T>)

CONTROL FLOW
  if / if let / while / while let / loop / for / match / let else

PATTERN MATCHING
  literal, range, binding, wildcard, rest, struct, tuple, enum, guard, or-pattern, @-binding

GENERICS & TRAITS
  fn<T: Bound>, where clauses
  trait + impl + default methods
  associated types / consts
  supertraits / subtraits
  orphan rule + newtype escape hatch
  derives (Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)
  marker traits (Send, Sync, Copy, Unpin, Sized)
  coherence

LIFETIMES
  explicit 'a
  elision
  'static
  HRTB: for<'a> Fn(&'a T)
  variance (co/contra/invariant) — advanced

ERROR HANDLING
  Result<T, E>, Option<T>
  ? operator
  panic! / unwrap / expect / assert / debug_assert / unreachable
  thiserror, anyhow patterns

MODULES & CRATES
  mod, use, pub, pub(crate), pub(super), pub(in path)
  crate tree, foo.rs vs foo/mod.rs
  Cargo workspaces, features, dependencies

ASYNC
  async fn, .await
  Future, Pin, poll, Context, Waker
  executors (tokio, smol, async-std, embassy)
  task, select!, join!, tokio::spawn
  async trait (async-trait or native)

CONCURRENCY
  thread::spawn, scope
  channels (mpsc, crossbeam, flume)
  atomics (AtomicUsize, Ordering)
  Mutex, RwLock, Condvar, Barrier
  Send / Sync
  rayon parallelism

UNSAFE
  unsafe fn / unsafe block / unsafe impl / unsafe trait
  raw pointers
  FFI (extern "C")
  union, transmute
  miri for verification

MACROS
  declarative (macro_rules!)
  procedural (derive, attribute, function-like)
  syn / quote / proc-macro2

TOOLING
  cargo (build/run/test/bench/doc/check/clippy/fmt)
  clippy lint groups
  rustfmt
  rust-analyzer (IDE backend)
  cargo-tools (nextest, tarpaulin, audit, deny, etc.)
```

---

# 12. Appendix A — Cookbook recipe-by-recipe crate index (super-dense)

(Organized in the official Cookbook order for quick lookup. Each line: `recipe` → `crate(s)`.)

```
ALGORITHMS / RANDOMNESS
  Generate random numbers                                   rand
  Generate random numbers within a range                    rand
  Generate random numbers with a given distribution         rand + rand_distr
  Generate random values of custom type                     rand
  Create random password (alphanumeric)                     rand
  Create random password (user-defined set)                 rand

ALGORITHMS / SORTING
  Sort vector of integers                                   std
  Sort vector of floats                                     std (partial_cmp)
  Sort vector of structs                                    std (derive Ord or sort_by)

COMMAND LINE
  Parse command-line arguments                              clap
  ANSI Terminal (color)                                     ansi_term / owo-colors

COMPRESSION
  Decompress tarball                                        flate2 + tar
  Compress directory into tarball                           flate2 + tar
  Decompress tarball stripping prefix                       flate2 + tar

CONCURRENCY / EXPLICIT THREADS
  Spawn short-lived thread                                  std + crossbeam (scoped; now std::thread::scope)
  Create parallel pipeline                                  crossbeam (channels + scope)
  Pass data between two threads                             std::sync::mpsc or crossbeam-channel
  Maintain global mutable state                             lazy_static or once_cell

CONCURRENCY / DATA PARALLEL (rayon)
  Mutate elements of array in parallel                      rayon
  Test if any/all match predicate                           rayon
  Search items with predicate                               rayon
  Sort vector                                               rayon
  Map-reduce                                                rayon
  Thumbnail jpgs in parallel                                rayon + image + glob

CONCURRENCY / ASYNC (Cookbook advanced recipes)
  Actor pattern with Tokio                                  tokio
  Custom Future (Pin, Waker, Poll)                          std::task, std::pin, std::future

CRYPTOGRAPHY
  SHA-256 digest of file                                    ring (or sha2)
  HMAC sign/verify                                          ring (or hmac + sha2)
  PBKDF2 password salt/hash                                 ring (or pbkdf2 + sha2)

DATABASE / SQLITE
  Create table                                              rusqlite
  Insert and query                                          rusqlite
  Transaction                                               rusqlite

DATABASE / POSTGRES
  Create table                                              postgres
  Insert and query aggregate data                           postgres

DATE/TIME / DURATION
  Measure elapsed time                                      std (Instant)
  Checked date/time calculations                            chrono
  Convert local time to another timezone                    chrono + chrono-tz
  Examine date/time components                              chrono
  Convert to/from Unix timestamp                            chrono
  Display formatted date/time                               chrono
  Parse string into DateTime                                chrono

DEV TOOLS / DEBUGGING
  Log debug message                                         log + env_logger
  Log error message                                         log + env_logger
  Log to stdout instead of stderr                           log + env_logger
  Enable log levels per module                              log + env_logger
  Use custom env var for log level                          log + env_logger
  Log to custom location                                    log4rs (or fern)
  Log to Unix syslog                                        syslog

DEV TOOLS / VERSIONING
  Parse and increment string version                        semver
  Parse complex version string                              semver
  Check external command version for compatibility          semver + std::process

DEV TOOLS / BUILD-TIME
  Compile and link static C library                         cc + build.rs
  Compile and link static C++ library                       cc + build.rs
  Compile C library with custom defines                     cc + build.rs

ENCODING / STRINGS
  Percent-encode URL string                                 percent-encoding
  application/x-www-form-urlencoded encoding                url (form_urlencoded)
  Encode/decode hex                                         hex (or data-encoding)
  Encode/decode base64                                      base64

ENCODING / CSV
  Read CSV records                                          csv
  Read with different delimiter                             csv
  Filter records matching predicate                         csv
  Handle invalid data with serde                            csv + serde
  Serialize records to CSV                                  csv
  Serialize using serde                                     csv + serde
  Transform column                                          csv + serde

ENCODING / STRUCTURED
  Serialize/deserialize unstructured JSON                   serde_json
  Deserialize TOML config                                   toml + serde
  Read/write integers in LE byte order                      byteorder

ERROR HANDLING
  Handle errors correctly in main                           Box<dyn Error> (or anyhow)
  Avoid discarding errors during conversions                std (From/Into)
  Obtain backtrace                                          std::backtrace + RUST_BACKTRACE

FILE SYSTEM
  Read lines from file                                      std (BufReader)
  Avoid write-read same file                                tempfile / same-file
  Memory-mapped access                                      memmap2
  Files modified in last 24h                                walkdir + std::time
  Recursively find duplicate filenames                      walkdir
  Recursively find files matching predicate                 walkdir
  Traverse skipping dotfiles                                walkdir
  Recursively calculate file sizes                          walkdir
  Find symlink loops for path                               same-file

HARDWARE
  Check number of logical CPU cores                         num_cpus

DATA_STRUCTURES
  Bitfield of flags (C-style)                               bitflags

MEMORY MGMT (lazy globals / interior mutability patterns)
  Lazily evaluated static / one-time init                   once_cell / lazy_static / std::sync::OnceLock
  `Cell` / `RefCell` patterns (cookbook labels)             std::cell
  `LazyCell` / `LazyLock`                                   std (1.80+ LazyLock on stable path)

NETWORK (server)
  Listen on unused TCP/IP port                              std

NETWORK (client)
  HTTP GET                                                  reqwest
  Download file to temp directory                           reqwest + tempfile
  Query GitHub API                                          reqwest + serde_json
  HTTP Basic Auth                                           reqwest

URL
  Parse URL                                                 url
  Base + relative                                           url
  Origin                                                    url
  Remove fragment                                           url
  Download via HTTPS                                        reqwest (rustls-tls)

OS
  Run piped external commands                               std::process
  Redirect stdout/stderr of child to file                   std::process
  Continuously process child outputs                        std::process
  Read environment variable                                 std::env

SCIENCE / LINEAR ALGEBRA
  Vector math                                               ndarray
  Matrix computations                                       ndarray

TEXT / PROCESSING
  Collect unicode graphemes                                 unicode-segmentation

TEXT / REGEX
  Verify/extract login from email                           regex + lazy_static
  Extract unique hashtags                                   regex + lazy_static
  Extract phone numbers                                     regex
  Filter log by regex                                       regex
  Replace text pattern                                      regex

WEB / SCRAPING
  Extract all links from HTML                               reqwest + select / scraper
  Check webpage for broken links                            reqwest + scraper
  Extract unique MediaWiki article links                    reqwest + regex

WEB / MIME
  Get MIME type from string                                 mime
  Get MIME type from filename                               mime_guess
  Parse HTTP response MIME type                             mime + reqwest

WEB / REQUESTS
  Check resource exists                                     reqwest (HEAD)
  Custom User-Agent                                         reqwest
  Partial download (Range)                                  reqwest
  Rate-limited API                                          reqwest + std::thread::sleep
  POST file to paste-rs                                     reqwest

WEB / FULL STACK (Leptos)
  Filtered HTML results / server-synchronized components    leptos
```

---

# 13. Appendix B — Rustlings exercise file map (approximate, based on well-known structure)

```
exercises/
  00_intro/                 — intro1, intro2 — format macros, cargo basics
  01_variables/             — variables1..6 — let, mut, shadowing, const
  02_functions/             — functions1..5 — declaration, return, params
  03_if/                    — if1..3 — if-expressions
  04_primitive_types/       — primitive_types1..6 — bool, char, tuple, array, slice
  05_vecs/                  — vecs1..2 — Vec construction, iteration
  06_move_semantics/        — move_semantics1..6 — move, borrow, return ownership
  07_structs/               — structs1..3 — named/tuple/unit structs, methods
  08_enums/                 — enums1..3 — variants, match, if let
  09_strings/               — strings1..4 — String vs &str conversion
  10_modules/               — modules1..3 — mod, pub, use
  11_hashmaps/              — hashmaps1..3 — insert/get/entry
  12_options/               — options1..3 — Some/None, ?, combinators
  13_error_handling/        — errors1..6 — Result, custom error, From, ?
  14_generics/              — generics1..2 — generic fn/struct, bounds
  15_traits/                — traits1..5 — defining/implementing, dyn Trait
  16_lifetimes/             — lifetimes1..3 — explicit lifetime annotations
  17_tests/                 — tests1..4 — #[test], assert_eq, should_panic
  18_iterators/             — iterators1..5 — map, filter, collect, impl Iterator
  19_smart_pointers/        — box1, rc1, arc1, cow1 — heap, shared, interior mutability
  20_threads/               — threads1..3 — spawn, Arc+Mutex, channels
  21_macros/                — macros1..4 — using macros, macro_rules!
  22_clippy/                — clippy1..3 — idiom cleanup
  23_conversions/           — using_as, from_into, from_str, try_from_into, as_ref_mut
  quizzes/                    — quiz1…quizN (consolidated under exercises/quizzes on current main)
```

Exact file names vary by Rustlings release; the **numbered `00_`…`23_` directory order** on [main](https://github.com/rust-lang/rustlings/tree/main/exercises) is authoritative. Quizzes may live under **`exercises/quizzes/`** rather than loose `quizN.rs` at repo root.

---

# 14. Appendix C — Comprehensive Rust TOC (aligned to official Course Structure)

**Top-level** ([site map](https://google.github.io/comprehensive-rust/)): Welcome, translations, PDF, **Rust Fundamentals** (Days 1–4), then **deep dives**: [Android](https://google.github.io/comprehensive-rust/android.html), [Chromium](https://google.github.io/comprehensive-rust/chromium.html), [Bare-metal](https://google.github.io/comprehensive-rust/bare-metal.html), [Concurrency](https://google.github.io/comprehensive-rust/concurrency/welcome.html), [Idiomatic Rust](https://google.github.io/comprehensive-rust/idiomatic/welcome.html), [Unsafe deep-dive](https://google.github.io/comprehensive-rust/unsafe-deep-dive/welcome.html) (WIP).

**Rust Fundamentals — Day 1**

- AM: Welcome; Hello, World; Types and Values; Control Flow Basics.
- PM: Tuples and Arrays; References; User-Defined Types.

**Rust Fundamentals — Day 2**

- AM: Pattern Matching; Methods and Traits; Generics.
- PM: Closures; Standard Library Types; Standard Library Traits.

**Rust Fundamentals — Day 3**

- AM: Memory Management; Smart Pointers.
- PM: Borrowing; Lifetimes.

**Rust Fundamentals — Day 4**

- AM: Iterators; Modules; Testing.
- PM: Error Handling; Unsafe Rust.

**Concurrency deep-dive (separate track)**

- AM: Threads; Channels; Send and Sync; Shared State; Exercises.
- PM: Async Basics; Channels and Control Flow; Pitfalls; Exercises.

**Instructor meta:** [Course Structure](https://google.github.io/comprehensive-rust/running-the-course/course-structure.html), [translations](https://google.github.io/comprehensive-rust/running-the-course/translations.html).

---

# 15. Appendix D — "Canonical first-day setup" the three sources imply

```bash
# Install
rustup default stable
rustup component add rustfmt clippy rust-src rust-analyzer

# New crate
cargo new my_project && cd my_project

# Good-defaults Cargo.toml
[dependencies]
anyhow = "1"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }

[dev-dependencies]
criterion = "0.5"
proptest = "1"
tempfile = "3"

# Pre-commit loop
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps --open
```

---

# 16. Key takeaways for an LLM consuming this knowledge base

1. When a user asks "how do I X in Rust", default to the **Cookbook recipe crate** from the table above. If that crate is unmaintained, the modern successor is in the secondary column.
2. When suggesting code structure, **lead with ownership design**: who owns the data, who borrows, what's the lifetime.
3. For errors: `thiserror` in libraries, `anyhow` in apps. `?` at every fallible call. `.context()` at every layer boundary.
4. For concurrency: `rayon` for CPU-parallel, `tokio` for I/O. Default to `Arc<Mutex<T>>` for shared mutable state; offer `parking_lot` for perf upgrade; `Arc<RwLock<T>>` for many-readers-one-writer.
5. For async: **never hold a `std::sync::Mutex` across `.await`**. Drop the guard first or use `tokio::sync::Mutex`.
6. For testing: colocate unit tests with source; integration tests in `tests/`; `cargo test` runs all three kinds (including doc tests).
7. For CLI: `clap` derive API; `tracing` for logging with levels; `anyhow::Result<()>` from `main`.
8. When tempted to reach for `unsafe`: check whether a `RefCell`, `Pin`, `Box::leak`, arena, or newtype avoids it. Most `unsafe` in application code is unnecessary.
9. When generic bounds get messy: hide implementation behind a trait object (`Box<dyn MyTrait>`) or refactor into smaller type-state structs.
10. When async code is "hanging": check for (a) missing `.await`, (b) blocking inside async (missing `spawn_blocking`), (c) `Mutex` held across await.

---

# 17. Supplementary compression — “дожим” for LLM retrieval

Dense addenda: compiler diagnostics, tooling, std choices, and how Cookbook + Rustlings + Comprehensive Rust reinforce each other.

## 17.1 High-frequency `rustc` errors → usual fix (playbook)

| Error family | Typical situation | Fix pattern |
|--------------|-------------------|-------------|
| **E0382** use of moved value | Used `v` after `let x = v` or passed `v` into `fn` by value | Pass `&v` / `&mut v`, `.clone()` if cheap, or restructure so one owner |
| **E0499** / **E0502** cannot borrow mutably + immutably | Two `&mut` or `&`+`&mut` to same path | Shrink borrow scope; split struct fields; use two-phase borrows; `split_at_mut` for slices |
| **E0597** dropped while borrowed | Returned `&` to local / stack data | Return owned `String`/`Vec` or take `&'a` from caller-owned buffer |
| **E0515** / **E0621** lifetime mismatch | Returned reference outlives input | Add explicit `'a` tying return to param; `'static` only if truly static |
| **E0716** temporary dropped while borrowed | `let r = &vec![].push(...)` style | Bind intermediate to `let` binding first |
| **`Send` future not `Send`** | `Rc`/`RefCell`/`std::sync::MutexGuard` across `.await` | `Arc` + `tokio::sync::Mutex`; drop guard before await; `spawn_local` on single-thread runtime |
| **Trait bound not satisfied** | Generic `T` missing `Debug`/`Clone`/… | Add `where T: Trait` or `#[derive(...)]` on own types |
| **Orphan / coherence** | `impl Foreign for Foreign` | Newtype wrapper in your crate |

## 17.2 Rustlings CLI (operational)

| Command | Role |
|---------|------|
| `rustlings watch` | Re-run on save; advances when exercise passes |
| `rustlings run exercise_name` | Single exercise |
| `rustlings hint exercise_name` | Progressive hints (pedagogy: read error first) |
| `rustlings verify` | Full course check (CI-style) |
| `rustlings reset` | Restore exercise to pristine (when stuck) |

Install/update: follow [rustlings.rust-lang.org](https://rustlings.rust-lang.org) — usually `cargo install rustlings` or project-specific `Cargo.toml` in repo.

## 17.3 Std / API choices the Cookbook assumes you already know

| Need | Prefer | Avoid / note |
|------|--------|----------------|
| Owned text (UTF-8) | `String` | `OsString` unless OS-specific bytes |
| OS path | `Path` / `PathBuf` | Lossy `to_string_lossy()` only at display boundary |
| OS string (non-UTF8) | `OsStr` / `OsString` | Casting to `str` without checking |
| Fallible allocation | `try_reserve` on `Vec`/`String` | Blind `push` in untrusted size scenarios |
| Sorting floats | `sort_by(\|a,b\| a.partial_cmp(b).unwrap())` | `sort` (requires `Ord`) |
| Keyed map, no order | `HashMap` | `BTreeMap` when range/ordered iteration matters |
| Set | `HashSet` / `BTreeSet` | Same ordering tradeoff as maps |
| Fallible main | `fn main() -> Result<(), E>` with `E: Debug` | Panic in `main` for recoverable I/O errors |

## 17.4 Cookbook + Rustlings + Comprehensive Rust — reinforcement map

| Concept | Where it appears first (pedagogy) | Where it becomes “production habit” |
|---------|-----------------------------------|-------------------------------------|
| Ownership / borrow | Rustlings `move_semantics`; Comprehensive **Day 3 PM** | Cookbook FS/network (borrows over `Read`/`Write`) |
| Traits / generics | Rustlings `traits`/`generics`; Comprehensive **Day 2** | Cookbook Serde/CSV/HTTP (bounds everywhere) |
| Errors | Rustlings `error_handling`; Comprehensive **Day 4 PM** | Cookbook About: `anyhow` in examples |
| Iterators | Rustlings `iterators`; Comprehensive **Day 4 AM** | Cookbook parallel/chains (`rayon`, `Iterator`) |
| Concurrency | Rustlings `threads` | Cookbook `rayon`/`crossbeam`; Comprehensive **Concurrency track** |
| Idioms / lint | Rustlings `clippy` | Everywhere: `cargo clippy -D warnings` |

Use **Rustlings** for compiler-error literacy; **Comprehensive Rust** for schedule-aligned theory blocks; **Cookbook** for crate-level “what do I import?” answers.

## 17.5 Async vs sync vs parallel — one decision

```
CPU-bound parallel over collections?     → rayon (sync) or chunked work + std::thread
I/O-bound many connections / sleeps?     → tokio (async)
One background thread from sync code?    → std::thread::spawn or scoped
Need borrow from parent stack in thread? → thread::scope (Fundamentals teaches after sync basics; Cookbook uses for pipelines)
```

## 17.6 `serde` feature flags (Cookbook recipes constantly)

- **`serde`**: always `features = ["derive"]` for `Serialize`/`Deserialize`.
- **`serde_json`**: `json!`, `from_str`, `to_string`; `Value` for unstructured.
- **CSV/TOML**: same derives; invalid rows → `Result` per record or `#[serde(default)]`.
- **Deny unknown fields** for strict configs: `#[serde(deny_unknown_fields)]` on struct.

## 17.7 Security-adjacent defaults (Cookbook crypto chapter alignment)

- Passwords: prefer **argon2** / **password-hash** ecosystem over bare PBKDF2 in new code; Cookbook still demonstrates PBKDF2 via **ring** — treat as “how API works”, not prescriptive algorithm choice for 2026 greenfield.
- TLS: **rustls** over OpenSSL when pure-Rust is acceptable; **reqwest** with `rustls-tls`, `default-features = false` to avoid pulling OpenSSL accidentally.
- Random: **`rand`** + **`OsRng`** for crypto; `thread_rng()` fine for simulations/non-crypto.

## 17.8 PDF / offline Comprehensive Rust

Course ships as **[comprehensive-rust.pdf](https://google.github.io/comprehensive-rust/comprehensive-rust.pdf)** — useful for full-text search when the HTML TOC is not at hand.

---

# 18. Cargo, workspace, and daily workflow (useful addendum)

Practical bits that Cookbook recipes rarely spell out but every non-trivial crate needs. Optimized for “what flag / what `Cargo.toml` line?” lookups.

## 18.1 Workspace layout (minimal mental model)

```text
repo/
  Cargo.toml          # [workspace] members = ["crates/foo", "crates/bar"]
  crates/foo/Cargo.toml
  crates/foo/src/lib.rs
  crates/bar/Cargo.toml   # depends: foo = { path = "../foo" }
```

- **One `Cargo.lock` at workspace root** — commit for binaries; libraries often still commit for reproducible CI.
- **Package vs crate:** workspace has many **packages**; each can expose `lib` + multiple `bin` targets.
- **Renaming deps:** `serde_json = { package = "serde_json", version = "1" }` rarely needed; use for version pin aliasing only.

## 18.2 Feature flags — patterns

| Pattern | Manifest sketch | When |
|---------|-----------------|------|
| Optional heavy dep | `[dependencies]\nfoo = { version = "1", optional = true }\n[features]\ndefault = []\nuse-foo = ["dep:foo"]` | TLS backend, async runtime |
| Pass-through | `reqwest = { optional = true, ... }` in lib; re-export feature | Library wrapping HTTP |
| **`dep:` syntax** (Cargo 1.60+) | `feature-x = ["dep:bar"]` | Cleaner than old `bar` feature name collision |

**Rule:** `cargo tree -e features` shows which features pulled which deps — first debug step for “why is openssl here?”.

## 18.3 `cargo` commands — what to run when

| Goal | Command |
|------|---------|
| Fast typecheck (no codegen link) | `cargo check` |
| Release perf | `cargo build --release` |
| Tests + doc tests | `cargo test` |
| Only lib tests, one thread (debug) | `cargo test --lib -- --test-threads=1` |
| Expand macros | `cargo expand` (needs `cargo install cargo-expand`) |
| MSRV check | `cargo msrv verify` or `cargo +1.xx check` |
| Duplicate deps | `cargo tree -d` |

## 18.4 Environment variables LLMs should know

| Variable | Effect |
|----------|--------|
| `RUST_LOG` | `tracing` / `env_logger`: e.g. `info`, `my_crate=debug` |
| `RUST_BACKTRACE=1` | Full backtrace on panic (`1` short, `full` verbose) |
| `RUSTFLAGS="-C target-cpu=native"` | CPU-specific SIMD (release tuning) |
| `CARGO_INCREMENTAL=0` | Disable incremental (sometimes for CI reproducibility) |
| `SQLX_OFFLINE=true` | `sqlx` compile without DB (uses `.sqlx` cache) |

## 18.5 `rust-version` (MSRV) in `Cargo.toml`

```toml
[package]
rust-version = "1.75"
```

- Signals **minimum** supported compiler; does not bump automatically. Use with **`cargo-msrv`** or CI matrix on oldest supported stable.
- **Edition** (`edition = "2021"`) is separate — set at crate root for `let else`, etc.

## 18.6 Build scripts (`build.rs`) — when Cookbook uses them

- **Link native code:** `cc` crate compiling C/C++ (Cookbook dev-tools chapter).
- **`rerun-if-changed=path`** — tell Cargo when to rerun `build.rs` (avoid stale builds).
- **Generated code:** `OUT_DIR` + `include!(concat!(env!("OUT_DIR"), "/generated.rs"))`.

## 18.7 Lints at crate root (complements Rustlings `clippy` exercises)

```rust
#![warn(missing_docs)]           // libraries: public API docs
#![warn(rust_2018_idioms)]
#![warn(clippy::all)]
// Optional stricter:
// #![warn(clippy::pedantic)]
```

- **`cargo clippy --all-targets --all-features`** — CI should match what users build.

## 18.8 `cargo doc` for onboarding others

| Flag | Use |
|------|-----|
| `cargo doc --no-deps --open` | Fast, only this crate |
| `cargo doc --document-private-items` | Internal API review |
| **Docs.rs** | Published crate docs — check **feature badges** before copying examples from README |

## 18.9 Supply chain (light touch)

- **`cargo audit`** — RUSTSEC advisories (run in CI).
- **`cargo deny check`** — licenses + bans + advisories in one policy file (`deny.toml`).
- **`cargo-semver-checks`** — catch semver breaks before publish (especially libs).

---

*End of Cluster 08 notes.*
