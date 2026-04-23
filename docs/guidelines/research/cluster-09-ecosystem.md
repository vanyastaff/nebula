---

cluster: 09-ecosystem
primary_tag: 11-ecosystem-crate-picks
secondary_tags: [10-testing-and-tooling, 12-modern-rust, 01-meta-principles]
rust_edition_min: "2021"
rust_version_min: "1.95"
audience: coding-llms
last_reviewed: 2026-04-21
provenance: >
  Primary synthesis from live-fetched April 2026 indexes: blessed.rs/crates
  (full recommended directory), rust-unofficial/awesome-rust README (raw),
  areweasyncyet.rs, arewewebyet.org, arewegameyet.rs (landing + ecosystem
  categories), this-week-in-rust.org issues 643–647 (Crate of the Week +
  compiler/library highlights), lborb.github.io/book (meta book index).
  lib.rs landing page fetched successfully (index scale + about link); `/categories`
  returned 404 — use site search / tag pages instead. blessed.rs cross-links
  lib.rs for many crates. arewelearningyet.com often times out; ML picks use
  blessed.rs numerics + awesome-rust “Machine learning” + Appendix F. Re-verify
  MSRV at Cargo edit time.
sources_fetched_2026_04:

- url: [https://raw.githubusercontent.com/rust-unofficial/awesome-rust/master/README.md](https://raw.githubusercontent.com/rust-unofficial/awesome-rust/master/README.md)
status: ok
- url: [https://blessed.rs/crates](https://blessed.rs/crates)
status: ok
- url: [https://lib.rs/](https://lib.rs/)
status: ok
- url: [https://lib.rs/categories](https://lib.rs/categories)
status: "404"
- url: [https://this-week-in-rust.org/](https://this-week-in-rust.org/)
status: ok
- url: [https://this-week-in-rust.org/blog/2026/04/15/this-week-in-rust-647/](https://this-week-in-rust.org/blog/2026/04/15/this-week-in-rust-647/)
status: ok
- url: [https://this-week-in-rust.org/blog/2026/04/08/this-week-in-rust-646/](https://this-week-in-rust.org/blog/2026/04/08/this-week-in-rust-646/)
status: ok
- url: [https://this-week-in-rust.org/blog/2026/04/01/this-week-in-rust-645/](https://this-week-in-rust.org/blog/2026/04/01/this-week-in-rust-645/)
status: ok
- url: [https://this-week-in-rust.org/blog/2026/03/25/this-week-in-rust-644/](https://this-week-in-rust.org/blog/2026/03/25/this-week-in-rust-644/)
status: ok
- url: [https://this-week-in-rust.org/blog/2026/03/18/this-week-in-rust-643/](https://this-week-in-rust.org/blog/2026/03/18/this-week-in-rust-643/)
status: ok
- url: [https://areweasyncyet.rs/](https://areweasyncyet.rs/)
status: ok
- url: [https://arewegameyet.rs/](https://arewegameyet.rs/)
status: ok
- url: [https://arewewebyet.org/](https://arewewebyet.org/)
status: ok
- url: [https://arewelearningyet.com/](https://arewelearningyet.com/)
status: timeout
- url: [https://lborb.github.io/book/](https://lborb.github.io/book/)
status: ok

---

# Cluster 09 — Rust Ecosystem Crate Catalog

> **Purpose.** Give a coding LLM (Claude, Cursor, etc.) a single authoritative
> answer to "which crate do I reach for?" when writing Rust 1.95+ code. The
> format is deliberately dense and table-heavy so lookups are cheap.
>
> **How to read the picks.** Each domain section has:
>
> 1. **Canonical** — the one crate to use by default. Pick this if the user
>   has no opinion.
> 2. **Alternatives** — named crates with the *one* concrete reason each is
>   preferred (lighter, sync-only, no-std, MPL-licensed, etc.).
> 3. **Avoid / legacy** — crates that show up in search results or stale
>   tutorials but should not be chosen for new code. The note says *why*.
> 4. **Typical imports** — copy-pasteable `use` lines and `Cargo.toml`
>   feature flags.
>
> **Status keys.** `[stable]` = 1.0+, widely adopted, API frozen.
> `[mature]` = 0.x but de-facto stable and maintained. `[active]` = under
> active development, breaking changes possible. `[maintenance]` = still
> works, no new features. `[deprecated]` = do not pick for new code.
> `[nightly]` = requires nightly compiler.

---

## Table of contents

1. [Ecosystem index synthesis (fetched)](#00-ecosystem-index-synthesis-fetched)
2. [Meta: how to pick a crate](#01-meta-how-to-pick-a-crate)
3. [Async runtimes](#02-async-runtimes)
4. [HTTP clients](#03-http-clients)
5. [HTTP / web frameworks](#04-http--web-frameworks)
6. [Serialization](#05-serialization)
7. [Error handling](#06-error-handling)
8. [Logging & tracing](#07-logging--tracing)
9. [CLI](#08-cli)
10. [Database](#09-database)
11. [Testing](#10-testing)
12. [Data structures & utilities](#11-data-structures--utilities)
13. [Regex & parsing](#12-regex--parsing)
14. [Date & time](#13-date--time)
15. [UUID](#14-uuid)
16. [Random](#15-random)
17. [Cryptography & TLS](#16-cryptography--tls)
18. [Compression](#17-compression)
19. [Image, media, SVG, PDF](#18-image-media-svg-pdf)
20. [GUI](#19-gui)
21. [Game development](#20-game-development)
22. [Embedded](#21-embedded)
23. [WebAssembly](#22-webassembly)
24. [Derive helpers & macros](#23-derive-helpers--macros)
25. [Numerics, science, ML](#24-numerics-science-ml)
26. [Configuration](#25-configuration)
27. [Build tools & task runners](#26-build-tools--task-runners)
28. [Filesystem & IO](#27-filesystem--io)
29. [Concurrency primitives](#28-concurrency-primitives)
30. [Process & subprocess](#29-process--subprocess)
31. [Text, strings, Unicode](#30-text-strings-unicode)
32. [Encoding (base64, hex, etc.)](#31-encoding-base64-hex-etc)
33. [Templating](#32-templating)
34. [Email](#33-email)
35. [FFI / interop](#34-ffi--interop)
36. [Observability: metrics](#35-observability-metrics)
37. [Modern Rust feature status](#36-modern-rust-feature-status-are-we-x-yet)
38. [Testing & cargo tooling](#37-testing--cargo-tooling)
39. [Canonical reference books](#38-canonical-reference-books)
40. [Quick decision matrix](#39-quick-decision-matrix)
41. [Appendix F — Supplementary decision tables](#appendix-f--supplementary-decision-tables-extra-push)

---

## 00 Ecosystem index synthesis (fetched)

Tag: `11-ecosystem-crate-picks` + `01-meta-principles`. April 2026 snapshot tying
**blessed.rs** (hand-curated “recommended crate directory”), **awesome-rust**
(broad category lists), **arewewebyet** / **arewegameyet** / **areweasyncyet**
(status pages), and **This Week in Rust** (community spotlight). Use this
section when you need “what do the curators agree on?” before drilling into
domain tables below.

### Curator alignment matrix (canonical picks)


| Domain                  | blessed.rs canonical                                                               | awesome-rust (presence)                  | arewewebyet / other                                                                          |
| ----------------------- | ---------------------------------------------------------------------------------- | ---------------------------------------- | -------------------------------------------------------------------------------------------- |
| Async runtime           | `tokio` (new projects); `smol` modular; `futures-executor` for `block_on`          | Large “Asynchronous” library list        | areweasyncyet: async/await **yes** (1.39+); ecosystem around tokio/mio/async-std             |
| HTTP client             | `reqwest` (async); `ureq` (sync minimal)                                           | “Network programming”, “Web programming” | AWWY: mature client ecosystem                                                                |
| HTTP server / framework | `axum` (most new); `actix-web` (max perf); notes `rocket`, `poem`, `warp`, `tide`  | Web frameworks subsection                | AWWY: **Actix Web** + **Axum** as production-ready; **Warp**, **Tide** as named alternatives |
| Serialization           | `serde` + format crates; `prost`/`tonic` gRPC; `postcard` no_std; `rkyv` zero-copy | “Encoding”, “Data processing”            | —                                                                                            |
| Errors                  | `anyhow` apps; `thiserror` libs; `color-eyre` user-facing                          | —                                        | —                                                                                            |
| Logging                 | `tracing` (go-to); `log` simple non-async; `slog` structured                       | “Logging”                                | —                                                                                            |
| CLI args                | `clap` full-featured; `bpaf`, `lexopt`, `pico-args` minimal                        | “Command-line”                           | —                                                                                            |
| SQL                     | `sqlx`; `diesel` (+ `diesel-async`); `sea-orm` on sqlx; `rusqlite` sync SQLite     | “Database” libs                          | AWWY: Diesel, sqlx, native drivers                                                           |
| TLS                     | `rustls` modern; `native-tls` system SSL                                           | “Cryptography”                           | blessed: prefer rustls posture                                                               |
| Crypto primitives       | `ring`, `webpki`, `subtle`, `zeroize`; RustCrypto org crates                       | —                                        | —                                                                                            |
| GUI                     | `egui`, `iced`, `tauri`, `slint`, `dioxus-desktop`, `gtk4`+`relm4`, etc.           | “GUI”                                    | —                                                                                            |
| Games                   | `bevy`, `fyrox`, `ggez`, `macroquad`, `glam`                                       | “Game development”                       | arewegameyet: **many** category pages (ECS, engines, audio, physics, …)                      |
| Numerics / DF           | `nalgebra`, `ndarray`, `polars`, `datafusion`                                      | “Computation”, ML sections               | [§00 ML fallback](#ml-ecosystem-fallback-arewelearningyet) + [§24](#24-numerics-science-ml)  |
| gRPC                    | `tonic`                                                                            | —                                        | blessed networking section                                                                   |


**Interpretation.** When **blessed.rs** and **arewewebyet** both name the same
crate (e.g. Axum, Actix, Diesel, sqlx), treat it as **strong consensus**.
**awesome-rust** is broader (more crates, less ordering) — use for discovery,
not default choice.

### This Week in Rust — Crate of the Week (issues 643–647)


| Issue                                                                       | Date       | Crate of the Week                                  | Role (one line)                               |
| --------------------------------------------------------------------------- | ---------- | -------------------------------------------------- | --------------------------------------------- |
| [643](https://this-week-in-rust.org/blog/2026/03/18/this-week-in-rust-643/) | 2026-03-18 | [grab](https://github.com/anwitars/grab)           | CLI: CSV → JSON conversion                    |
| [644](https://this-week-in-rust.org/blog/2026/03/25/this-week-in-rust-644/) | 2026-03-25 | [noq](https://github.com/n0-computer/noq)          | QUIC transport protocol (pure Rust)           |
| [645](https://this-week-in-rust.org/blog/2026/04/01/this-week-in-rust-645/) | 2026-04-01 | [tsastat](https://github.com/AnkurRathore/tsastat) | Thread State Analysis (TSA) for Linux         |
| [646](https://this-week-in-rust.org/blog/2026/04/08/this-week-in-rust-646/) | 2026-04-08 | [aimdb-core](https://crates.io/crates/aimdb-core)  | Type-safe data pipeline; Rust types as schema |
| [647](https://this-week-in-rust.org/blog/2026/04/15/this-week-in-rust-647/) | 2026-04-15 | [Myth Engine](https://github.com/panxinmiao/myth)  | Cross-platform rendering / render-graph       |


**TWiR tooling signal (646):** JetBrains **RustRover 2026.1** advertises native
**cargo-nextest** integration — reinforces `nextest` as mainstream test runner.

### TWiR — Rust project highlights relevant to modern idioms (647 sample)

Tag: `12-modern-rust`. From merged PR summaries in issue 647 (illustrative; not
exhaustive):


| Theme          | Example merged work                                                                                            | LLM takeaway                                                    |
| -------------- | -------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `const` in std | `const Default` for `LazyCell`/`LazyLock`; `constify` iterators / `DoubleEndedIterator` / `Step` for `NonZero` | Prefer `LazyLock`/`OnceLock` in const contexts where applicable |
| Diagnostics    | `#[diagnostic::on_const]`, `#[diagnostic::on_unknown]`, `#[diagnostic::on_move]` on `Rc`/`Arc`                 | Error types can integrate with rustc’s diagnostic attributes    |
| Ranges API     | Stabilizations around `Range` types / iterators                                                                | Watch `std::ops::Range*` churn when targeting latest stable     |


Official blog cross-links in the same period: **docs.rs** building fewer targets
by default (646); **WebAssembly targets** and undefined symbols (646); **Rust
1.94.1** (645). Align crate features with default docs.rs target set when
publishing.

### Are We Async Yet? — executive summary

Tag: `12-modern-rust`. Site confirms: **async/await stable since Rust 1.39**.
Ecosystem centers on **futures**, **mio**, **tokio**, **async-std**. The page is
now largely a **tracker of open compiler issues** (async diagnostics, drop
tracking, RPITIT edge cases) — still useful for “why did this async fn fail to
compile?” deep dives, not for picking crates.

### Are We Web Yet? — executive summary

**Headline:** “Yes, and it's freaking fast.” Names **Actix Web** and **Axum** as
mature/production frameworks; **Warp** and **Tide** as “innovative.” **Diesel**
(ORM), **sqlx** (async SQL), MongoDB/SQLite/Postgres/MySQL drivers cited.
Explicit caveat: **no Django/Rails-sized monolith** — expect Flask/Sinatra-style
composition. **WASM:** points to **Yew**, **Seed**, and the Rust WASM book.

### Are We Game Yet? — executive summary

Landing page stresses ecosystem is **young but usable**; **Ecosystem** hub
lists categories (2D/3D rendering, ECS, engines, physics, audio, networking,
UI, etc.) with **per-category crate counts** — use for discovery; canonical
picks for engines remain **Bevy** / **macroquad** / **Fyrox** (see §20).

### Awesome Rust — how to use this repo

The README is a **very large** categorized list (applications, dev tools,
libraries by topic). It does **not** rank crates inside categories. Workflow for
LLMs:

1. Find category (e.g. “Parsing”, “Cryptography”).
2. Copy candidate names **into** blessed.rs or crates.io for validation.
3. Prefer blessed.rs when both list the same crate.

### The Little Book of Rust Books (lborb) — meta pointers

Tag: `01-meta-principles`. The site aggregates:

- **Official books** (rust-lang.org)
- **Unofficial books** (community)
- **Application books** (domain-specific)

Also points to **mdBook** and non-mdBook titles via **Rust Books** list. Use for
“what to read after The Book,” not for crate selection.

### lib.rs — curated index (fetched)

Tag: `01-meta-principles`. [Lib.rs](https://lib.rs/) positions itself as a
**lightweight, opinionated, curated, unofficial alternative to crates.io**
(see [about](https://lib.rs/about)). Landing page (2026-04): **~257k** indexed
Rust libraries and applications — same order of magnitude as the full registry,
but with editorial signals (badges, categories, maintainer notes) crates.io does
not surface as prominently.

**LLM workflow**


| Goal                        | crates.io        | lib.rs                                     |
| --------------------------- | ---------------- | ------------------------------------------ |
| Exact semver / owners       | Primary          | Mirror; verify on crates.io before pinning |
| “What is everyone using?”   | Download counts  | Front-page / tag / category discovery      |
| “Is this crate maintained?” | GitHub link only | Often richer context on crate page         |
| Search                      | `cargo search`   | Web UI + filters                           |


**Note:** `https://lib.rs/categories` returned **404** in-session — navigate from
the home page or use `/tags/` / crate search instead of hard-coding `/categories`.

### ML ecosystem fallback (arewelearningyet)

Tag: `11-ecosystem-crate-picks`. [arewelearningyet.com](https://arewelearningyet.com/)
is the traditional “state of ML in Rust” hub; it **timed out** during automated
fetch. Until live content is available, use this **consensus-aligned** matrix
(overlaps blessed.rs “Math / Scientific” + awesome-rust ML lists):


| Need                       | Canonical starting points | Notes                                                  |
| -------------------------- | ------------------------- | ------------------------------------------------------ |
| DataFrames / analytics     | `polars`                  | Arrow-native; Python pandas-like ergonomy in Rust      |
| In-process SQL analytics   | `datafusion`              | Apache Arrow; heavy OLAP                               |
| Classical ML (CPU)         | `linfa`                   | scikit-learn-shaped API on `ndarray`                   |
| DL training / research     | `burn`                    | Pure Rust graph, multi-backend                         |
| DL inference (HF-friendly) | `candle`                  | Smaller surface than PyTorch; common in Rust LLM tools |
| Bindings to libtorch       | `tch`                     | Maximum model zoo compatibility; C++ dep               |
| ONNX Runtime               | `ort`                     | Deploy exported `.onnx` models                         |
| Numerical arrays           | `ndarray`                 | N-dim; ecosystem hub                                   |
| Linear algebra             | `nalgebra`                | Vectors/matrices; sim/robotics                         |
| Game / graphics math       | `glam`                    | SIMD-friendly; use with Bevy/render stacks             |
| Tokenizers (HF parity)     | `tokenizers`              | Bindings to Hugging Face tokenizer crate               |


**Rule:** prefer `**candle` or `burn`** for “Rust-native” stacks; reach for
`**tch`** only when you must run a specific `torch` checkpoint without ONNX
export; use `**ort**` for cross-framework ONNX deployment.

---

## 01 Meta: how to pick a crate

Tag: `01-meta-principles`. Curator wisdom distilled from blessed.rs,
awesome-rust, and the lib.rs front page.


| Signal                                           | Good threshold                | Why it matters                    |
| ------------------------------------------------ | ----------------------------- | --------------------------------- |
| Downloads on crates.io                           | > 1 M total or > 100 k recent | Proxy for real-world testing.     |
| Recent commit                                    | Within ~6 months              | Dead crates silently break.       |
| Issues: open vs closed                           | > 2:1 closed                  | Active maintenance.               |
| Listed on blessed.rs                             | Present                       | Hand-curated; very high bar.      |
| Listed as "Crate of the Week" on TWiR            | Yes                           | Community-blessed.                |
| `#![deny(unsafe_code)]` or documented unsafe     | Present                       | Audit-friendly.                   |
| MSRV declared in `Cargo.toml` via `rust-version` | Present                       | Won't silently break CI.          |
| Semver discipline                                | Uses `cargo-semver-checks`    | Stable upgrades.                  |
| 1.0 release                                      | Ideal but not required        | Many 0.x crates are de-facto 1.0. |


**Rule of thumb (blessed.rs curator).** Prefer the crate that appears in
`Cargo.lock` of `rustc`, `cargo`, `tokio`, `serde`, `ripgrep`, or
`rust-analyzer`. Those are the six biggest "taste canaries" of the ecosystem.

**When two crates are equally good:**

- Pick the one with **fewer transitive deps** — build time matters.
- Pick the one with **feature flags** for optional subsystems — downstream
users pay for what they use.
- Pick the one that is `**no_std`-capable** if there's any chance the code
migrates to embedded / WASM.
- Pick the one that **doesn't pull tokio** if you're in a library (let the
binary choose the runtime).

**Red flags:**

- Crate name squatting (`rocket-contrib` after `rocket` 0.5+, `actix` core vs
`actix-web`, etc.) — always check the README for the "current" name.
- "Version 0.0.x" with no recent release.
- No `README.md` on crates.io.
- Only example is `hello_world`.

**MSRV policy cheat-sheet.**

- `serde`, `tokio`, `reqwest`, `axum` — MSRV roughly "latest stable minus 6".
- `clap` 4.x — MSRV floats, often pinned to stable-minus-3.
- Pin MSRV in your own crate with `rust-version = "1.85"` (or whatever you
actually test). `cargo build` will refuse to compile with older toolchains
instead of giving cryptic "unstable feature" errors.

---

## 02 Async runtimes

Tag: `11-ecosystem-crate-picks`. The most contested domain in Rust — get it
right first because every other async crate transitively depends on the
runtime choice.

### Picks


| Crate                      | Role                                                                                  | Status                                                                             | When to pick                                                                              |
| -------------------------- | ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `**tokio`**                | Multi-threaded work-stealing runtime + timers, channels, fs, net, sync                | **[stable]** — the de-facto standard, >95% of async crates target it               | **Canonical for servers, CLIs, anything networked.**                                      |
| `smol`                     | Lightweight single-binary runtime, built on `async-executor` + `async-io` + `polling` | [mature]                                                                           | Small binaries, embedded-ish, or when you want to read the runtime source in one sitting. |
| `async-std`                | `std`-shaped async runtime                                                            | **[maintenance]** — last major release 2023; migrate to tokio or smol for new code | Legacy codebases only.                                                                    |
| `embassy`                  | `no_std` async runtime for embedded (Cortex-M, RISC-V, ESP, nRF, STM32)               | **[mature]** — see cluster on embedded                                             | **Canonical for embedded async.**                                                         |
| `glommio`                  | Thread-per-core, io_uring-native                                                      | [active]                                                                           | Linux-only, ultra-low-latency, single-purpose services.                                   |
| `monoio`                   | Thread-per-core, io_uring                                                             | [active]                                                                           | Same niche as glommio; ByteDance-backed.                                                  |
| `compio`                   | Thread-per-core on IOCP/io_uring/kqueue                                               | [active]                                                                           | Cross-platform thread-per-core.                                                           |
| `tokio-uring`              | `tokio` + io_uring completion-based I/O                                               | [active]                                                                           | Keep tokio ergonomics, gain io_uring throughput on Linux.                                 |
| `futures` / `futures-lite` | Runtime-agnostic combinators (`join!`, `select!`, `Stream`, `Sink`, utils)            | [stable]                                                                           | Use in **libraries** — stay runtime-neutral. `futures-lite` if you want no macros.        |


### Canonical for a new project

- **Application (server, CLI, tool):** `tokio = { version = "1", features = ["full"] }`.
- **Embedded / no_std:** `embassy`.
- **Library:** Depend on `futures` trait machinery only; let the caller
choose. Exception: if the library literally only makes sense inside tokio
(e.g., wraps a tokio primitive), take the dep.

### Typical imports

```toml
# Cargo.toml — application
[dependencies]
tokio = { version = "1", features = ["full"] }
# or granular:
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "io-util", "sync", "signal", "fs"] }
futures = "0.3"        # for StreamExt, SinkExt, join!, etc.
async-trait = "0.1"    # only if you target MSRV < 1.75 (AFIT stabilized 1.75)

# Library (runtime-neutral)
[dependencies]
futures-core = "0.3"
futures-util = { version = "0.3", default-features = false }
pin-project-lite = "0.2"
```

### Runtime selection heuristics

- **"I'm writing an HTTP server."** → tokio. Every mainstream framework is
tokio-native.
- **"I want async but also single binary under 1 MB."** → smol.
- **"I'm in `no_std`."** → embassy.
- **"I want max QPS on Linux."** → glommio or tokio-uring.
- **"I have sync code and want to sprinkle async."** → don't. Add a single
`#[tokio::main]` at the top or use `pollster::block_on` for a one-off.

### Are We Async Yet? — status cheat-sheet (Rust 1.95 era)

Tag: `12-modern-rust`.


| Feature                                         | Status as of Rust 1.95                                             | Notes                                                      |
| ----------------------------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------- |
| `async fn` in traits (AFIT)                     | **Stable since 1.75**                                              | Static dispatch only; for dyn use `#[async_trait]`.        |
| Return-position `impl Trait` in traits (RPITIT) | Stable 1.75                                                        | Enables `-> impl Future` signatures in traits.             |
| `async fn` in `dyn` trait objects               | Workaround via `async-trait` crate or `dynosaur` / `trait-variant` | Native dyn AFIT not yet stable.                            |
| `trait-variant` crate                           | Generates `Send`-bounded and non-`Send` variants of async traits   | Community standard for libraries.                          |
| `async closures`                                | **Stable 1.85** (`async                                            |                                                            |
| `async Drop`                                    | Not stable                                                         | Use explicit `.shutdown().await` methods.                  |
| `async fn` in `Fn` trait                        | Partial via `AsyncFnOnce`/`AsyncFnMut`/`AsyncFn` stable 1.85       |                                                            |
| Generators / coroutines                         | Unstable (`gen {}`)                                                | Use `async-stream` or `futures::stream::unfold` meanwhile. |
| `impl Trait` in type aliases (TAIT)             | Stable 1.75 (for RPITIT); broader TAIT still limited               |                                                            |
| `try_join!` / `join!` / `select!`               | Via `tokio::` or `futures::`                                       | Pick the macro from the runtime you're using.              |


---

## 03 HTTP clients

Tag: `11-ecosystem-crate-picks`.


| Crate                          | Role                                                                                       | Status        | Pick when                                                         |
| ------------------------------ | ------------------------------------------------------------------------------------------ | ------------- | ----------------------------------------------------------------- |
| `**reqwest**`                  | High-level async HTTP client on top of hyper; JSON, cookies, redirects, multipart, proxies | **[stable]**  | **Canonical.** Any async HTTP client need.                        |
| `hyper` (client)               | Low-level async HTTP/1, HTTP/2, HTTP/3 client                                              | [stable]      | You're building a framework or reqwest lacks a primitive. Rarely. |
| `ureq`                         | Blocking, sync-only HTTP client, minimal deps                                              | [mature]      | CLIs that shouldn't pull tokio; install scripts; `ureq = "2"`.    |
| `isahc`                        | Async HTTP via libcurl                                                                     | [maintenance] | Only if you need libcurl's exact behavior.                        |
| `surf`                         | Runtime-agnostic async client                                                              | [maintenance] | Legacy async-std codebases.                                       |
| `attohttpc`                    | Sync client smaller than ureq                                                              | [mature]      | Very small binaries.                                              |
| `xh` / `hurl`                  | Not libs — CLI wrappers around reqwest for httpie-like UX                                  | —             | —                                                                 |
| `http`                         | Types (`Method`, `Uri`, `StatusCode`, `HeaderMap`) shared across client/server crates      | [stable]      | Re-export; every HTTP crate depends on it.                        |
| `http-body` / `http-body-util` | `Body` trait + helpers                                                                     | [stable]      | Library interop.                                                  |


`**reqwest` features you'll want:**

```toml
reqwest = { version = "0.12", default-features = false, features = [
  "rustls-tls",       # do NOT use the default native-tls / openssl
  "http2",
  "gzip", "brotli", "deflate", "zstd",
  "json",
  "cookies",
  "multipart",
  "stream",
  "charset",
] }
```

**Do not** use `reqwest`'s default `native-tls` feature in containers —
prefer `rustls-tls`. `native-tls` silently pulls OpenSSL.

---

## 04 HTTP / web frameworks

Tag: `11-ecosystem-crate-picks`.

### Picks


| Framework    | Role                                                                     | Status                                                | Pick when                                                            |
| ------------ | ------------------------------------------------------------------------ | ----------------------------------------------------- | -------------------------------------------------------------------- |
| `**axum`**   | Tokio/hyper-native framework, tower middleware, extractor-based handlers | **[stable]** (1.0 released, maintained by tokio team) | **Canonical for 2026.** Best fit with tokio/tower/tonic ecosystems.  |
| `actix-web`  | Actor-flavored framework, historical leader in benchmarks                | [stable]                                              | You want actor model, or existing Actix code.                        |
| `rocket`     | Macro-heavy, very ergonomic, async on 0.5+                               | [mature]                                              | Prototyping, student work, ergonomics-over-perf.                     |
| `warp`       | Filter-combinator model                                                  | [maintenance]                                         | Legacy; prefer axum for new code (axum is from same maintainers).    |
| `poem`       | Tokio-native, middleware-focused, OpenAPI-first                          | [mature]                                              | Strong OpenAPI/GraphQL story out of the box.                         |
| `salvo`      | Tokio-native, batteries-included                                         | [active]                                              | You want WebSocket + static serving + ACME in one.                   |
| `loco`       | Rails-inspired, on axum                                                  | [active]                                              | You want scaffolding (models, migrations, mailers).                  |
| `ntex`       | Fork of actix, raw performance                                           | [active]                                              | You are sure perf matters more than ecosystem.                       |
| `pavex`      | Compile-time DI, no macros                                               | [active/pre-1.0]                                      | You want reflection-free type safety; willing to adopt new paradigm. |
| `tonic`      | gRPC on hyper/tower                                                      | [stable]                                              | Canonical gRPC.                                                      |
| `tower`      | Service abstraction — middleware layer shared by axum/tonic/hyper        | [stable]                                              | Always. Middleware layer.                                            |
| `tower-http` | Ready-made HTTP middleware (CORS, trace, compress, auth, timeout, etc.)  | [stable]                                              | Always with axum.                                                    |


### Canonical stack

```toml
# Cargo.toml — production HTTP server
[dependencies]
tokio   = { version = "1", features = ["full"] }
axum    = { version = "0.8", features = ["macros", "multipart", "ws", "tracing"] }
tower   = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "compression-full", "timeout", "limit", "request-id", "catch-panic"] }
serde   = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
thiserror = "2"
anyhow  = "1"          # only in main.rs / bin-adjacent code
```

### Typical imports

```rust
use axum::{
    Router, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::{
    cors::CorsLayer,
    trace::TraceLayer,
    compression::CompressionLayer,
    timeout::TimeoutLayer,
};
```

### Modern idiom (Rust 1.95)

- Handlers return `Result<T, E>` where `E: IntoResponse`.
- State via `State<Arc<AppState>>`; don't hand-thread state.
- Prefer `axum::extract::Json` input + `axum::Json` output.
- Use `#[axum::debug_handler]` while iterating — it unlocks real error messages.
- For SSE: `axum::response::sse::{Sse, Event}`.
- For WebSockets: `axum::extract::ws::{WebSocket, WebSocketUpgrade}`.

---

## 05 Serialization

Tag: `11-ecosystem-crate-picks`.


| Crate                 | Format                                        | Status                                                      | Pick when                                   |
| --------------------- | --------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------- |
| `**serde**`           | Framework; derive + trait                     | **[stable]**                                                | **Always.**                                 |
| `**serde_json`**      | JSON                                          | **[stable]**                                                | Canonical JSON.                             |
| `serde_yaml`          | YAML 1.2                                      | **[maintenance]** — unmaintained by author, use `serde_yml` | Legacy.                                     |
| `serde_yml`           | Active fork of `serde_yaml`                   | [active]                                                    | New YAML code.                              |
| `serde-yaml-ng`       | Another fork                                  | [active]                                                    | Alternative to `serde_yml`.                 |
| `toml`                | TOML                                          | [stable]                                                    | Config files, `Cargo.toml` manipulation.    |
| `toml_edit`           | TOML with formatting/comments preserved       | [stable]                                                    | Editing existing TOML (e.g., `cargo-edit`). |
| `ron`                 | Rusty Object Notation                         | [mature]                                                    | Game configs, Bevy scenes.                  |
| `bincode` 2.x         | Compact binary, no schema                     | [stable]                                                    | **Canonical binary for Rust-to-Rust.**      |
| `postcard`            | no_std binary, COBS framing                   | [stable]                                                    | **Canonical for embedded / wire.**          |
| `rmp-serde`           | MessagePack                                   | [stable]                                                    | Polyglot wire format.                       |
| `ciborium`            | CBOR                                          | [stable]                                                    | IETF standards, COSE, WebAuthn.             |
| `rkyv`                | Zero-copy, archived layout                    | [mature]                                                    | mmap'd data, game saves, max read speed.    |
| `bitcode`             | Compact, field-packing                        | [active]                                                    | Smaller than bincode on many workloads.     |
| `serde_qs`            | Query strings (nested)                        | [stable]                                                    | Web form/query parsing.                     |
| `serde_urlencoded`    | x-www-form-urlencoded flat                    | [stable]                                                    | HTML forms.                                 |
| `quick-xml` + `serde` | XML                                           | [stable]                                                    | Canonical XML.                              |
| `csv`                 | CSV with serde                                | [stable]                                                    | Canonical CSV.                              |
| `prost`               | Protocol Buffers                              | [stable]                                                    | Canonical protobuf.                         |
| `capnp`               | Cap'n Proto                                   | [mature]                                                    | Cap'n Proto.                                |
| `flatbuffers`         | FlatBuffers                                   | [mature]                                                    | FlatBuffers.                                |
| `borsh`               | Deterministic binary (crypto/Solana heritage) | [stable]                                                    | Deterministic hash inputs.                  |


### Canonical picks

- Text config → **TOML** (`toml`) or YAML (`serde_yml`).
- Web JSON → `**serde_json`**.
- Rust-to-Rust wire → `**bincode` 2.x** (or `postcard` if no_std).
- Embedded/UART → `**postcard`**.
- mmap / ultra-fast read → `**rkyv`**.
- Cross-language schema'd wire → `**prost**` (protobuf).

### Typical imports

```toml
[dependencies]
serde = { version = "1", features = ["derive", "rc"] }
serde_json = { version = "1", features = ["preserve_order"] }
serde_with = { version = "3", features = ["chrono_0_4"] }  # very useful
humantime-serde = "1"
```

```rust
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct User { id: u64, full_name: String }
```

### `serde_with` highlights (always worth the dep)

- `#[serde_as(as = "DisplayFromStr")]`
- `#[serde_as(as = "DurationSeconds<u64>")]`
- `#[serde_as(as = "Base64")]`
- `#[serde(with = "humantime_serde")]` on `Duration`.

---

## 06 Error handling

Tag: `11-ecosystem-crate-picks`.

### Picks


| Crate               | Role                                                     | Status       | Pick when                                         |
| ------------------- | -------------------------------------------------------- | ------------ | ------------------------------------------------- |
| `**thiserror**` 2.x | Derive `Error` with variant-level messages and `#[from]` | **[stable]** | **Canonical for library errors.**                 |
| `**anyhow`**        | One-size-fits-all `anyhow::Error`, `?` with `Context`    | **[stable]** | **Canonical for binary / `main.rs`.**             |
| `eyre`              | `anyhow`-shaped but with pluggable reporters             | [mature]     | You want a custom report hook.                    |
| `color-eyre`        | `eyre` + colored/`span_trace` reports                    | [mature]     | Pretty CLIs.                                      |
| `miette`            | Error + source annotations (like rustc's carets)         | [mature]     | Compilers, linters, user-facing tools.            |
| `snafu`             | Context selectors, less magic than anyhow                | [mature]     | You want structured context, not one-off strings. |
| `error-stack`       | Error report with attached context types                 | [mature]     | Typed context chain.                              |


### Canonical rule

- **Library crate:** `thiserror` per-module error enums; expose
`MyCrateError` at crate root. Implement `From<OtherError>` via `#[from]`.
Never pull `anyhow` into a library's public API.
- **Binary crate / `main.rs`:** `anyhow::Result<T>`, `.context("...")`,
`?`-everywhere. Optionally `color-eyre::install()` at startup.
- **User-facing tool (linter, compiler):** add `miette` on top of thiserror
via `#[derive(Diagnostic)]`.

### Typical imports

```toml
thiserror = "2"
anyhow    = "1"
miette    = { version = "7", features = ["fancy"] }
```

```rust
// lib — thiserror
use thiserror::Error;
#[derive(Debug, Error)]
pub enum DbError {
    #[error("connection failed: {0}")]
    Connection(#[from] sqlx::Error),
    #[error("not found: {id}")]
    NotFound { id: u64 },
}

// bin — anyhow
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let cfg = std::fs::read_to_string("app.toml")
        .context("reading app.toml")?;
    Ok(())
}
```

### Anti-patterns

- Using `anyhow::Error` in a library's public API — callers can't match
variants.
- Using `thiserror` in a binary to manually list every conceivable source
error — use `anyhow` at the leaf.
- `Box<dyn Error>` — works but loses downcast ergonomics; `anyhow` is
strictly better.
- Swallowing `?` errors without `.context()`.

---

## 07 Logging & tracing

Tag: `11-ecosystem-crate-picks`.


| Crate                      | Role                                           | Status        | Pick when                                                      |
| -------------------------- | ---------------------------------------------- | ------------- | -------------------------------------------------------------- |
| `**tracing**`              | Structured, span-aware events                  | **[stable]**  | **Canonical.** Libraries and binaries alike.                   |
| `**tracing-subscriber`**   | Collect, filter, format tracing events         | **[stable]**  | Always, in the binary.                                         |
| `tracing-appender`         | Rolling file output, non-blocking writer       | [stable]      | Production file logging.                                       |
| `tracing-bunyan-formatter` | Bunyan JSON                                    | [mature]      | Node-shop pipelines.                                           |
| `tracing-log`              | Bridge `log` crate → `tracing`                 | [stable]      | When a dep still uses `log!()`.                                |
| `tracing-error`            | `ErrorLayer` to attach span traces to errors   | [stable]      | Pair with `color-eyre`.                                        |
| `tracing-opentelemetry`    | OTel bridge                                    | [active]      | OTel vendor integration.                                       |
| `log`                      | Old facade trait                               | [stable]      | Library-facing fallback; most new libs use `tracing` directly. |
| `env_logger`               | Simple `RUST_LOG`-driven line logger for `log` | [stable]      | Scripts, examples. Not for production services.                |
| `pretty_env_logger`        | `env_logger` + colors                          | [mature]      | Local dev only.                                                |
| `slog`                     | Pre-tracing structured logging                 | [maintenance] | Legacy only; migrate to tracing.                               |
| `fern`                     | `log` backend with custom format               | [maintenance] | Legacy.                                                        |
| `simple_logger`            | Minimal `log` backend                          | [mature]      | CLIs that shouldn't pull tracing.                              |


### Canonical stack

```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json", "fmt", "time", "chrono"] }
tracing-appender = "0.2"
tracing-error = "0.2"
```

```rust
use tracing::{info, warn, error, debug, trace, instrument};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_target(true).with_thread_ids(false))
        .init();
}

#[instrument(skip(db), fields(user_id = %req.user_id))]
async fn handle(db: &Db, req: Req) -> Result<Resp, E> { /* ... */ }
```

### Rule

- **Library:** emit `tracing::event!` macros. Do not configure subscribers.
- **Binary:** install `tracing-subscriber` at the top of `main`.
- **Never** mix `log` and `tracing` on purpose — if a dep uses `log`, enable
`tracing-log`'s `LogTracer::init()` to bridge it.

---

## 08 CLI

Tag: `11-ecosystem-crate-picks`.

### Argument parsers


| Crate                             | Role                                         | Status        | Pick when                                        |
| --------------------------------- | -------------------------------------------- | ------------- | ------------------------------------------------ |
| `**clap`** 4.x (`derive` feature) | Full-featured arg parser, derive API         | **[stable]**  | **Canonical.**                                   |
| `argh`                            | Minimal derive parser, Google-origin         | [mature]      | Tiny binaries, no help auto-wrap.                |
| `bpaf`                            | Combinator-first (plus derive); minimal deps | [mature]      | You want combinators or hate clap's binary size. |
| `lexopt`                          | Hand-rolled argument parsing helper          | [mature]      | You want total control, no macros.               |
| `pico-args`                       | Minimal, hand-parse                          | [mature]      | Fastest-to-compile.                              |
| `gumdrop`                         | Derive-based                                 | [maintenance] | Legacy.                                          |
| `structopt`                       | Predecessor to `clap-derive`                 | [deprecated]  | Never for new code.                              |


### Shell completions / man pages


| Crate                   | Role                                                                 |
| ----------------------- | -------------------------------------------------------------------- |
| `clap_complete`         | Generate bash/zsh/fish/pwsh/elvish completions from `clap::Command`. |
| `clap_complete_nushell` | Nushell completions.                                                 |
| `clap_mangen`           | Generate troff man pages.                                            |
| `clap_complete_fig`     | Fig.                                                                 |


### Interactive UI


| Crate                         | Role                                                        | Status        | Pick when                          |
| ----------------------------- | ----------------------------------------------------------- | ------------- | ---------------------------------- |
| `**indicatif`**               | Progress bars, spinners, multi-progress                     | [stable]      | **Canonical.**                     |
| `**dialoguer`**               | Confirm, input, select, multiselect, password, fuzzy-select | [stable]      | **Canonical prompts.**             |
| `inquire`                     | Modern prompt alternative                                   | [mature]      | Richer prompt types.               |
| `console`                     | Term detection, colors, styling primitives                  | [stable]      | Underlies `indicatif`/`dialoguer`. |
| `crossterm`                   | Cross-platform terminal control (keys, cursor, alt-screen)  | [stable]      | TUIs.                              |
| `termion`                     | Unix-only terminal control                                  | [maintenance] | Legacy.                            |
| `ratatui` (formerly `tui-rs`) | Terminal UI framework                                       | [stable]      | **Canonical TUI.**                 |
| `cursive`                     | Alt TUI framework                                           | [mature]      | Form-heavy TUIs.                   |


### Coloring


| Crate                        | Role                                               | Pick when                      |
| ---------------------------- | -------------------------------------------------- | ------------------------------ |
| `owo-colors`                 | Zero-alloc, const-friendly, nested styles          | **Canonical for new code.**    |
| `colored`                    | Ergonomic `.red()`, `.bold()`                      | Widely used, slightly heavier. |
| `nu-ansi-term` / `ansi_term` | Legacy                                             | Avoid.                         |
| `termcolor`                  | Used by rustc, supports Windows consoles uniformly | Build tools, lint output.      |
| `anstream`                   | Terminal color translation on Windows              | Underlies clap 4 coloring.     |


### Canonical CLI stack

```toml
clap = { version = "4", features = ["derive", "env", "wrap_help", "cargo", "unicode", "string"] }
clap_complete = "4"
clap_mangen   = "0.2"
indicatif  = "0.17"
dialoguer  = { version = "0.11", features = ["fuzzy-select", "password", "history"] }
owo-colors = "4"
anyhow     = "1"
tracing    = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
```

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, env = "APP_CONFIG")]
    config: Option<std::path::PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd { Build { #[arg(long)] release: bool }, Run }

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    /* ... */
    Ok(())
}
```

---

## 09 Database

Tag: `11-ecosystem-crate-picks`.

### Async SQL


| Crate                                  | Role                                            | Status       | Pick when                                             |
| -------------------------------------- | ----------------------------------------------- | ------------ | ----------------------------------------------------- |
| `**sqlx**`                             | Async, query-at-compile-time-verified, no ORM   | **[stable]** | **Canonical async SQL.** Postgres/MySQL/SQLite/MSSQL. |
| `sea-orm`                              | Async ORM built on sqlx                         | [stable]     | You want relations/active-record.                     |
| `diesel`                               | Sync, type-safe query DSL, ORM-ish              | [stable]     | **Canonical sync ORM.**                               |
| `diesel-async`                         | Async adapter for diesel                        | [mature]     | Diesel DSL + async runtime.                           |
| `deadpool-postgres` + `tokio-postgres` | Low-level async Postgres                        | [stable]     | You want raw SQL pooling without sqlx.                |
| `rusqlite`                             | Sync SQLite wrapper                             | [stable]     | **Canonical SQLite.** Local-first apps.               |
| `libsql`                               | Turso's fork of SQLite with async + replication | [active]     | Edge SQLite.                                          |
| `refinery`                             | SQL migrations                                  | [mature]     | Alternative to sqlx/diesel CLI migrations.            |
| `sqlx-cli`                             | sqlx migration tool                             | [stable]     | With sqlx.                                            |
| `diesel_cli`                           | Diesel migrations                               | [stable]     | With diesel.                                          |


### Embedded / key-value


| Crate         | Role                                 | Pick when                                           |
| ------------- | ------------------------------------ | --------------------------------------------------- |
| `**redb`**    | Pure-Rust, ACID, single-file, B-tree | **Canonical pure-Rust embedded DB.**                |
| `sled`        | LSM, pure-Rust                       | **[maintenance]** — use redb or fjall for new code. |
| `fjall`       | LSM, pure-Rust, active               | Newer LSM alternative.                              |
| `rocksdb`     | C++ RocksDB bindings                 | Canonical for LSM at scale.                         |
| `heed`        | LMDB bindings                        | Read-heavy mmap store.                              |
| `rkv`         | LMDB-based (Mozilla)                 | [maintenance]                                       |
| `persy`       | Pure-Rust ACID KV                    | Alternative.                                        |
| `fst`         | Finite-state transducer maps         | Large ordered key sets.                             |
| `polodb-core` | Pure-Rust embedded NoSQL             | Document-shaped.                                    |
| `surrealdb`   | Multi-model DB (crate = client)      | You want SurrealDB.                                 |


### NoSQL clients


| Crate                          | Role                                                                    | Status   |
| ------------------------------ | ----------------------------------------------------------------------- | -------- |
| `redis` / `fred` / `bb8-redis` | Redis. `fred` for rich pipelines, `redis` crate for de-facto canonical. | [stable] |
| `mongodb`                      | Official async MongoDB driver                                           | [stable] |
| `elasticsearch`                | Official async client                                                   | [mature] |
| `meilisearch-sdk`              | Meilisearch client                                                      | [mature] |
| `qdrant-client`                | Vector DB                                                               | [active] |
| `opensearch`                   | Fork of elasticsearch client                                            | [mature] |


### Migrations

- sqlx → `sqlx migrate`.
- diesel → `diesel migration`.
- Runtime → `refinery`.
- Standalone → `sqlx-migrate`.

### Canonical Postgres stack

```toml
sqlx = { version = "0.8", default-features = false, features = [
  "runtime-tokio", "tls-rustls",
  "postgres", "macros", "migrate",
  "chrono", "uuid", "json", "bigdecimal",
] }
```

```rust
#[derive(sqlx::FromRow)]
struct User { id: i64, email: String }

let user: User = sqlx::query_as!(User,
    "SELECT id, email FROM users WHERE id = $1", user_id)
    .fetch_one(&pool).await?;
```

### Notes

- `sqlx` requires `DATABASE_URL` at compile time unless you use
`SQLX_OFFLINE=true` + `cargo sqlx prepare` (commit the `.sqlx/` dir).
- `diesel` 2.x has first-class Postgres + MySQL + SQLite; pick features
carefully.
- Never use `postgres-native-tls` in containers — `rustls` works without
system OpenSSL.

---

## 10 Testing

Tag: `10-testing-and-tooling`.


| Crate / tool            | Role                                                           | Status        | Pick when                                               |
| ----------------------- | -------------------------------------------------------------- | ------------- | ------------------------------------------------------- |
| `**cargo-nextest**`     | Parallel test runner, ~60% faster than `cargo test`            | **[stable]**  | **Canonical test runner.**                              |
| `**proptest`**          | Property testing with shrinking                                | [stable]      | **Canonical proptest.**                                 |
| `quickcheck`            | Older property testing                                         | [maintenance] | Legacy; prefer proptest.                                |
| `**insta`**             | Snapshot testing (`assert_snapshot!`, `assert_yaml_snapshot!`) | [stable]      | Golden-file tests. Pair with `cargo-insta`.             |
| `expect-test`           | Inline snapshots (in-source)                                   | [mature]      | When you want the expectation visible at the call site. |
| `**rstest`**            | Parametrized tests, fixtures                                   | [stable]      | Table-driven tests.                                     |
| `test-case`             | `#[test_case(...)]` attribute                                  | [mature]      | Alternative to rstest parametric.                       |
| `**mockall**`           | Mock object generator, derive + trait                          | [stable]      | **Canonical mock.**                                     |
| `faux`                  | Alt mock lib, less boilerplate                                 | [mature]      | Lighter mocks for small traits.                         |
| `mry`                   | Another mocker                                                 | [active]      |                                                         |
| `assert_cmd`            | Test `bin` crates — invoke, assert stdout/exit                 | [stable]      | **Canonical CLI testing.**                              |
| `assert_fs`             | Filesystem fixture assertions                                  | [stable]      | Pair with assert_cmd.                                   |
| `predicates`            | Combinators for assertions                                     | [stable]      | Pair with assert_cmd.                                   |
| `wiremock`              | HTTP mock server for tests                                     | [stable]      | **Canonical HTTP mock.**                                |
| `mockito`               | Simpler HTTP mock (sync)                                       | [mature]      |                                                         |
| `httpmock`              | Another HTTP mock                                              | [mature]      |                                                         |
| `cargo-fuzz`            | libFuzzer integration                                          | [stable]      | **Canonical fuzzing.**                                  |
| `afl.rs`                | AFL integration                                                | [mature]      | When you want AFL specifically.                         |
| `honggfuzz`             | Honggfuzz integration                                          | [mature]      |                                                         |
| `arbitrary`             | Structured fuzzing inputs                                      | [stable]      | Pair with cargo-fuzz.                                   |
| `bolero`                | Unified fuzz + property runner                                 | [active]      | Run one harness under multiple engines.                 |
| `criterion`             | Statistical benchmarks                                         | [stable]      | **Canonical benches.**                                  |
| `divan`                 | New benchmark lib                                              | [active]      | Smaller alternative to criterion.                       |
| `iai` / `iai-callgrind` | Instruction-count benches                                      | [mature]      | Deterministic CI benches.                               |
| `**cargo-mutants`**     | Mutation testing                                               | [active]      | Find tests that "pass vacuously".                       |
| `cargo-tarpaulin`       | Coverage (Linux)                                               | [stable]      | Canonical coverage.                                     |
| `cargo-llvm-cov`        | Coverage via llvm-cov (cross-platform)                         | [stable]      | **Preferred coverage for CI.**                          |
| `grcov`                 | Mozilla coverage                                               | [mature]      | Firefox-heritage teams.                                 |
| `loom`                  | Concurrency model checker                                      | [stable]      | Unsafe/lockless data structures.                        |
| `shuttle`               | Concurrency fuzzing (alt to loom)                              | [mature]      |                                                         |
| `trybuild`              | Compile-fail tests for proc-macros                             | [stable]      | **Canonical proc-macro testing.**                       |
| `trycmd`                | Integration tests for CLI snapshot                             | [mature]      |                                                         |
| `macrotest`             | Expand macros in tests                                         | [mature]      |                                                         |


### Canonical test stack

```toml
[dev-dependencies]
tokio      = { version = "1", features = ["macros", "rt-multi-thread", "test-util"] }
rstest     = "0.21"
proptest   = "1"
insta      = { version = "1", features = ["yaml", "json", "redactions"] }
mockall    = "0.12"
wiremock   = "0.6"
assert_cmd = "2"
predicates = "3"
assert_fs  = "1"
```

```rust
use rstest::rstest;

#[rstest]
#[case("alice", true)]
#[case("", false)]
fn validates(#[case] input: &str, #[case] ok: bool) { /* ... */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runs() { /* ... */ }
```

### Run-these commands

```
cargo install cargo-nextest --locked
cargo install cargo-insta   --locked
cargo install cargo-llvm-cov --locked
cargo install cargo-mutants --locked
cargo install cargo-fuzz    --locked
cargo install cargo-deny    --locked
cargo install cargo-audit   --locked
```

---

## 11 Data structures & utilities

Tag: `11-ecosystem-crate-picks`.


| Crate                                         | Role                                                                                                        | Status       | Pick when                                                                     |
| --------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------ | ----------------------------------------------------------------------------- |
| `**itertools**`                               | Iterator adapters (`chunks`, `group_by`, `unique`, `dedup`, `cartesian_product`, `sorted`, `k_smallest`, …) | [stable]     | **Canonical.** Almost always wanted.                                          |
| `**rayon`**                                   | Data parallelism (`par_iter`)                                                                               | [stable]     | **Canonical for sync parallelism.**                                           |
| `crossbeam`                                   | Lock-free channels, scoped threads, atomics, epoch                                                          | [stable]     | Scoped threads (`crossbeam::scope`) and MPMC channels.                        |
| `crossbeam-channel`                           | Unbounded/bounded channels faster than std's mpsc                                                           | [stable]     | Sync MPMC.                                                                    |
| `flume`                                       | MPMC channel, simpler API                                                                                   | [stable]     | Alternative to crossbeam-channel.                                             |
| `parking_lot`                                 | Faster `Mutex`, `RwLock` than std; smaller                                                                  | [stable]     | Always. Drop-in for `std::sync::Mutex`.                                       |
| `dashmap`                                     | Concurrent hashmap (sharded)                                                                                | [stable]     | Concurrent maps without `RwLock<HashMap>` bottleneck.                         |
| `scc`                                         | Alternative concurrent hashmap/treeindex                                                                    | [mature]     |                                                                               |
| `**ahash`**                                   | Fast non-cryptographic HashMap hasher                                                                       | [stable]     | **Default non-crypto hasher.** DoS-safe-ish.                                  |
| `fxhash`                                      | Firefox-speed hasher (not DoS-safe)                                                                         | [stable]     | Internal-only maps.                                                           |
| `rustc-hash`                                  | Another fast hasher                                                                                         | [stable]     | rustc/compiler-adjacent code.                                                 |
| `hashbrown`                                   | SwissTable impl (same as std's HashMap)                                                                     | [stable]     | no_std HashMap.                                                               |
| `indexmap`                                    | Insertion-ordered HashMap/HashSet                                                                           | [stable]     | Deterministic iteration.                                                      |
| `smallvec`                                    | Stack-allocated `Vec` up to N                                                                               | [stable]     | Tiny vecs on the hot path.                                                    |
| `tinyvec`                                     | Stack-only or spill-to-heap                                                                                 | [stable]     | No-unsafe alternative to smallvec.                                            |
| `arrayvec`                                    | Fixed-capacity stack `Vec`                                                                                  | [stable]     | `no_std`, bounded capacity.                                                   |
| `smol_str`                                    | Small-string optimization for immutable strings                                                             | [stable]     | Short frequent strings (identifiers).                                         |
| `compact_str`                                 | 24-byte inline `String` replacement                                                                         | [stable]     | Short mutable strings.                                                        |
| `bytes`                                       | `Bytes`, `BytesMut` — reference-counted byte buffers                                                        | [stable]     | **Canonical for network IO.**                                                 |
| `bytemuck`                                    | Safe transmute between POD types                                                                            | [stable]     | Zero-copy buffer reinterpret.                                                 |
| `zerocopy`                                    | Safe transmute + bounded unsafe                                                                             | [stable]     | Rival to bytemuck; kernel-heritage.                                           |
| `**once_cell`** / std `OnceLock` + `LazyLock` | One-time init                                                                                               | std 1.80+    | Use `std::sync::LazyLock` — stable since 1.80; drop `once_cell` for new code. |
| `lazy_static`                                 | Historical lazy static                                                                                      | [deprecated] | Never for new code.                                                           |
| `arc-swap`                                    | Atomic `Arc<T>` swap                                                                                        | [stable]     | Lock-free config reload.                                                      |
| `triomphe`                                    | Alternative `Arc` (no weak)                                                                                 | [stable]     | Smaller `Arc` where you never need `Weak`.                                    |
| `slab`                                        | Arena with stable keys                                                                                      | [stable]     | IDs into a compact vec.                                                       |
| `typed-arena`                                 | Arena for same-typed allocations                                                                            | [stable]     | Many same-type short-lived objects.                                           |
| `bumpalo`                                     | Bump arena                                                                                                  | [stable]     | Per-request arena.                                                            |
| `generational-arena`                          | Arena with generation counter                                                                               | [mature]     | Reuse-safe IDs.                                                               |
| `slotmap`                                     | Like generational-arena but faster                                                                          | [stable]     | Game entity IDs.                                                              |
| `bitvec`                                      | Bit-level Vec                                                                                               | [stable]     | Canonical bit manipulation.                                                   |
| `bit-set`                                     | Bitset                                                                                                      | [mature]     |                                                                               |
| `roaring`                                     | Compressed bitmaps                                                                                          | [stable]     | Large sparse bitsets.                                                         |
| `fixedbitset`                                 | Dense bitset                                                                                                | [stable]     | petgraph's choice.                                                            |
| `memchr`                                      | Vectorized byte/char search                                                                                 | [stable]     | Search primitives (underlies many parsers).                                   |
| `bstr`                                        | Byte-string type with unicode-aware ops                                                                     | [stable]     | CLI tools that process bytes (ripgrep).                                       |
| `num_cpus`                                    | Detect logical CPU count                                                                                    | [stable]     | Legacy; prefer `std::thread::available_parallelism`.                          |
| `humantime`                                   | Parse/format durations like "2h 30m"                                                                        | [stable]     | Config duration parsing.                                                      |
| `humansize`                                   | 1024 → "1 KiB"                                                                                              | [mature]     |                                                                               |


### Canonical "always-add" set

```toml
itertools    = "0.14"
rayon        = "1"
parking_lot  = "0.12"
dashmap      = "6"
ahash        = "0.8"
bytes        = "1"
smallvec     = { version = "1", features = ["union", "const_generics"] }
indexmap     = "2"
```

---

## 12 Regex & parsing

Tag: `11-ecosystem-crate-picks`.

### Regex


| Crate          | Role                         | Status       | Pick when                                                                      |
| -------------- | ---------------------------- | ------------ | ------------------------------------------------------------------------------ |
| `**regex**`    | Official regex               | **[stable]** | **Canonical.** Linear-time, no backtracking.                                   |
| `regex-lite`   | Smaller, no Unicode          | [stable]     | Binary-size-sensitive.                                                         |
| `fancy-regex`  | Backreferences, lookaround   | [mature]     | You need features `regex` doesn't support.                                     |
| `pcre2`        | PCRE2 bindings               | [mature]     | You literally need PCRE semantics.                                             |
| `onig`         | Oniguruma bindings           | [mature]     | Legacy.                                                                        |
| `aho-corasick` | Multi-pattern literal search | [stable]     | **Underlies `regex` for large alt-groups.** Use directly for dictionary match. |


### Parser combinators / generators


| Crate         | Role                                              | Status        | Pick when                              |
| ------------- | ------------------------------------------------- | ------------- | -------------------------------------- |
| `**winnow`**  | Modern fork of nom; ergonomic, zero-copy          | [stable]      | **Canonical combinator parser.**       |
| `nom`         | Original combinator parser                        | [stable]      | Existing code or you prefer its style. |
| `pest`        | PEG grammar in external file                      | [stable]      | You want a declarative grammar.        |
| `**logos`**   | Perfect-hash lexer via derive                     | [stable]      | **Canonical lexer.**                   |
| `lalrpop`     | LALR(1) parser generator                          | [mature]      | Classic compiler grammar.              |
| `tree-sitter` | Incremental parser framework                      | [mature]      | IDEs, editors, syntax highlight.       |
| `chumsky`     | Combinator parser with first-class error recovery | [active]      | DSLs with user-facing errors.          |
| `peg`         | Rust-macro PEG                                    | [mature]      |                                        |
| `combine`     | Combinator parser                                 | [maintenance] |                                        |


### Canonical picks

- One-off text matching → `**regex`**.
- Large dictionary substring match → `**aho-corasick`**.
- Lexer → `**logos**` (+ combinator downstream).
- Full-featured parser → `**winnow**` (for new code) or `chumsky` (for great errors).
- Grammar-in-file → `**pest**`.

```toml
regex  = "1"
winnow = "0.6"
logos  = "0.15"
```

---

## 13 Date & time

Tag: `11-ecosystem-crate-picks`.


| Crate             | Role                                                     | Status                        | Pick when                                                                |
| ----------------- | -------------------------------------------------------- | ----------------------------- | ------------------------------------------------------------------------ |
| `**jiff**`        | Modern TZ-aware datetime, like Java `java.time`          | [active, rapidly stabilizing] | **Canonical for new code (2026).** Designed by `regex`/`ripgrep` author. |
| `chrono`          | De-facto standard for a decade; timezone via `chrono-tz` | [stable]                      | Existing codebases; ubiquitous deps.                                     |
| `time`            | std-friendly; `OffsetDateTime`; `#![no_std]`-friendly    | [stable]                      | `no_std`, or you specifically want the `time` API.                       |
| `humantime`       | Parse "3h 5m"                                            | [stable]                      | Config durations.                                                        |
| `speedate`        | Fast ISO-8601 parser                                     | [mature]                      | When parsing is the hot path.                                            |
| `chrono-tz`       | IANA tz database for chrono                              | [stable]                      | With chrono.                                                             |
| `chrono-humanize` | "2 hours ago" formatting                                 | [mature]                      |                                                                          |
| `duration-str`    | "1h30m" parser                                           | [mature]                      |                                                                          |


### Canonical

- **New code (2026+):** `jiff`. Has proper `Zoned`, `Timestamp`, `civil::Date`, arithmetic that respects calendars, and is actively maintained.
- **Legacy interop with `serde_with`/`sqlx`/etc.:** `chrono` until `jiff`
integrations land across your stack.
- `**no_std`:** `time` with default features off.

### Typical imports

```toml
# New code
jiff = { version = "0.1", features = ["serde"] }

# Legacy-compatible
chrono = { version = "0.4", default-features = false, features = ["std", "clock", "serde"] }
chrono-tz = "0.10"
```

---

## 14 UUID


| Crate      | Status   | Pick when                                     |
| ---------- | -------- | --------------------------------------------- |
| `**uuid**` | [stable] | **Canonical.** All standard versions plus v7. |
| `ulid`     | [stable] | You need lexicographically sortable IDs.      |
| `nanoid`   | [mature] | Shorter string IDs.                           |
| `cuid2`    | [mature] | Collision-resistant string IDs.               |


```toml
uuid = { version = "1", features = ["v4", "v7", "serde", "fast-rng"] }
```

- **Pick `v7` for new databases** — sortable, timestamp-leading, reduces
B-tree fragmentation on Postgres PKs.
- `v4` for random opaque IDs.
- Avoid `v1`/`v6` (MAC-address leakage).

---

## 15 Random


| Crate           | Role                                    | Status       | Pick when                         |
| --------------- | --------------------------------------- | ------------ | --------------------------------- |
| `**rand`**      | Standard RNG facade + distributions     | **[stable]** | **Canonical.**                    |
| `rand_chacha`   | ChaCha-based CSPRNG                     | [stable]     | Reproducible secure RNG.          |
| `rand_xoshiro`  | Fast non-crypto RNG                     | [stable]     | Simulation.                       |
| `rand_pcg`      | PCG family                              | [stable]     |                                   |
| `fastrand`      | Tiny, no-deps, thread-local RNG         | [stable]     | Don't want to pull rand's deps.   |
| `**getrandom`** | OS-level randomness                     | [stable]     | Seed a CSPRNG; or direct entropy. |
| `rand_distr`    | Normal, gamma, beta, etc. distributions | [stable]     | Statistics.                       |
| `nanorand`      | Small, no_std                           | [mature]     | `no_std` non-crypto.              |


```toml
rand = "0.9"
fastrand = "2"   # if you just want `fastrand::u32(..)`
```

- For cryptography RNG: do **not** use rand's default; use `OsRng` from `rand`
or `getrandom` directly.

---

## 16 Cryptography & TLS

Tag: `11-ecosystem-crate-picks`. Curator rule: cryptography is the one domain
where "canonical and use nothing else" matters most.

### TLS stacks


| Crate                 | Role                                              | Status       | Pick when                                      |
| --------------------- | ------------------------------------------------- | ------------ | ---------------------------------------------- |
| `**rustls`**          | Pure-Rust TLS 1.2/1.3                             | **[stable]** | **Canonical.**                                 |
| `tokio-rustls`        | tokio integration                                 | [stable]     | With tokio/hyper/reqwest.                      |
| `rustls-native-certs` | Load system root CAs for rustls                   | [stable]     | Prod clients.                                  |
| `webpki-roots`        | Bundled Mozilla roots                             | [stable]     | When you don't want to depend on system store. |
| `rustls-pemfile`      | Parse PEM certs/keys                              | [stable]     |                                                |
| `native-tls`          | System TLS (SChannel / SecureTransport / OpenSSL) | [mature]     | **Avoid unless the platform mandates it.**     |
| `openssl`             | OpenSSL bindings                                  | [stable]     | Legacy integration only.                       |
| `boring`              | Cloudflare's BoringSSL fork                       | [mature]     | CF-aligned infra.                              |
| `s2n-tls`             | AWS s2n bindings                                  | [mature]     | AWS-aligned infra.                             |


### Hashing


| Crate                   | Role                                            | Status   | Pick when                                        |
| ----------------------- | ----------------------------------------------- | -------- | ------------------------------------------------ |
| `**sha2`** (RustCrypto) | SHA-256/384/512                                 | [stable] | Canonical SHA-2.                                 |
| `sha1` (RustCrypto)     | SHA-1                                           | [stable] | Git, old protocols. Do not use for new security. |
| `sha3` (RustCrypto)     | SHA-3/Keccak                                    | [stable] | NIST/crypto interop.                             |
| `**blake3`**            | BLAKE3 (fast, parallel)                         | [stable] | **Canonical non-crypto fast hash.**              |
| `blake2`                | BLAKE2                                          | [stable] |                                                  |
| `md-5`                  | MD5                                             | [stable] | Legacy integrity only.                           |
| `digest`                | Trait (`Digest`, `Output`) shared by RustCrypto | [stable] |                                                  |


### Symmetric


| Crate                  | Role                    | Status   |
| ---------------------- | ----------------------- | -------- |
| `**aes-gcm`**          | AES-GCM AEAD            | [stable] |
| `**chacha20poly1305`** | ChaCha20-Poly1305       | [stable] |
| `aes-gcm-siv`          | Misuse-resistant AEAD   | [stable] |
| `aes`                  | Block cipher primitive  | [stable] |
| `chacha20`             | Stream cipher primitive | [stable] |


### Asymmetric / signatures


| Crate                             | Role                                | Status   |
| --------------------------------- | ----------------------------------- | -------- |
| `**ed25519-dalek**`               | Ed25519 signing                     | [stable] |
| `x25519-dalek`                    | X25519 DH                           | [stable] |
| `curve25519-dalek`                | Low-level curve arithmetic          | [stable] |
| `p256` / `p384` / `p521` / `k256` | NIST curves (pure Rust)             | [stable] |
| `rsa`                             | RSA (pure Rust, no OpenSSL)         | [stable] |
| `ring`                            | Amazon/Brian Smith's curated crypto | [stable] |


### Password hashing


| Crate                     | Role               | Status   | Pick when                    |
| ------------------------- | ------------------ | -------- | ---------------------------- |
| `**argon2**` (RustCrypto) | Argon2id           | [stable] | **Canonical for passwords.** |
| `scrypt`                  | scrypt             | [stable] | Legacy compat.               |
| `bcrypt`                  | bcrypt             | [stable] | Legacy DB.                   |
| `password-hash`           | `PHC`-string trait | [stable] | Shared by RustCrypto hashes. |


### Tokens / JOSE


| Crate          | Role                    | Status   |
| -------------- | ----------------------- | -------- |
| `jsonwebtoken` | JWT HS*/RS*/ES*/Ed25519 | [stable] |
| `josekit`      | Full JOSE (JWS/JWE/JWK) | [mature] |
| `biscuit`      | JWT (alt)               | [mature] |


### Canonical imports

```toml
rustls = { version = "0.23", features = ["aws_lc_rs"] }  # or "ring" provider
rustls-native-certs = "0.8"
sha2 = "0.10"
blake3 = "1"
argon2 = "0.5"
aes-gcm = "0.10"
chacha20poly1305 = "0.10"
ed25519-dalek = "2"
rand = "0.9"
zeroize = "1"
secrecy = "0.10"   # `Secret<T>` wrapper that zeroes on drop
```

### Rules

- **Never roll your own.** Use RustCrypto or ring.
- **TLS in prod = rustls.** Native-tls and openssl only where the platform
demands it.
- **Hold secrets in `secrecy::Secret<T>`.** Implements `Zeroize` on drop.
- **Passwords = Argon2id.** Never SHA-* a password alone.

---

## 17 Compression


| Crate               | Role                                                       | Status   | Pick when                     |
| ------------------- | ---------------------------------------------------------- | -------- | ----------------------------- |
| `**flate2`**        | gzip, zlib, deflate                                        | [stable] | **Canonical gzip/zlib.**      |
| `**zstd`**          | Zstandard                                                  | [stable] | **Canonical Zstd.**           |
| `lz4_flex`          | Pure-Rust LZ4                                              | [stable] | LZ4 without C deps.           |
| `lz4`               | C-bindings LZ4                                             | [stable] |                               |
| `snap`              | Snappy                                                     | [stable] | Snappy interop.               |
| `brotli`            | Brotli (pure Rust)                                         | [stable] | Web-serving content-encoding. |
| `xz2`               | XZ/LZMA                                                    | [stable] |                               |
| `bzip2`             | Bzip2                                                      | [stable] |                               |
| `zip`               | Zip archive                                                | [stable] | **Canonical ZIP.**            |
| `tar`               | TAR archive                                                | [stable] | **Canonical TAR.**            |
| `async-compression` | Async wrappers (tokio + futures) for gzip/zstd/brotli/etc. | [stable] | Tokio streams.                |
| `zip-extract`       | High-level extract                                         | [mature] |                               |


```toml
flate2 = "1"
zstd   = "0.13"
brotli = "7"
tar    = "0.4"
zip    = "2"
```

---

## 18 Image, media, SVG, PDF

### Image


| Crate                      | Role                                          | Status   |
| -------------------------- | --------------------------------------------- | -------- |
| `**image**`                | Decode/encode PNG/JPEG/GIF/WebP/TIFF/BMP/AVIF | [stable] |
| `imageproc`                | Filters, morphology on top of `image`         | [mature] |
| `fast_image_resize`        | SIMD-accelerated resize                       | [stable] |
| `kamadak-exif`             | EXIF parser                                   | [mature] |
| `jpeg-decoder` / `png`     | Underlying format crates                      | [stable] |
| `webp`                     | WebP                                          | [mature] |
| `avif-serialize` / `ravif` | AVIF encoding                                 | [mature] |


### SVG / vector


| Crate                | Role                                  | Status   |
| -------------------- | ------------------------------------- | -------- |
| `**resvg**`          | SVG rasterization                     | [stable] |
| `usvg` / `tiny-skia` | Underlying crates                     | [stable] |
| `svg`                | SVG writer                            | [mature] |
| `lyon`               | 2D path tessellation                  | [stable] |
| `kurbo`              | 2D curve math                         | [stable] |
| `piet`               | Retained-mode 2D graphics abstraction | [mature] |


### PDF


| Crate                | Role                           | Status   |
| -------------------- | ------------------------------ | -------- |
| `lopdf`              | Low-level PDF write            | [mature] |
| `printpdf`           | PDF generation                 | [mature] |
| `pdf-extract`        | Text extraction                | [mature] |
| `genpdf`             | High-level PDF generation      | [mature] |
| `typst` (as a crate) | Full Typst compiler as library | [active] |


### Audio


| Crate       | Role                                 | Status   |
| ----------- | ------------------------------------ | -------- |
| `cpal`      | Cross-platform audio I/O             | [stable] |
| `rodio`     | High-level audio playback on cpal    | [stable] |
| `kira`      | Game audio                           | [mature] |
| `symphonia` | Pure-Rust decoder (MP3/FLAC/AAC/OGG) | [stable] |
| `hound`     | WAV                                  | [stable] |
| `dasp`      | DSP primitives                       | [mature] |


### Video


| Crate              | Role               | Status   |
| ------------------ | ------------------ | -------- |
| `ffmpeg-next`      | FFmpeg bindings    | [mature] |
| `gstreamer`        | GStreamer bindings | [mature] |
| `re_video` (rerun) | Pure-Rust demux    | [active] |


---

## 19 GUI

Tag: `11-ecosystem-crate-picks`.


| Framework             | Paradigm                              | Status                       | Pick when                                                     |
| --------------------- | ------------------------------------- | ---------------------------- | ------------------------------------------------------------- |
| `**egui**` + `eframe` | Immediate-mode                        | **[stable, rapid releases]** | **Canonical** debug UI / simple apps / embedded debug panels. |
| `**iced`**            | Elm-ish retained                      | [mature, approaching 0.14]   | Desktop apps, cross-platform.                                 |
| `**tauri`** 2         | HTML+CSS+JS front with Rust back      | **[stable]**                 | **Canonical for "Electron killer"**; mobile 2.0+.             |
| `slint`               | DSL-based, embedded-friendly          | [stable]                     | Embedded Linux, commercial apps.                              |
| `dioxus`              | React-like (VDOM); desktop/web/mobile | [stable]                     | React-shaped mental model.                                    |
| `gtk4-rs`             | GTK4 bindings                         | [stable]                     | Gnome integration.                                            |
| `relm4`               | Elm-ish on gtk4-rs                    | [mature]                     | Gtk apps, Rust-first.                                         |
| `fltk-rs`             | FLTK bindings                         | [mature]                     | Tiny native apps.                                             |
| `makepad`             | Live-coded GPU UI                     | [active]                     | Experimental.                                                 |
| `xilem`               | Linebender next-gen UI                | [active/pre-1.0]             | Watch this space.                                             |
| `masonry`             | Retained widget tree under xilem      | [active]                     | With xilem.                                                   |
| `yew` / `leptos`      | Web-only (see WASM)                   | —                            | —                                                             |
| `cushy`               | Skia-based UI                         | [active]                     |                                                               |
| `ribir`               | Declarative reactive                  | [active]                     |                                                               |
| `freya`               | React-like, Skia                      | [active]                     |                                                               |


### Canonical

- **Debug overlay / dev tool:** egui.
- **Shipping desktop app:** tauri (if web front is OK) or iced (native).
- **Embedded Linux:** slint.

```toml
# egui standalone
eframe = { version = "0.29", features = ["default_fonts", "glow", "wgpu"] }
egui   = "0.29"
```

---

## 20 Game development

Tag: `11-ecosystem-crate-picks`.

### Engines


| Crate                | Role                          | Status                         | Pick when                                   |
| -------------------- | ----------------------------- | ------------------------------ | ------------------------------------------- |
| `**bevy**`           | ECS-first, wgpu-based engine  | **[stable, monthly releases]** | **Canonical for a new Rust game.**          |
| `macroquad`          | Simple, immediate-mode 2D     | [stable]                       | Tiny games, game jams.                      |
| `fyrox`              | Scene-graph 3D engine, editor | [mature]                       | Editor-first workflow.                      |
| `ggez`               | LÖVE-inspired 2D              | [maintenance]                  | Tiny 2D.                                    |
| `amethyst`           | —                             | **[discontinued]**             | Never for new code — team migrated to bevy. |
| `piston`             | —                             | **[maintenance]**              | Legacy.                                     |
| `bracket-lib` (rltk) | Roguelike toolkit             | [mature]                       | Roguelikes.                                 |


### ECS


| Crate      | Role                          | Status        |
| ---------- | ----------------------------- | ------------- |
| `bevy_ecs` | Bevy's ECS, usable standalone | [stable]      |
| `hecs`     | Minimal archetype ECS         | [mature]      |
| `specs`    | Parallel ECS                  | [maintenance] |
| `legion`   | ECS                           | [maintenance] |
| `evenio`   | Event-based ECS               | [active]      |


### Graphics / rendering


| Crate               | Role                                                     | Status        |
| ------------------- | -------------------------------------------------------- | ------------- |
| `**wgpu`**          | Safe WebGPU abstraction over Vulkan/Metal/DX12/GL/WebGPU | [stable]      |
| `ash`               | Vulkan bindings                                          | [stable]      |
| `vulkano`           | High-level Vulkan                                        | [mature]      |
| `glow`              | GL 3.3+ / GLES 2.0+                                      | [stable]      |
| `metal`             | Metal bindings                                           | [mature]      |
| `rend3`             | High-level 3D renderer on wgpu                           | [maintenance] |
| `winit`             | Window + input event loop                                | [stable]      |
| `glutin`            | GL context                                               | [mature]      |
| `raw-window-handle` | Window handle interop trait                              | [stable]      |


### Physics


| Crate                           | Role                                  | Status                     |
| ------------------------------- | ------------------------------------- | -------------------------- |
| `**rapier2d`** / `**rapier3d**` | 2D/3D rigid-body physics              | [stable]                   |
| `avian` (formerly bevy_xpbd)    | Bevy-integrated physics               | [active]                   |
| `parry`                         | Collision primitives (used by rapier) | [stable]                   |
| `nphysics`                      | —                                     | [discontinued] use rapier. |


### Audio in games

- Use `kira` (game-centric) or `rodio` (simple).

### Input

- `gilrs` — gamepads.
- `winit` — keyboard/mouse via window events.
- `gamepad-rs` — alt.

### Canonical

```toml
bevy   = "0.14"
rapier3d = { version = "0.22", features = ["simd-stable"] }
kira   = "0.9"
gilrs  = "0.11"
```

### Are We Game Yet — status

- Game engine: **yes**, pick bevy.
- Graphics: **yes**, wgpu is production.
- Physics: **yes**, rapier/parry.
- Audio: **yes** (kira/rodio/cpal).
- Tooling: **partial** — editors are fragmented; Fyrox has an editor, bevy
has `bevy_editor_pls`, Slint has Slint-live, but nothing matches Unity UI.

---

## 21 Embedded

Tag: `11-ecosystem-crate-picks`.


| Crate                        | Role                                                                    | Status                                        |
| ---------------------------- | ----------------------------------------------------------------------- | --------------------------------------------- |
| `**embassy`**                | Async executor + HAL for Cortex-M / RISC-V / ESP / STM32 / nRF / RP2040 | **[stable]** — canonical.                     |
| `embedded-hal`               | Trait layer (SPI, I2C, GPIO, delay) — 1.0 stable                        | [stable]                                      |
| `embedded-hal-async`         | Async traits                                                            | [stable]                                      |
| `cortex-m`                   | Cortex-M core (low-level)                                               | [stable]                                      |
| `cortex-m-rt`                | Runtime for Cortex-M binaries                                           | [stable]                                      |
| `cortex-m-rtic`              | Real-Time Interrupt-driven Concurrency (RTIC) framework                 | [stable]                                      |
| `rtic`                       | New name for rtic 2.x                                                   | [active]                                      |
| `riscv` / `riscv-rt`         | RISC-V equivalents                                                      | [stable]                                      |
| `esp-hal`                    | ESP32 HAL                                                               | [active]                                      |
| `esp-idf-hal`                | ESP-IDF binding                                                         | [mature]                                      |
| `stm32*-hal`                 | Per-family HALs                                                         | [mature]                                      |
| `rp2040-hal` / `rp235x-hal`  | Raspberry Pi Pico                                                       | [mature]                                      |
| `nrf-hal`                    | Nordic                                                                  | [mature]                                      |
| `defmt`                      | Deferred formatting, tiny logs                                          | [stable]                                      |
| `probe-rs`                   | Host-side flash/debug tool                                              | [stable] — canonical replacement for OpenOCD. |
| `panic-probe` / `panic-halt` | Panic handler                                                           | [stable]                                      |
| `heapless`                   | `no_std` Vec/HashMap/String with fixed capacity                         | [stable]                                      |
| `postcard`                   | Canonical no_std wire format                                            | [stable]                                      |
| `embedded-graphics`          | Pixel graphics                                                          | [stable]                                      |
| `smoltcp`                    | no_std TCP/IP stack                                                     | [mature]                                      |
| `embedded-svc`               | Service traits                                                          | [active]                                      |


### Canonical embedded stack

```toml
embassy-executor = { version = "0.6", features = ["arch-cortex-m", "executor-thread", "integrated-timers"] }
embassy-time     = "0.3"
embassy-stm32    = { version = "0.2", features = ["stm32h743zi", "time-driver-any"] }
defmt            = "0.3"
panic-probe      = "0.3"
heapless         = "0.8"
postcard         = "1"
```

Host side: `cargo install probe-rs-tools --locked`.

---

## 22 WebAssembly

Tag: `11-ecosystem-crate-picks`.

### Core interop


| Crate                      | Role                                                   | Status                                                                        |
| -------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `**wasm-bindgen**`         | JS<->Rust bridge                                       | [stable]                                                                      |
| `**js-sys**`               | JS builtin bindings                                    | [stable]                                                                      |
| `**web-sys**`              | Web API bindings (feature-gated)                       | [stable]                                                                      |
| `gloo`                     | High-level browser utilities (timers, storage, events) | [mature]                                                                      |
| `console_error_panic_hook` | Pipe panics to `console.error`                         | [stable]                                                                      |
| `wee_alloc`                | Tiny alloc                                             | [maintenance] — use default alloc since Rust size regressions largely closed. |


### Toolchain


| Tool               | Role                                        | Status   |
| ------------------ | ------------------------------------------- | -------- |
| `**wasm-pack**`    | Build + bundle + publish                    | [stable] |
| `trunk`            | Dev server + bundler for Rust-only web apps | [stable] |
| `wasm-bindgen-cli` | Required by wasm-pack/trunk                 | [stable] |


### Rust frontend frameworks


| Framework        | Paradigm                                           | Status       | Pick when                            |
| ---------------- | -------------------------------------------------- | ------------ | ------------------------------------ |
| `**leptos**`     | Fine-grained reactivity (SolidJS-style), SSR-first | **[stable]** | **Canonical for 2026 SSR apps.**     |
| `**dioxus`**     | VDOM, React-shaped; desktop + web + mobile         | [stable]     | Cross-platform UI with one codebase. |
| `yew`            | React/Elm-ish VDOM                                 | [mature]     | Existing code, simpler mental model. |
| `sycamore`       | Fine-grained, Solid-like                           | [mature]     | Alternative to leptos.               |
| `perseus`        | SSR framework on sycamore/leptos                   | [mature]     |                                      |
| `maud` / `rstml` | HTML macros (server-side)                          | [stable]     | Templating without framework.        |


### Runtime (non-browser WASM)

- `wasmtime` — canonical Wasmtime runtime embedder.
- `wasmer` — alternative runtime.
- `wasmi` — pure-Rust interpreter (no JIT).
- `wasm-tools` — CLI swiss-army.
- `wit-bindgen` — WASI Component Model bindings.
- `wasi` — direct WASI syscalls.

### Canonical leptos stack

```toml
leptos   = { version = "0.7", features = ["csr", "hydrate", "ssr"] }
leptos_axum = "0.7"
leptos_meta = "0.7"
leptos_router = "0.7"
server_fn = "0.7"
```

---

## 23 Derive helpers & macros

Tag: `11-ecosystem-crate-picks`.


| Crate                                  | Role                                                                           | Status                                                                  |
| -------------------------------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| `**derive_more**`                      | Derive `Display`, `From`, `Into`, `Add`, `Deref`, `AsRef`, `Constructor`, etc. | [stable]                                                                |
| `educe`                                | Alt to derive_more                                                             | [mature]                                                                |
| `**strum**` + `strum_macros`           | `EnumIter`, `Display`, `EnumString`, `AsRefStr`, `EnumCount`, `VariantNames`   | [stable]                                                                |
| `enum-iterator`                        | `Sequence` derive for enum iteration                                           | [stable]                                                                |
| `enum_dispatch`                        | Generate dispatch match over enum variants implementing a trait                | [stable]                                                                |
| `enum-ordinalize`                      | Ordinal arithmetic for enums                                                   | [mature]                                                                |
| `num_enum`                             | `Into/TryFrom<integer>` for C-like enums                                       | [stable]                                                                |
| `int-enum`                             | alt                                                                            | [mature]                                                                |
| `**bitflags**`                         | `Flag` bitset macros                                                           | [stable]                                                                |
| `cfg-if`                               | `cfg_if!` macro for nested cfg                                                 | [stable]                                                                |
| `paste`                                | Identifier concatenation in macros                                             | [stable]                                                                |
| `**thiserror**`                        | (see § 6)                                                                      |                                                                         |
| `**parse-display**`                    | `Display`/`FromStr` derive from format string                                  | [mature]                                                                |
| `displaydoc`                           | Derive `Display` from doc comment                                              | [mature]                                                                |
| `const_format`                         | Const-time `format!` and concat                                                | [mature]                                                                |
| `static_assertions`                    | `const_assert!` etc.                                                           | [stable]                                                                |
| `**pin-project**` / `pin-project-lite` | Safe `Pin` projections                                                         | [stable] — use `pin-project-lite` in libraries to avoid proc-macro dep. |
| `async-trait`                          | `async fn` in `dyn` traits (pre-1.75, or for dyn-safety)                       | [stable]                                                                |
| `trait-variant`                        | Generate Send/non-Send variants of async traits                                | [mature]                                                                |
| `dyn-clone`                            | Clone-safe `dyn Trait`                                                         | [mature]                                                                |
| `delegate`                             | Forwarding methods to a field                                                  | [mature]                                                                |
| `ambassador`                           | Alt delegation                                                                 | [mature]                                                                |
| `serde_with`                           | Cross-cutting serde helpers via derive attrs                                   | [stable]                                                                |
| `auto_impl`                            | Auto-impl trait for `Box<T>`, `&T`, `Arc<T>`                                   | [stable]                                                                |
| `educe`                                | Big-hammer derive                                                              | [mature]                                                                |
| `macro_rules_attribute`                | Use macro_rules as attribute                                                   | [stable]                                                                |
| `impls`                                | `impls!(T: Send + Sync)` macro                                                 | [mature]                                                                |


### Canonical

```toml
derive_more = { version = "1", features = ["full"] }
strum       = { version = "0.26", features = ["derive"] }
bitflags    = "2"
pin-project-lite = "0.2"
```

---

## 24 Numerics, science, ML

Tag: `11-ecosystem-crate-picks`.

### Numeric traits & ints


| Crate                      | Role                                                            |
| -------------------------- | --------------------------------------------------------------- |
| `num-traits`               | `Zero`, `One`, `Num`, `Float`, `Signed`. Canonical.             |
| `num`                      | Umbrella re-export (integers, bigint, complex, rational, iter). |
| `num-bigint`               | Arbitrary precision ints.                                       |
| `num-rational`             | Rationals.                                                      |
| `num-complex`              | Complex.                                                        |
| `num-integer`              | Int traits.                                                     |
| `ordered-float`            | `OrderedFloat<f64>`/`NotNan<f64>` for sorting.                  |
| `decimal` / `rust_decimal` | Base-10 decimal for money. Canonical: `**rust_decimal**`.       |
| `fraction`                 | Fractions.                                                      |
| `bigdecimal`               | Arbitrary-precision decimal.                                    |


### Linear algebra


| Crate          | Role                                                      | Status        |
| -------------- | --------------------------------------------------------- | ------------- |
| `**nalgebra**` | Dynamic + static dims; geometry primitives                | [stable]      |
| `**ndarray**`  | NumPy-shaped dyn array                                    | [stable]      |
| `glam`         | SIMD-friendly fixed-size vectors/matrices (game/graphics) | [stable]      |
| `ultraviolet`  | Alt to glam                                               | [mature]      |
| `cgmath`       | Legacy graphics math                                      | [maintenance] |
| `euclid`       | 2D/3D geometry types                                      | [mature]      |
| `faer`         | Dense LAPACK-style matrix routines, pure Rust             | [active]      |
| `sprs`         | Sparse matrices                                           | [mature]      |


### DataFrames & analytics


| Crate                | Role                                 | Status       |
| -------------------- | ------------------------------------ | ------------ |
| `**polars**`         | DataFrame — lazy/eager, Arrow-backed | **[stable]** |
| `datafusion`         | SQL engine on Arrow                  | [stable]     |
| `arrow` / `arrow-rs` | Apache Arrow                         | [stable]     |
| `parquet`            | Parquet files                        | [stable]     |
| `duckdb`             | DuckDB bindings                      | [mature]     |


### ML / DL


| Crate         | Role                                                             | Status   |
| ------------- | ---------------------------------------------------------------- | -------- |
| `**candle`**  | HuggingFace's minimalist DL framework                            | [stable] |
| `**burn`**    | Pure-Rust DL framework (pluggable backends: WGPU, CUDA, NdArray) | [stable] |
| `tch`         | libtorch (C++) bindings                                          | [stable] |
| `ort`         | ONNX Runtime bindings                                            | [stable] |
| `dfdx`        | Type-safe tensors                                                | [mature] |
| `linfa`       | Classical ML (sklearn-shaped)                                    | [mature] |
| `smartcore`   | Classical ML                                                     | [mature] |
| `rust-bert`   | Transformer models (via tch)                                     | [mature] |
| `tokenizers`  | HF tokenizers (Rust core; Python front)                          | [stable] |
| `hf-hub`      | Model downloading                                                | [mature] |
| `safetensors` | Safe tensor format                                               | [stable] |
| `cubecl`      | GPU kernel DSL (powering burn's GPU backend)                     | [active] |


### Are We Learning Yet — status (2026)

- **Training from scratch:** possible but rare — most teams use candle or
burn for inference, tch for training.
- **Inference:** yes, candle/burn/ort are production.
- **Python parity:** no — PyTorch ecosystem still dominant for training.
- **GPU:** CUDA via candle/burn; cross-vendor via WGPU (burn) or cubecl.

### Canonical inference stack

```toml
candle-core = "0.7"
candle-nn   = "0.7"
candle-transformers = "0.7"
tokenizers = "0.20"
hf-hub     = "0.3"
safetensors = "0.4"
```

---

## 25 Configuration


| Crate                           | Role                                              | Status   | Pick when                         |
| ------------------------------- | ------------------------------------------------- | -------- | --------------------------------- |
| `**figment**`                   | Layered config (file + env + CLI), serde-native   | [stable] | **Canonical.**                    |
| `config`                        | Layered config                                    | [stable] | Legacy / simpler needs.           |
| `envy`                          | `Envy::from_env` deserialize struct from env vars | [stable] | 12-factor apps with envvars only. |
| `dotenvy`                       | `.env` loader (replaces unmaintained `dotenv`)    | [stable] | Dev/test.                         |
| `clap` with `#[arg(env = ...)]` | CLI that also reads env                           | [stable] |                                   |
| `serde_env`                     | Env → serde                                       | [mature] |                                   |
| `twelf`                         | Layered config, focused on 12-factor              | [mature] |                                   |
| `confy`                         | App-dir-aware config (xdg/appdata)                | [mature] | Desktop-app config.               |


### Canonical

```toml
figment = { version = "0.10", features = ["toml", "env", "json"] }
dotenvy = "0.15"
serde   = { version = "1", features = ["derive"] }
```

```rust
let config: AppConfig = Figment::new()
    .merge(Toml::file("App.toml"))
    .merge(Env::prefixed("APP_").split("__"))
    .extract()?;
```

- Rule: load `.env` in dev only; never rely on it in prod.
- Validate with `validator` or custom `TryFrom<RawConfig>`.

---

## 26 Build tools & task runners

Tag: `10-testing-and-tooling`.


| Tool                           | Role                                                                       | Status                                                                |
| ------------------------------ | -------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `**just**`                     | Simple task runner, `justfile`                                             | **[stable]** — **canonical replacement for `make` in Rust projects.** |
| `cargo-make`                   | Task runner as cargo extension                                             | [stable]                                                              |
| `cargo-xtask`                  | Pattern — workspace package for repo-local tasks (no extra binary install) | —                                                                     |
| `cargo-script` / `rust-script` | Run single-file Rust as scripts                                            | [stable]                                                              |
| `cargo-binstall`               | Install prebuilt binaries from crates.io                                   | [stable]                                                              |
| `cargo-watch`                  | Rerun on file change                                                       | [stable]                                                              |
| `bacon`                        | Background watcher (alt to cargo-watch)                                    | [stable]                                                              |
| `cargo-edit`                   | `cargo add/rm/upgrade`                                                     | [stable, now upstream]                                                |
| `cargo-outdated`               | Report outdated deps                                                       | [stable]                                                              |
| `cargo-udeps`                  | Find unused deps (nightly)                                                 | [mature]                                                              |
| `cargo-machete`                | Stable `cargo-udeps` alternative                                           | [stable]                                                              |
| `cargo-deny`                   | License/advisory/duplicate policy                                          | [stable]                                                              |
| `cargo-audit`                  | RUSTSEC advisory check                                                     | [stable]                                                              |
| `cargo-vet`                    | Supply-chain review ledger                                                 | [stable]                                                              |
| `cargo-expand`                 | Dump macro expansion                                                       | [stable]                                                              |
| `cargo-asm`                    | View emitted asm                                                           | [mature]                                                              |
| `cargo-bloat`                  | Binary size by crate                                                       | [stable]                                                              |
| `cargo-llvm-lines`             | Biggest generic instantiations                                             | [stable]                                                              |
| `cargo-chef`                   | Docker-layer-cache friendly dep compile                                    | [stable]                                                              |
| `sccache`                      | Compile cache                                                              | [stable]                                                              |
| `cargo-semver-checks`          | Catch accidental semver breaks                                             | [stable]                                                              |
| `cargo-public-api`             | Print public API diff                                                      | [stable]                                                              |
| `cargo-release`                | Tagged release orchestration                                               | [stable]                                                              |
| `cross`                        | Cross-compile via Docker                                                   | [stable]                                                              |
| `clippy` (`cargo clippy`)      | Lints                                                                      | [stable]                                                              |
| `rustfmt`                      | Formatter                                                                  | [stable]                                                              |
| `mold` / `lld`                 | Faster linker                                                              | [stable]                                                              |


### Canonical `.cargo/config.toml` snippet

```toml
[build]
rustflags = ["-C", "link-arg=-fuse-ld=lld"]     # or "mold" on linux

[alias]
lint  = "clippy --workspace --all-targets -- -D warnings"
b     = "build"
c     = "check"
t     = "nextest run"
ci    = "clippy --workspace --all-targets -- -D warnings"
```

### Canonical `justfile` skeleton

```
default:
    @just --list

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo nextest run --workspace

deny:
    cargo deny check
```

---

## 27 Filesystem & IO


| Crate                  | Role                                            | Status                 |
| ---------------------- | ----------------------------------------------- | ---------------------- |
| `**walkdir**`          | Recursive directory walking (sync)              | [stable]               |
| `**ignore**`           | Gitignore-aware walker (used by ripgrep)        | [stable]               |
| `jwalk`                | Parallel walkdir                                | [stable]               |
| `fs-err`               | `std::fs` wrappers with path-in-error context   | [stable] — **always.** |
| `fs_extra`             | Copy/move directories recursively               | [stable]               |
| `dirs`                 | Platform-appropriate dirs (home, config, cache) | [stable]               |
| `directories`          | Alt to `dirs`                                   | [stable]               |
| `tempfile`             | Cross-platform temp files/dirs                  | [stable]               |
| `notify`               | Filesystem watcher                              | [stable]               |
| `globwalk` / `globset` | Glob patterns                                   | [stable]               |
| `tokio::fs`            | Async filesystem (threadpool-based)             | [stable]               |
| `memmap2`              | mmap                                            | [stable]               |
| `filetime`             | Read/write file mtime/atime                     | [stable]               |
| `same-file`            | Compare file identity                           | [stable]               |
| `which`                | `which`/`where` equivalent for binary lookup    | [stable]               |
| `open`                 | Open URL in default browser                     | [mature]               |


### Canonical

```toml
walkdir  = "2"
ignore   = "0.4"
fs-err   = "3"
tempfile = "3"
dirs     = "5"
notify   = "8"
which    = "7"
```

---

## 28 Concurrency primitives

Tag: `11-ecosystem-crate-picks`. Overlap with § 11 — this section focuses
on sync/channel/atomic primitives.


| Crate                   | Role                                                                              | Status                                    |
| ----------------------- | --------------------------------------------------------------------------------- | ----------------------------------------- |
| `**parking_lot**`       | Faster Mutex/RwLock than std                                                      | [stable]                                  |
| `parking_lot_core`      | Low-level                                                                         | [stable]                                  |
| `**crossbeam**`         | Scoped threads, channels, atomics, epoch GC                                       | [stable]                                  |
| `**crossbeam-channel**` | MPMC                                                                              | [stable]                                  |
| `flume`                 | MPMC channel, async/sync                                                          | [stable]                                  |
| `tokio::sync`           | `Mutex`, `RwLock`, `Semaphore`, `Notify`, `oneshot`, `mpsc`, `broadcast`, `watch` | [stable]                                  |
| `async-channel`         | Runtime-agnostic async channels                                                   | [stable]                                  |
| `async-lock`            | Runtime-agnostic async locks                                                      | [stable]                                  |
| `event-listener`        | Async notify primitive                                                            | [stable]                                  |
| `atomic`                | Type-erased atomics for any `Copy` type                                           | [mature]                                  |
| `atomic_float`          | AtomicF32/F64                                                                     | [stable]                                  |
| `portable-atomic`       | `AtomicU64` on 32-bit targets                                                     | [stable]                                  |
| `atomic-waker`          | `Waker` cell                                                                      | [stable]                                  |
| `**rayon**`             | Data parallel iterator                                                            | [stable]                                  |
| `tokio-rayon`           | Bridge rayon into tokio                                                           | [mature]                                  |
| `async-scoped`          | Scoped async tasks                                                                | [mature]                                  |
| `tokio::task::JoinSet`  | Scoped join set (built-in)                                                        | [stable]                                  |
| `moka`                  | Concurrent cache (LRU, LFU, TTL)                                                  | [stable] — **canonical in-memory cache.** |
| `cached`                | Memoization/macro + TTL                                                           | [mature]                                  |
| `quick-cache`           | Tiny LFU cache                                                                    | [mature]                                  |
| `stretto`               | TinyLFU cache (Rust port of Ristretto)                                            | [mature]                                  |


### Canonical cache

```toml
moka = { version = "0.12", features = ["future"] }
```

---

## 29 Process & subprocess


| Crate                      | Role                                                  | Status                                              |
| -------------------------- | ----------------------------------------------------- | --------------------------------------------------- |
| `std::process::Command`    | Canonical.                                            |                                                     |
| `duct`                     | Higher-level subprocess with pipes                    | [stable]                                            |
| `xshell`                   | Shell-ish scripting in Rust                           | [stable]                                            |
| `tokio::process::Command`  | Async                                                 | [stable]                                            |
| `sysinfo`                  | Cross-platform system info (processes, memory, disks) | [stable]                                            |
| `nix`                      | Unix syscalls                                         | [stable]                                            |
| `rustix`                   | Pure-Rust `libc` replacement — no_std-capable, faster | [stable]                                            |
| `windows` / `windows-sys`  | Canonical Windows API bindings (Microsoft-maintained) | [stable]                                            |
| `winapi`                   | Legacy Windows bindings                               | [maintenance] — migrate to `windows`/`windows-sys`. |
| `ctrlc`                    | Trap SIGINT                                           | [stable]                                            |
| `signal-hook`              | Signal handling                                       | [stable]                                            |
| `daemonize`                | Unix daemonize                                        | [mature]                                            |
| `supervisord`-style crates | `sd-notify`, etc.                                     | [mature]                                            |


---

## 30 Text, strings, Unicode


| Crate                      | Role                                 | Status                         |
| -------------------------- | ------------------------------------ | ------------------------------ |
| `**unicode-segmentation**` | Grapheme/word/sentence iteration     | [stable]                       |
| `unicode-width`            | Display width (CJK etc.)             | [stable]                       |
| `unicode-normalization`    | NFC/NFD/NFKC/NFKD                    | [stable]                       |
| `unicode-ident`            | `is_xid_start`/`is_xid_continue`     | [stable]                       |
| `unicode-bidi`             | Bidi algorithm                       | [stable]                       |
| `icu` (ICU4X)              | Full internationalization            | [stable] — **canonical i18n.** |
| `rust-i18n`                | Simple gettext-ish                   | [mature]                       |
| `fluent` / `fluent-bundle` | Mozilla Fluent                       | [stable]                       |
| `textwrap`                 | Word wrapping                        | [stable]                       |
| `regex`                    | (§ 12)                               |                                |
| `convert_case`             | camelCase↔snake_case etc.            | [stable]                       |
| `heck`                     | Case conversions used by cargo/rustc | [stable]                       |
| `cow-utils`                | `Cow<str>`-friendly helpers          | [mature]                       |


---

## 31 Encoding (base64, hex, etc.)


| Crate                     | Role                   | Status   |
| ------------------------- | ---------------------- | -------- |
| `**base64`**              | Base64                 | [stable] |
| `base32`                  | Base32                 | [stable] |
| `hex`                     | Hex                    | [stable] |
| `data-encoding`           | Configurable encodings | [stable] |
| `percent-encoding`        | URL percent encoding   | [stable] |
| `url`                     | URL parse/build        | [stable] |
| `urlencoding`             | Simple encode/decode   | [stable] |
| `html-escape`             | HTML entity encode     | [stable] |
| `ammonia`                 | HTML sanitizer         | [stable] |
| `charset` / `encoding_rs` | Legacy encodings       | [stable] |
| `punycode`                | IDNA punycode          | [stable] |
| `idna`                    | IDNA                   | [stable] |
| `uuid`                    | (§ 14)                 |          |


---

## 32 Templating


| Crate        | Paradigm                                 | Status        | Pick when                                          |
| ------------ | ---------------------------------------- | ------------- | -------------------------------------------------- |
| `**askama`** | Jinja-like, type-checked at compile time | [stable]      | **Canonical.**                                     |
| `minijinja`  | Runtime Jinja, rendering-time context    | [stable]      | Dynamic templates / user-editable.                 |
| `tera`       | Jinja-like, runtime                      | [stable]      |                                                    |
| `handlebars` | Handlebars, runtime                      | [stable]      |                                                    |
| `liquid`     | Liquid template                          | [mature]      |                                                    |
| `maud`       | HTML macro (Rust-native)                 | [stable]      | Tiny HTML without template files.                  |
| `horrorshow` | HTML macro                               | [maintenance] |                                                    |
| `rstml`      | JSX-ish HTML macro                       | [active]      | Leptos/Dioxus-adjacent.                            |
| `rinja`      | Fork of askama                           | [active]      | Modern askama replacement when askama is inactive. |


---

## 33 Email


| Crate          | Role                          | Status   |
| -------------- | ----------------------------- | -------- |
| `**lettre`**   | SMTP client + message builder | [stable] |
| `mail-parser`  | MIME parser                   | [stable] |
| `mail-builder` | MIME builder                  | [stable] |
| `imap`         | IMAP client                   | [mature] |
| `async-imap`   | Async IMAP                    | [mature] |


```toml
lettre = { version = "0.11", features = ["tokio1-rustls-tls", "smtp-transport", "builder"] }
```

---

## 34 FFI / interop


| Crate                 | Role                                            | Status                                |
| --------------------- | ----------------------------------------------- | ------------------------------------- |
| `**pyo3**`            | Python bindings                                 | [stable] — **canonical Rust↔Python.** |
| `maturin`             | Build Python wheels from Cargo                  | [stable]                              |
| `rustpython`          | Python interpreter in Rust                      | [mature]                              |
| `**napi-rs`**         | Node.js native addons                           | [stable]                              |
| `neon`                | Node.js bindings (alt)                          | [mature]                              |
| `magnus`              | Ruby bindings                                   | [mature]                              |
| `rutie`               | Ruby bindings                                   | [mature]                              |
| `jni`                 | JVM bindings                                    | [stable]                              |
| `**cxx`**             | Safe C++ interop (Google Chromium)              | [stable] — **canonical Rust↔C++.**    |
| `bindgen`             | Generate Rust from C headers                    | [stable]                              |
| `cbindgen`            | Generate C headers from Rust                    | [stable]                              |
| `uniffi`              | Multi-language bindings (Kotlin, Swift, Python) | [stable]                              |
| `flutter_rust_bridge` | Dart/Flutter bindings                           | [stable]                              |
| `swift-bridge`        | Swift bindings                                  | [mature]                              |
| `csbindgen`           | C# bindings                                     | [mature]                              |
| `deno_core`           | Embed Deno's V8                                 | [mature]                              |
| `wasmer` / `wasmtime` | Embed WASM runtimes (§ 22)                      |                                       |


---

## 35 Observability: metrics


| Crate                         | Role                                      | Status   |
| ----------------------------- | ----------------------------------------- | -------- |
| `**metrics`**                 | Façade crate (like `tracing` for metrics) | [stable] |
| `metrics-exporter-prometheus` | Prometheus exporter                       | [stable] |
| `metrics-util`                | Utilities                                 | [stable] |
| `prometheus`                  | Direct Prometheus client (older)          | [stable] |
| `prometheus-client`           | Official Rust prometheus-client           | [stable] |
| `opentelemetry`               | OTel SDK                                  | [active] |
| `opentelemetry-otlp`          | OTLP exporter                             | [active] |
| `tracing-opentelemetry`       | Tracing → OTel                            | [active] |
| `sentry`                      | Sentry SDK                                | [stable] |
| `sentry-tracing`              | Tracing integration                       | [stable] |


---

## 36 Modern Rust feature status (Are We X Yet)

Tag: `12-modern-rust`. Snapshot of what's actually usable on stable Rust
1.95 (2026-04).


| Feature                                                  | Stable since                               | Library-friendly?               | Notes                                                                   |
| -------------------------------------------------------- | ------------------------------------------ | ------------------------------- | ----------------------------------------------------------------------- |
| `async fn` in inherent impls & free fns                  | 1.39                                       | yes                             |                                                                         |
| `async fn` in traits (AFIT)                              | 1.75                                       | yes (static dispatch)           | Use `trait-variant` for Send bounds.                                    |
| RPITIT (`-> impl Future` in traits)                      | 1.75                                       | yes                             |                                                                         |
| `async fn` in `dyn Trait`                                | not yet                                    | workaround: `async-trait` crate |                                                                         |
| async closures (`async                                   |                                            | {}`)                            | 1.85                                                                    |
| `AsyncFn`/`AsyncFnOnce`/`AsyncFnMut`                     | 1.85                                       | yes                             |                                                                         |
| GATs                                                     | 1.65                                       | yes                             | foundational for many traits (e.g., `LendingIterator`).                 |
| const generics MVP                                       | 1.51                                       | yes                             | integer-only.                                                           |
| `const_generics_defaults`                                | 1.59                                       | yes                             |                                                                         |
| `generic_const_exprs`                                    | unstable                                   | no                              |                                                                         |
| `let…else`                                               | 1.65                                       | yes                             |                                                                         |
| `if let` chains                                          | 1.87                                       | yes                             |                                                                         |
| `let` chains (`let` in `if`/`while`)                     | 1.87                                       | yes                             |                                                                         |
| TAIT (`type Foo = impl Trait;`)                          | partial (RPITIT 1.75; free TAIT unstable)  | no                              | Use associated types for now.                                           |
| `impl Trait` in associated types                         | 1.75 (via RPITIT)                          | yes                             |                                                                         |
| specialization                                           | unstable (min_specialization nightly-only) | no                              |                                                                         |
| `!` (never type)                                         | still unstable as type                     | no                              | `Infallible` as substitute.                                             |
| `try_trait_v2` (custom `?`)                              | nightly                                    | no                              |                                                                         |
| `std::sync::LazyLock` / `OnceLock`                       | 1.80                                       | yes                             | **drop `once_cell`/`lazy_static` for new code.**                        |
| `std::sync::Exclusive`                                   | 1.85                                       | yes                             |                                                                         |
| `std::io::Read::read_buf`                                | stable                                     | yes                             |                                                                         |
| `std::hint::assert_unchecked`                            | 1.81                                       | yes                             |                                                                         |
| `thread::available_parallelism`                          | 1.59                                       | yes                             | **drop `num_cpus`.**                                                    |
| `std::error::Error::provide` (error downcasting by type) | 1.65                                       | yes                             | Used by `error-stack`, `miette`.                                        |
| `std::backtrace`                                         | 1.65                                       | yes                             |                                                                         |
| stable `std::net::Ipv*` in const                         | 1.75                                       | yes                             |                                                                         |
| `std::path::Path::normalize_lexically`                   | nightly                                    | no                              | Use `path-clean` crate.                                                 |
| `std::sync::mpmc` (crossbeam-channel in std)             | nightly                                    | no                              | Use `crossbeam-channel`.                                                |
| `edition = 2024`                                         | rustc 1.85+                                | yes                             | Matches lifetime capture changes, unsafe attributes, gen blocks opt-in. |
| `gen {}` blocks                                          | nightly                                    | no                              | Use `async-stream` / `genawaiter`.                                      |
| `fn() -> impl Trait` capture rules (2024)                | edition 2024                               | yes                             | Fixes long-standing `impl Trait` lifetime surprises.                    |


### Community idiom pulse (TWiR-style)

- **Clippy 1.95** now defaults `needless_lifetimes` and
`uninlined_format_args` to warn.
- `**format!` with inline args** (`format!("x={x}")`) is the universal
style; fmt-args with `, x` are code-smell.
- `**let..else`** replaces 90% of early-return `match`.
- `**impl Into<Cow<'static, str>>`** is preferred over
`impl Into<String>` + `&'static str` overloads.
- `**#[non_exhaustive]**` on all public enums and structs in libs.
- `**#[must_use]**` on builders and `Result`-returning constructors.
- **Scoped threads:** `std::thread::scope`, no need for crossbeam unless
channels are also used.
- **Collection literals (`vec!`):** still preferred over `[_; _]` for Vec.
- **Slice patterns (`[first, .., last]`):** use freely, stable since 1.42.

---

## 37 Testing & cargo tooling (collected)

Tag: `10-testing-and-tooling`.

Canonical CI pipeline for a 2026 Rust crate:

```yaml
# .github/workflows/ci.yml (schematic)
- cargo fmt --all -- --check
- cargo clippy --workspace --all-targets --all-features -- -D warnings
- cargo nextest run --workspace --all-features
- cargo nextest run --workspace --no-default-features
- cargo doc --no-deps --workspace --all-features
- cargo deny check        # advisories + licenses + bans
- cargo semver-checks check-release   # libs only
- cargo llvm-cov nextest --workspace --lcov --output-path lcov.info
- cargo mutants --no-shuffle --in-place --test-tool nextest  # periodic, not every PR
```

Recommended dev-only installs:

```
cargo install --locked cargo-nextest cargo-insta cargo-mutants cargo-fuzz \
                       cargo-llvm-cov cargo-deny cargo-audit cargo-machete \
                       cargo-semver-checks cargo-public-api cargo-edit \
                       cargo-watch bacon cargo-expand cargo-bloat \
                       cargo-llvm-lines cargo-binstall just
```

---

## 38 Canonical reference books

Tag: `01-meta-principles`. Meta-pointers (not crates) from the Little Book
of Rust Books and the Rust community.


| Book                                         | Audience              | Why                                  |
| -------------------------------------------- | --------------------- | ------------------------------------ |
| The Rust Programming Language ("the book")   | beginner              | official canon                       |
| Rust by Example                              | beginner/intermediate | runnable examples                    |
| Rustonomicon                                 | intermediate/advanced | `unsafe`, layout, variance           |
| Rust Reference                               | advanced              | language semantics                   |
| Cargo Book                                   | all                   | Cargo features, workspaces, profiles |
| Rustdoc Book                                 | intermediate          | doc authoring                        |
| Edition Guide                                | all                   | migration per edition                |
| Style Guide (official)                       | all                   | `rustfmt` semantics                  |
| Unstable Book                                | nightly users         | feature gates                        |
| Rust Cookbook                                | intermediate          | recipe-style                         |
| Rust API Guidelines                          | library authors       | **essential for library design**     |
| Little Book of Rust Macros                   | intermediate          | `macro_rules!`                       |
| The Little Book of Rust Books (Lborb)        | all                   | index-of-indexes                     |
| Async Book                                   | intermediate          | foundations (pre-AFIT era)           |
| Tokio Tutorial                               | intermediate          | realistic async patterns             |
| Embedded Rust Book                           | intermediate          | `no_std`, peripherals                |
| Discovery Book (embedded)                    | beginner-embedded     | STM32F3 step-by-step                 |
| Embedonomicon                                | advanced-embedded     | write an HAL                         |
| Rust on ESP Book                             | embedded              | ESP32 specifics                      |
| Rust WASM book                               | intermediate          | wasm-bindgen/wasm-pack               |
| Rust and WebAssembly (game of life tutorial) | beginner              | canonical WASM tutorial              |
| Secure Rust Guidelines (ANSSI)               | advanced              | audit-grade coding rules             |


---

## 39 Quick decision matrix

One-line picks for the 30-second answer. Read top-to-bottom to build a
default `Cargo.toml`.


| Need                       | Canonical 2026 pick                                             |
| -------------------------- | --------------------------------------------------------------- |
| Async runtime (app)        | `tokio` full                                                    |
| Async runtime (embedded)   | `embassy`                                                       |
| HTTP client                | `reqwest` rustls + json + gzip                                  |
| HTTP server                | `axum` + `tower` + `tower-http`                                 |
| gRPC                       | `tonic`                                                         |
| Serialization base         | `serde` + `serde_json`                                          |
| Config file                | `toml`                                                          |
| Rust-to-Rust wire          | `bincode` 2 (std) / `postcard` (no_std)                         |
| Zero-copy archive          | `rkyv`                                                          |
| Error (lib)                | `thiserror`                                                     |
| Error (bin)                | `anyhow`                                                        |
| User-facing diagnostic     | `miette`                                                        |
| Logging/tracing            | `tracing` + `tracing-subscriber`                                |
| CLI parser                 | `clap` derive                                                   |
| Progress bar               | `indicatif`                                                     |
| Prompt                     | `dialoguer`                                                     |
| TUI framework              | `ratatui`                                                       |
| SQL (async)                | `sqlx`                                                          |
| SQL (sync/ORM)             | `diesel`                                                        |
| SQLite (app-local)         | `rusqlite`                                                      |
| Embedded KV (pure Rust)    | `redb`                                                          |
| Redis                      | `redis` crate (or `fred`)                                       |
| Test runner                | `cargo-nextest`                                                 |
| Property test              | `proptest`                                                      |
| Snapshot test              | `insta`                                                         |
| Mock                       | `mockall`                                                       |
| HTTP mock                  | `wiremock`                                                      |
| CLI bin test               | `assert_cmd` + `predicates`                                     |
| Fuzz                       | `cargo-fuzz` + `arbitrary`                                      |
| Benchmark                  | `criterion`                                                     |
| Coverage                   | `cargo-llvm-cov`                                                |
| Mutation                   | `cargo-mutants`                                                 |
| Parallel iteration         | `rayon`                                                         |
| Concurrent map             | `dashmap`                                                       |
| Cache                      | `moka`                                                          |
| Mutex                      | `parking_lot::Mutex`                                            |
| Regex                      | `regex`                                                         |
| Combinator parser          | `winnow`                                                        |
| Lexer                      | `logos`                                                         |
| Date/time (new)            | `jiff`                                                          |
| Date/time (legacy)         | `chrono`                                                        |
| UUID                       | `uuid` v7                                                       |
| Random                     | `rand` + `getrandom`                                            |
| TLS                        | `rustls`                                                        |
| Password hash              | `argon2`                                                        |
| Fast hash                  | `blake3`                                                        |
| AEAD                       | `aes-gcm` or `chacha20poly1305`                                 |
| Signatures                 | `ed25519-dalek`                                                 |
| Compression                | `flate2` / `zstd` / `brotli`                                    |
| Image                      | `image`                                                         |
| SVG                        | `resvg`                                                         |
| PDF                        | `printpdf` + `lopdf`                                            |
| Audio                      | `cpal` (I/O) + `symphonia` (decode) + `rodio`/`kira` (playback) |
| GUI (dev tool)             | `egui`                                                          |
| GUI (Electron replacement) | `tauri`                                                         |
| GUI (native)               | `iced`                                                          |
| Game engine                | `bevy`                                                          |
| Physics                    | `rapier`                                                        |
| Embedded HAL               | `embedded-hal` 1.0                                              |
| Embedded async             | `embassy`                                                       |
| WASM bindings              | `wasm-bindgen` + `web-sys`                                      |
| WASM frontend              | `leptos` or `dioxus`                                            |
| WASM runtime (host)        | `wasmtime`                                                      |
| Derive helpers             | `derive_more`, `strum`, `thiserror`                             |
| Bitflags                   | `bitflags`                                                      |
| Pin projection             | `pin-project-lite`                                              |
| Linear algebra (sci)       | `nalgebra` / `ndarray`                                          |
| Linear algebra (game)      | `glam`                                                          |
| DataFrame                  | `polars`                                                        |
| DL framework               | `candle` or `burn`                                              |
| Classical ML               | `linfa`                                                         |
| Tokenizers                 | `tokenizers`                                                    |
| Config loader              | `figment`                                                       |
| `.env`                     | `dotenvy`                                                       |
| Task runner                | `just`                                                          |
| Watcher                    | `bacon`                                                         |
| Supply-chain check         | `cargo-deny`                                                    |
| Semver check               | `cargo-semver-checks`                                           |
| Walk FS                    | `walkdir` or `ignore`                                           |
| Error-contextful fs        | `fs-err`                                                        |
| Temp files                 | `tempfile`                                                      |
| FS watch                   | `notify`                                                        |
| Unicode segmentation       | `unicode-segmentation`                                          |
| i18n                       | `icu` (ICU4X)                                                   |
| Template                   | `askama`                                                        |
| Email                      | `lettre`                                                        |
| Python binding             | `pyo3` + `maturin`                                              |
| Node binding               | `napi-rs`                                                       |
| C++ binding                | `cxx`                                                           |
| Multi-lang binding         | `uniffi`                                                        |
| Windows API                | `windows`/`windows-sys`                                         |
| Unix syscalls              | `rustix` (or `nix`)                                             |
| Metrics                    | `metrics` + `metrics-exporter-prometheus`                       |
| OTel                       | `opentelemetry` + `opentelemetry-otlp`                          |
| Error reporting SaaS       | `sentry`                                                        |


---

## Appendix A — Crate name aliases / things that confuse

- `once_cell` vs `std::sync::{OnceLock, LazyLock}` — **use std since 1.80.**
- `lazy_static!` — **deprecated.** Use `LazyLock`.
- `rand` 0.9 — API renamed (`thread_rng` still works, prefer `rand::rng()`).
- `rand` 0.8 APIs still widespread in deps; both live side-by-side.
- `chrono` vs `time` vs `jiff` — **jiff > chrono > time** for new code if
you don't need no_std.
- `serde_yaml` — **unmaintained.** Use `serde_yml`.
- `dotenv` (without the `y`) — unmaintained. Use `dotenvy`.
- `structopt` — predecessor of clap-derive; **never** for new code.
- `winapi` — legacy; use `windows-sys` or `windows`.
- `failure` — predecessor of thiserror/anyhow; **retired.**
- `error-chain` — same; retired.
- `rocket_contrib` — rolled into `rocket` proper in 0.5.
- `actix` (core) vs `actix-web` — you almost always want `actix-web`.
- `warp` — maintainers moved to `axum`.
- `async-std` vs `tokio` — tokio won. async-std is in maintenance.
- `hyper` 0.14 vs 1.0 — **1.0 is the current line**; 0.14 compat reqs a
`hyper-util` glue. Most users should use `reqwest`/`axum` and never touch
hyper directly.

## Appendix B — "When two crates look identical"


| You see                          | Pick                                          | Because                                                           |
| -------------------------------- | --------------------------------------------- | ----------------------------------------------------------------- |
| smallvec vs tinyvec              | smallvec                                      | wider adoption; tinyvec only if `#![forbid(unsafe_code)]` matters |
| once_cell vs std::sync::LazyLock | std                                           | stable since 1.80                                                 |
| chrono vs time                   | chrono for legacy, jiff for new               | jiff is the 2026 trend                                            |
| thiserror vs snafu               | thiserror                                     | ubiquity, lighter                                                 |
| anyhow vs eyre                   | anyhow                                        | ubiquity; eyre only if you want custom reports                    |
| reqwest vs hyper (client)        | reqwest                                       | batteries included                                                |
| axum vs actix-web                | axum                                          | tokio/tower alignment, simpler state                              |
| sqlx vs diesel                   | sqlx                                          | async; diesel if you want compile-time schema safety *and* sync   |
| bevy vs fyrox                    | bevy                                          | ecosystem momentum                                                |
| candle vs burn                   | candle                                        | simpler, HF-aligned; burn for multi-backend GPU                   |
| nalgebra vs glam                 | glam for games/graphics, nalgebra for science |                                                                   |
| rand 0.8 vs 0.9                  | 0.9 for new code                              |                                                                   |
| polars vs datafusion             | polars for interactive, datafusion for SQL    |                                                                   |
| leptos vs yew                    | leptos                                        | fine-grained reactivity, SSR-first                                |
| tauri vs iced                    | tauri if UI is web; iced for pure native      |                                                                   |


## Appendix C — Domain-specific starter Cargo.tomls

### Axum web service

```toml
[dependencies]
tokio      = { version = "1", features = ["full"] }
axum       = { version = "0.8", features = ["macros", "multipart", "ws"] }
tower      = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "compression-full", "timeout", "request-id"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx       = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "macros", "uuid", "chrono"] }
uuid       = { version = "1", features = ["v7", "serde"] }
chrono     = { version = "0.4", features = ["serde"] }
thiserror  = "2"
anyhow     = "1"
tracing    = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
figment    = { version = "0.10", features = ["toml", "env"] }
reqwest    = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip"] }
argon2     = "0.5"
jsonwebtoken = "9"
```

### CLI tool

```toml
[dependencies]
clap       = { version = "4", features = ["derive", "env", "wrap_help"] }
anyhow     = "1"
tracing    = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
indicatif  = "0.17"
dialoguer  = "0.11"
owo-colors = "4"
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
toml       = "0.8"
dirs       = "5"
tempfile   = "3"
walkdir    = "2"
ignore     = "0.4"
fs-err     = "3"
reqwest    = { version = "0.12", default-features = false, features = ["rustls-tls", "blocking", "json"] }
```

### Bevy game

```toml
[dependencies]
bevy       = { version = "0.14", features = ["wayland"] }
rapier3d   = { version = "0.22", features = ["simd-stable"] }
bevy_rapier3d = "0.28"
serde      = { version = "1", features = ["derive"] }
ron        = "0.8"
rand       = "0.9"

[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 3
```

### Embassy on STM32

```toml
[dependencies]
embassy-executor = { version = "0.6", features = ["arch-cortex-m", "executor-thread", "integrated-timers"] }
embassy-time     = { version = "0.3", features = ["tick-hz-32_768"] }
embassy-stm32    = { version = "0.2", features = ["stm32h743zi", "time-driver-any", "exti", "unstable-pac"] }
embassy-sync     = "0.6"
embassy-futures  = "0.1"
cortex-m         = "0.7"
cortex-m-rt      = "0.7"
defmt            = "0.3"
defmt-rtt        = "0.4"
panic-probe      = { version = "0.3", features = ["print-defmt"] }
heapless         = "0.8"
postcard         = "1"

[profile.release]
lto = true
codegen-units = 1
opt-level = "z"
panic = "abort"
```

### Leptos SSR app

```toml
[dependencies]
leptos         = { version = "0.7", features = ["nightly"] }
leptos_meta    = { version = "0.7", features = ["nightly"] }
leptos_router  = { version = "0.7", features = ["nightly"] }
leptos_axum    = { version = "0.7", optional = true }
server_fn      = { version = "0.7", features = ["serde-lite"] }
axum           = { version = "0.8", optional = true }
tokio          = { version = "1", features = ["full"], optional = true }
wasm-bindgen   = "=0.2.99"
console_error_panic_hook = "0.1"

[features]
csr = ["leptos/csr"]
hydrate = ["leptos/hydrate"]
ssr = ["dep:axum", "dep:tokio", "dep:leptos_axum", "leptos/ssr", "leptos_meta/ssr", "leptos_router/ssr"]
```

## Appendix D — Anti-patterns checklist for LLMs

- Do **not** suggest `lazy_static!` in new code; use `LazyLock`.
- Do **not** suggest `once_cell::sync::Lazy` when targeting Rust 1.80+.
- Do **not** suggest `num_cpus::get()`; use `std::thread::available_parallelism()`.
- Do **not** suggest `failure` / `error-chain`; use `thiserror` + `anyhow`.
- Do **not** suggest `dotenv`; use `dotenvy`.
- Do **not** suggest `serde_yaml`; use `serde_yml`.
- Do **not** suggest `structopt`; use `clap` v4 derive.
- Do **not** suggest `rocket_contrib`; Rocket 0.5 rolled it in.
- Do **not** suggest `winapi`; use `windows`/`windows-sys`.
- Do **not** suggest `actix` (the actor core); use `actix-web` for the web framework.
- Do **not** suggest `async-std` for new code; use `tokio`.
- Do **not** suggest `slog` for new code; use `tracing`.
- Do **not** suggest `env_logger` for production; use `tracing-subscriber`.
- Do **not** suggest `native-tls` when `rustls` will do.
- Do **not** suggest `OpenSSL` unless interop literally requires it.
- Do **not** suggest `#[async_trait]` when targeting Rust 1.75+ unless dyn-safety is required.
- Do **not** hand-roll crypto; use RustCrypto or rustls.
- Do **not** suggest `Box<dyn Error>` in bin code; use `anyhow::Result`.
- Do **not** return `anyhow::Error` from a library's public API; use `thiserror`.
- Do **not** block inside `async fn`; use `tokio::task::spawn_blocking`.
- Do **not** pick `serde_json::Value` when a typed `Deserialize` struct works.
- Do **not** pull tokio into a library if you can stay runtime-agnostic with `futures`.

## Appendix E — MSRV expectations (canonical crates, April 2026)


| Crate                  | Typical MSRV policy                  |
| ---------------------- | ------------------------------------ |
| `tokio`                | stable - 6 (currently 1.76)          |
| `serde`                | stable - 6                           |
| `reqwest`              | stable - 6                           |
| `axum`                 | stable - 4                           |
| `clap` 4               | stable - 6                           |
| `sqlx`                 | stable - 6                           |
| `bevy`                 | edition 2021 + latest-stable-minus-1 |
| `tracing`              | stable - 6                           |
| `thiserror` / `anyhow` | stable - 8 (very conservative)       |
| `hyper` 1              | stable - 6                           |
| `rustls`               | stable - 6                           |


Rule of thumb: pin your crate's `rust-version` to `stable − 4` if you care
about enterprise users; pin to `latest stable` if you want to use 2024
edition conveniences and recent `std` APIs like `LazyLock`.

## Appendix F — Supplementary decision tables (extra push)

Tag: `11-ecosystem-crate-picks` + `10-testing-and-tooling`. Dense lookups that
did not fit earlier sections — **Embassy vs RTIC**, **async traits in public
APIs**, **supply-chain commands**, **frameworks Salvo/Poem/Rocket** side-by-side.

### Embedded: Embassy vs RTIC (`cortex-m-rtic` / `rtic`)


| Criterion      | **Embassy**                                              | **RTIC**                                                |
| -------------- | -------------------------------------------------------- | ------------------------------------------------------- |
| Model          | Async `await`, cooperative tasks                         | Static priority + lock-free resources, interrupt-driven |
| Best when      | Radio/WiFi stacks, long I/O, many drivers with async HAL | Hard real-time ISRs, minimal deps, predictable latency  |
| Ecosystem      | Large async HAL (nRF, STM32, RP2040, …)                  | Mature; excellent for “classic” embedded control        |
| Learning curve | Async + embassy-executor concepts                        | Resources / tasks / priorities model                    |


**Heuristic:** new project with **async-first vendor HAL** → start **Embassy**.
Bare-metal control loop with **strict ISR timing** and no async → **RTIC**.
Both coexist in the community; do not mix executors in one firmware without a
clear boundary.

### Async traits in library APIs (Rust 1.75+)


| Situation                                          | Recommendation                                                        |
| -------------------------------------------------- | --------------------------------------------------------------------- |
| Static dispatch only (`impl MyTrait for Concrete`) | Native `async fn` in trait (AFIT) — **no** `async-trait`              |
| `dyn Trait` + `Send`, object-safe                  | `async_trait::async_trait` **or** hand-written `Pin<Box<dyn Future>>` |
| Need `Send` and non-`Send` variants of same trait  | `**trait-variant`** (community pattern for libraries)                 |
| MSRV below 1.75                                    | `async-trait`                                                         |


### Web frameworks — when Salvo / Poem / Rocket over Axum


| Framework     | Reach for it when                                       | Tradeoff vs Axum                              |
| ------------- | ------------------------------------------------------- | --------------------------------------------- |
| **Axum**      | Default (see §04)                                       | —                                             |
| **Salvo**     | Built-in extras: OpenAPI, ACME, static, WS in one crate | Smaller ecosystem examples than Axum          |
| **Poem**      | OpenAPI-first, middleware composition similar to Tower  | Less “default stack” blog content than Axum   |
| **Rocket**    | Max ergonomics, proc-macro routes, fairings             | Compile-time heaviness; check release cadence |
| **Actix-web** | Throughput legend, actor patterns                       | Different middleware model than Tower         |


### Supply-chain & policy tooling (blessed.rs “Tooling” alignment)


| Tool                            | Role                                                               |
| ------------------------------- | ------------------------------------------------------------------ |
| `cargo audit`                   | Advisory DB (RustSec) for known vulns                              |
| `cargo deny`                    | Licenses + bans + advisories + duplicates (Embark-style policy)    |
| `cargo-semver-checks`           | Lint breaking API changes before publish                           |
| `cargo-machete`                 | Find unused deps (CI hygiene)                                      |
| `cargo-nextest`                 | Parallel, flaky-aware test runner (IDE support in RustRover 2026+) |
| `cargo-mutants`                 | Mutation testing — finds weak tests                                |
| `cargo-fuzz` + `libfuzzer-sys`  | Coverage-guided fuzzing                                            |
| `release-plz` / `cargo-release` | Version bump + changelog + publish automation                      |


### crates.io ↔ lib.rs ↔ docs.rs (discovery chain)

1. **Discover** candidates: lib.rs search or `cargo search`.
2. **Validate**: crates.io page → repository, `rust-version`, license.
3. **API surface**: docs.rs (default target only since 2026 docs.rs policy — see TWiR 646).
4. **Policy**: run `cargo deny` / `cargo audit` before locking versions in `Cargo.lock`.

---

*End of catalog.*