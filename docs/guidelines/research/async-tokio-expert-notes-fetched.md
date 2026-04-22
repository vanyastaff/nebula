# Rust async/await & Tokio — expert KB notes (fetched sources)

Dense extraction for LLM consumption. Primary sources: [Rust Async Book](https://rust-lang.github.io/async-book/), [Tokio Tutorial](https://tokio.rs/tokio/tutorial), [Tokio Topics](https://tokio.rs/tokio/topics), [docs.rs/tokio](https://docs.rs/tokio/latest/tokio/) (runtime, task). Fetched April 2026.

*(RU) Есть §7.11–7.13, §13 (снапшот экосистемы: не только Tokio), и блок «Кратко по-русски».*

---

## Taxonomy map


| Tag                          | This file                                                                                     |
| ---------------------------- | --------------------------------------------------------------------------------------------- |
| **03-idioms**                | §3                                                                                            |
| **04-design-patterns**       | §4                                                                                            |
| **05-anti-patterns**         | §5                                                                                            |
| **07-async-concurrency**     | §7 (core), §7.11–7.14 (LocalSet, blocking APIs, join fairness, sync primitives)               |
| **08-unsafe-and-ffi**        | §8                                                                                            |
| **09-performance**           | §9                                                                                            |
| **11-ecosystem-crate-picks** | §11 + §13.0 catalog                                                                           |
| **12-modern-rust**           | §12                                                                                           |
| **13-ecosystem-snapshot**    | §13 — §13.0 catalog (tokio, async-std, actix, thin_main_loop, async-task), then version notes |


---

## 03-idioms — async idioms

- `**?` through futures**: `?` inside an `async` block/async fn propagates `Result`/`Option` out of that async value; combine with `select!` carefully — `?` in a *branch handler* propagates out of the whole `select!` (Tokio tutorial).
- `**.await` at call site**: Calling `async fn` does **not** run the body until `.await` (or `block_on` / executor polling). Example: `say_world()` then `println!("hello")` then `op.await` prints `hello` then `world` (Tokio hello-tokio).
- **Lazy futures**: A Rust future is the computation object, not a background job; the executor **polls** it. No progress without polling (Async in depth, Tokio).
- **Combinators**: Prefer `join!` for concurrent completion of multiple futures; `select!` for first completion; stream adapters (`map`, `filter`, `take`, `filter_map`) order matters (Tokio streams page).

---

## 04-design-patterns — actors, tasks, channels, select, cancellation

- **Actor / dedicated task**: For shared `Client` with `&mut self`, spawn one task owning the connection; others send commands via `mpsc`; responses via `oneshot` (Tokio channels chapter). Matches Tokio’s “spawn a task to manage state and use message passing.”
- **Bounded queues**: Use bounded `mpsc`; unbounded memory is a failure mode. Async laziness avoids implicit unbounded queuing unless you spawn without awaiting (Tokio channels).
- `**select!` loop — multiplexer**: Merge multiple `recv()` branches; `else => break` when all closed; random branch ordering avoids starvation of later channels (Tokio select).
- **Long-lived future in `select!*`*: Create the future **outside** the loop, `tokio::pin!(operation)`, then `select! { _ = &mut operation => ... }` so the same in-flight op is polled across iterations (Tokio select).
- **Branch preconditions**: `res = &mut operation, if !done =>` disables a branch; needed to avoid polling a completed future (panic: async fn resumed after completion) (Tokio select).
- **Graceful shutdown (Topics: shutdown)**:
  1. Detect shutdown (e.g. `tokio::signal::ctrl_c`, or internal trigger).
  2. Signal all subsystems (`CancellationToken::cancel()` after cloning tokens).
  3. Wait for completion (`tokio_util::task::TaskTracker::wait().await` after `close()`).
- `**CancellationToken`**: Clones are indistinguishable; cancel one cancels all; `select! { _ = token.cancelled() => ... }` allows shutdown procedure (flush, etc.) before exit (Tokio topics/shutdown).

---

## 05-anti-patterns

- **Blocking in async**: `println!` is blocking I/O; avoid in hot async paths (Async book intro). Use `spawn_blocking` for blocking syscalls/compute (docs.rs `spawn_blocking`).
- `**std::sync::MutexGuard` across `.await`**: Guard is not `Send` → `tokio::spawn` future not `Send`. Fix: **scope** so guard drops before `.await`; plain `drop(lock)` is insufficient — compiler uses **scope analysis**, not dataflow (Tokio shared-state). Some crates mark guards `Send` → **still deadlock risk** if another task blocks on same mutex on same thread while first task holds lock across await.
- **Unconditional `tokio::sync::Mutex`**: More expensive; uses sync mutex internally; don’t default to it—prefer short `std::sync::Mutex` critical sections **not** spanning `.await` (Tokio shared-state).
- **Spawning without joining**: `JoinHandle` errors on panic or runtime shutdown; fire-and-forget loses backpressure—pair with bounded channels / trackers for shutdown.
- **Fat futures / large stack buffers**: Task state is one allocation; size ≥ largest cross-`.await` state. Prefer **heap** `Vec` for large buffers in spawned connection loops, not huge stack arrays (Tokio I/O echo server).
- **EOF on read**: `Ok(0)` must exit read loop; otherwise tight spin + CPU burn (Tokio I/O).
- `**select!` / drop = cancel**: Dropping losing branches cancels them; code must be **cancellation-safe** if it observes partial effects (Tokio select + Async book cancellation stub).
- `**spawn_blocking` misuse**: Not for infinite loops; blocks shutdown once running; `abort` ineffective after start; cap parallel CPU work with `Semaphore` or rayon (docs.rs `spawn_blocking`).

---

## 07-async-concurrency — core machinery

### 7.1 `Future`, `Poll`, `Context`, `Waker`

- **Trait (std)**: `fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>`.
- **Semantics**: `Poll::Ready` completes; `Poll::Pending` means register for wake. **Contract**: if returning `Pending`, arrange for `cx.waker()` to be woken when progress is possible; forgetting → hang (Tokio Async in depth, Async book wakeups).
- **Waker migration**: A future may move between tasks; **each `poll` may pass a different `Waker`** — update stored waker (compare with `will_wake`) (Tokio Async in depth, Delay fix).
- **Composition**: Outer futures call inner `poll`; no implicit parallelism—concurrency comes from `spawn`, `select!`, `join!`, etc.

### 7.2 Executors and mini-tokio mental model

- Executor repeatedly polls top-level task until pending; waker queues task for retry (Tokio Async in depth).
- Busy-loop polling without sleeping is wrong; real runtimes integrate I/O and timers.

### 7.3 `Send` and `tokio::spawn`

- `**'static`**: Spawned closure must own its data (use `async move`); `Arc` for shared ownership (Tokio spawning).
- `**Send`**: Types held **across** `.await` must be `Send` for `tokio::spawn` because the runtime may move the task between threads at await points. Example: `Rc` held across `yield_now().await` fails (Tokio spawning).

### 7.4 `std::sync::Mutex` vs `tokio::sync::Mutex` (shared state)

- **Rule of thumb**: `std::sync::Mutex` OK inside async when **contention low** and lock **not held across `.await`**.
- **High contention** on blocking mutex: blocks **worker thread** → blocks all tasks on that thread (Tokio shared-state).
- `**current_thread` runtime**: Mutex never contended across threads (single worker) — useful for sync API bridges (Tokio shared-state, docs.rs runtime bridging).
- **Sharding**: e.g. `Vec<Mutex<HashMap<...>>>` + hash shard reduces contention (Tokio shared-state).

### 7.5 Channels (Tokio)


| Primitive     | Shape             | Use                                                                  |
| ------------- | ----------------- | -------------------------------------------------------------------- |
| **mpsc**      | MPSC, bounded     | Work queues, actor inbox; `Sender::clone` for many producers         |
| **oneshot**   | SPSC, one value   | Single RPC-style reply; `send` is sync; receiver drop signals cancel |
| **broadcast** | MPMC, many values | All subscribers see each message                                     |
| **watch**     | MPMC, latest only | Config, readiness flags, no history                                  |


- `**std::sync::mpsc` / crossbeam** in async: block the thread — wrong for async **waiting** (use async channels) (Tokio channels).
- **MPMC “job queue”**: `async-channel` if each message to one consumer only (Tokio channels).

### 7.6 `select!` vs `spawn` (Tokio)

- `**spawn`**: independent tasks; **no borrowing** across tasks; may run on different cores.
- `**select!`**: single task, **branches are not simultaneous**; branches can **borrow** disjointly; only one completion wins; losers **dropped** (cancellation).

### 7.7 `select!` details (Tokio)

- Up to **64** branches; random poll order for fairness; pattern match branches need `**else`** if no pattern always matches.
- Resume async op across iterations: `**pin!**` + `&mut` future in `select!`.
- `**Pin::set**`: Reset pinned op when replacing long-lived future (Tokio select advanced example).

### 7.8 Streams

- `**Stream**`: `poll_next(self: Pin<&mut Self>, cx) -> Poll<Option<Item>>` — like `Future` but many values (Tokio streams).
- `**StreamExt::next**`: async iteration until `None`; **pin** nontrivial streams (`tokio::pin!`) before `next().await` (Tokio streams).
- **Adapters**: `map`, `filter`, `take`, `filter_map` — ordering matters.
- `**async-stream` crate**: `stream!` macro until native async iterators stabilize (Tokio streams).
- **futures crate `select!**`: requires `**FusedFuture**` and `**Unpin**` — often `.fuse()` + `pin_mut!` on futures; different from `tokio::select!` (Async book old `select` chapter).

### 7.9 I/O and framing (Tokio)

- `**AsyncRead` / `AsyncWrite**`: use `AsyncReadExt` / `AsyncWriteExt`; `read` returns `Ok(0)` on EOF.
- `**io::split**`: generic split uses `Arc`/`Mutex`; `**TcpStream::split(&mut)**` zero-cost same-task; `**into_split**` `Arc` — can move across tasks (Tokio I/O).
- **Framing**: buffer in `BytesMut`; parse partial frames; on EOF `0`, if buffer non-empty → error (partial frame) (Tokio framing).
- `**read_buf**`: updates `BufMut` cursor without manual indexing; avoids zero-init cost vs `Vec` resize (Tokio framing).
- `**BufWriter**`: batch writes; remember `flush().await` when needed (Tokio framing).

### 7.10 Runtime selection (docs.rs `tokio::runtime`)

- **Multi-thread (work-stealing)**: default for parallel I/O; worker per core approx.
- **Current-thread**: no extra worker threads; tasks only run under `Runtime::block_on` (or `LocalSet` driving) — good for tests and sync wrappers.
- `**!Send` futures**: use `**LocalSet**` / local runtime path (decision tree in docs).
- **Fairness**: bounded tasks + bounded `poll` time ⇒ eventual scheduling; spurious wakeups allowed.
- **NUMA**: Tokio **not** NUMA-aware — possibly multiple runtimes.

### 7.11 `LocalSet` and `spawn_local` (!Send futures)

Source: [docs.rs `LocalSet](https://docs.rs/tokio/latest/tokio/task/struct.LocalSet.html)`, Tokio tutorial (Send bound).

- `**tokio::spawn` requires `Send**`: `Rc`, some other `!Send` state cannot live across `.await` in a spawned task if it must migrate threads.
- `**LocalSet**`: schedules `**!Send` futures on the current thread only** — no cross-thread migration of that task’s state.
- `**LocalSet::run_until(async { ... }).await**`: valid under `#[tokio::main]`, `#[tokio::test]`, or **directly** inside `Runtime::block_on`. **Invalid inside `tokio::spawn**` — you cannot nest `run_until` that way.
- `**task::spawn_local**`: spawns onto the active `LocalSet`; use inside `run_until` / `enter()` context.
- `**LocalSet` as `Future**`: `local.await` runs until all tasks on the set complete (same placement rules — not from inside `tokio::spawn`).
- **Pattern for `!Send` from multi-threaded runtime**: dedicate a **thread** with `current_thread` runtime + `LocalSet` + `mpsc`/`oneshot` bridge (official example in `LocalSet` docs: `LocalSpawner`).
- `**LocalSet::block_on(&rt, fut)**`: drives local futures; **must not** be called from async context. Docs: `**block_in_place` inside `spawn_local` under `LocalSet::block_on` panics** — use `**spawn_blocking**` for blocking sections instead.

### 7.12 `block_in_place` vs `spawn_blocking`

Source: [docs.rs `block_in_place](https://docs.rs/tokio/latest/tokio/task/fn.block_in_place.html)`, [docs.rs `spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)`.


| Mechanism                | What it does | Caveat  |
| ------------------------ | ------------ | ------- |
| `**task::block_in_place( |              | ...)`** |
| `**task::spawn_blocking( |              | ...)`** |


- `**join!` + `block_in_place`**: other branches of `**join!` in the same task are suspended** during `block_in_place`; if you need true overlap, use `**spawn_blocking`** (docs `block_in_place`).
- **Re-enter async from sync**: inside `block_in_place`, `Handle::current().block_on(async { ... })` is allowed (nested sync entry pattern).

### 7.13 `join!`, `try_join!`, and scheduler fairness

Source: Async Book [part-guide/concurrency-primitives.md](https://rust-lang.github.io/async-book/part-guide/concurrency-primitives.html) (Composing futures concurrently).

- `**tokio::join!(a, b, ...)`**: runs futures **concurrently on the same task** (time-sliced), **not in parallel** — no extra OS threads from the macro itself.
- `**try_join!`**: like `join!`, but if any branch yields `**Err`**, others are **cancelled** (fail-fast).
- **Fairness nuance**: the runtime’s scheduler mostly sees **tasks**, not individual futures inside `join!`. Example from the book: **99 futures joined in one task** can each get ~0.5% of that task’s time vs **99 spawned tasks** each getting ~1% of the pool — biased when massive `join!` fan-in lives on one task. Prefer `**spawn` + `JoinSet`** when you need parallelism and balanced scheduling (book + Tokio `JoinSet` docs).
- **Deadlock risk**: `join!` on same thread — if one future **blocks the thread** (wrong mutex, blocking I/O), **none** of the joined futures progress.
- `**JoinSet`**: dynamic collection of spawned tasks with join semantics; use when the number of tasks grows at runtime (book points to [JoinSet](https://docs.rs/tokio/latest/tokio/task/struct.JoinSet.html)).

### 7.14 `tokio::sync` — Notify, Semaphore, RwLock, watch (quick map)

Extends §7.5 table for LLM routing:

- `**Notify`**: manual **wake one / wake all**; building blocks for locks/conditions without holding `Mutex` across await in simple cases; used internally by many Tokio primitives (see tutorial “Notify utility” for delay example).
- `**Semaphore`**: **limit concurrency** (max N in-flight ops); acquire `.await` releases on drop; pair with `spawn_blocking` pools to cap parallel blocking work.
- `**RwLock`**: async **many readers / one writer**; still subject to **don’t hold across long await unless intentional** — same design discipline as `Mutex`.
- `**watch::channel`**: **single latest value**; great for **config snapshots** and **shutdown flags** (`bool` or enum) where readers only need “current” state; combine with `select!` for graceful shutdown without full `mpsc` history.

---

## 08-unsafe-and-ffi — `Pin` / `Unpin`

From Async Book **part-reference/pinning.md** (fetched):

- `**Pin`**: pointer modifier; guarantees **address validity** for `!Unpin` pointees until drop; compiler erases `Pin` at runtime for sized pointers.
- `**Unpin`**: auto trait; most types are `Unpin`. Opt-out: `PhantomPinned` / internal `!Unpin`.
- **If `T: Unpin`**, `Pin<&mut T>` effectively `&mut T` — pinning irrelevant.
- `**Future::poll` / `Stream::poll_next**`: take `Pin<&mut Self>` so async state machines can hold self-references.
- **Practical**: `tokio::pin!` / `std::pin::pin!` for stack pinning; `Box::pin` for heap; `**Pin::set`** replaces pinned slot safely.
- **Pin projection**: structural vs not; use `**pin-project`** / **pin-project-lite** for safe field projection; unsafe projection must respect structural pinning rules.
- **Drop on `!Unpin`**: treat as pinned in drop glue (`inner_drop` pattern in book).

---

## 09-performance

- **Work-stealing vs current-thread**: former for throughput across cores; latter for minimal threads / deterministic tests / sync bridges (docs.rs, Tokio bridging).
- `**spawn_blocking`**: dedicated blocking pool; grows until max then queues; **cap** CPU-heavy parallelism; prefer **rayon** for pure CPU parallel (Tokio tutorial “when not to use Tokio”).
- `**block_in_place`**: see **§7.12** — relocates other tasks to free the **current** worker on multi-thread runtimes; **forbidden** on `current_thread`; pairs with `Handle::block_on` for sync→async re-entry; avoid under `join!` if you need overlap.
- **File I/O**: OS async file APIs limited; threadpool often as good as async for bulk file read (Tokio tutorial).
- **Cooperative budgeting**: Tokio has cooperative yielding; long CPU loops without `.await` starve runtime — split or `yield_now` (general Tokio guidance + runtime fairness section).
- **LIFO slot** (multi-thread runtime): wake-to-front optimization; can be disabled (`disable_lifo_slot`) (docs.rs runtime).

---

## 11-ecosystem-crate-picks


| Crate                                     | Role                                                                             |
| ----------------------------------------- | -------------------------------------------------------------------------------- |
| **tokio**                                 | Runtime, net, io, time, sync, signal; feature-gate deps                          |
| **tokio-util**                            | `CancellationToken`, codec, `TaskTracker`, etc.                                  |
| **tokio-stream**                          | `StreamExt`, adapters until `Stream` in std                                      |
| **futures**                               | `join!`, `select!` (different semantics), `FuturesUnordered`, `ArcWake`          |
| **futures-concurrency**                   | Tuple/array `join().await` style concurrency (Async book mentions)               |
| **async-trait**                           | Dynamic dispatch async traits when needed (heap per call); see §12               |
| **pin-project / pin-project-lite**        | Safe pin projection                                                              |
| **bytes**                                 | `Bytes`, `BytesMut`, cheap clones for networking                                 |
| **tracing**, **tracing-subscriber**       | Structured async-friendly diagnostics (Tokio topics/tracing)                     |
| **console-subscriber**, **tokio-console** | Live task/resource view (`tokio_unstable`, `RUSTFLAGS` cfg) (tracing-next-steps) |
| **reqwest**, **hyper**, **axum**          | De facto HTTP stack in Tokio ecosystem (mentioned in Tokio docs / tutorials)     |
| **async-std**                             | std-like async API + own runtime — alternative to Tokio for some apps            |
| **actix**                                 | Actor framework (often with Tokio); **actix-web** is separate (HTTP)             |
| **async-task**                            | Executor-building block; dependency of several runtimes                          |
| **thin_main_loop**                        | Experimental GUI-oriented main loop / executor                                   |


---

## 12-modern-rust — AFIT, RPITIT, `impl Trait`

- **Historical**: Pre-stabilization, `async fn` in traits required nightly or `**async-trait`** (erased to `Box<dyn Future + Send>`-style) — allocation per call (Async book `async_in_traits.md` — **outdated narrative**).
- **Rust 1.75+**: `**async fn` in traits** stabilized with desugaring to return-position `impl Future` (RPITIT) — use native `async fn` in traits for static dispatch; still evolving for `**dyn Trait`** object safety (see project releases / [Rust 1.75 release notes](https://releases.rs/docs/1.75.0/) for AFIT).
- `**impl Future` vs `dyn Future` vs `Box<dyn Future>`**:
  - `**impl Future**`: zero-cost, monomorphized; preferred in public APIs when concrete.
  - `**dyn Future + Send**`: type-erased; needed for heterogeneous collections; pinned boxed.
  - `**Box<dyn Future<Output = T> + Send>**`: common for trait objects or recursive/async recursion patterns.

---

## Cancellation safety (explicit)

- Async book **cancellation.md** in repo is still a **stub** (headings only); Tokio defines behavior: **drop future** ⇒ cancel; only at **await points** cooperative; `select!` drops losing branches.
- **Not cancel-safe**: buffered channel slots filled then cancelled sender; `Mutex` guard holding invariant; multi-step commit without idempotence — pattern: **defer** with guard types or transactional boundaries.

---

## Bridging sync ↔ async (Tokio topics/bridging)

- `**#[tokio::main]`** expands to `Runtime::new_multi_thread().enable_all().build().unwrap().block_on(async { ... })` (bridging doc).
- **Embed async in sync**: store `Runtime` (`current_thread` for single-threaded `block_on` wrappers); each method `rt.block_on(async { ... })`.
- `**current_thread` caveat**: spawned tasks **freeze** when not inside `block_on` — use **multi_thread** if background tasks must run while sync code does other work.
- **Runtime in thread + `mpsc`**: actor spawner pattern; `blocking_send` from sync side.

---

## 13-ecosystem-snapshot — recent versions (multi-crate)

Condensed from upstream **CHANGELOG** / release notes (for LLM “what changed since the tutorial”). Versions are as stamped in those repos around **2025–2026**; pin exact numbers in `Cargo.lock` for production.

### 13.0 Async ecosystem — core building blocks (catalog)

Short orientations for LLMs. These are **orthogonal layers**: a **runtime** (executor + I/O), optional **actor** framework, optional **HTTP** stack — pick one coherent stack per binary unless you explicitly bridge runtimes.


| Component                                                     | What it is                                                                                                                         | Notes                                                                                                                                                                                                                                      |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **[tokio](https://crates.io/crates/tokio)**                   | Event-driven, **non-blocking I/O** platform: scheduler, timers, TCP/UDP/Unix, fs, sync primitives, signals.                        | **Default choice** for network servers and most async Rust. `**async`/`await`** has been the supported model since the **0.2.x alpha** line onward (modern 1.x is the stable series). Builds on `**mio`** (epoll/kqueue/…) under the hood. |
| **[async-std](https://crates.io/crates/async-std)**           | “Async stdlib”: `**std`-like** APIs (`fs`, `net`, …) for `**async`/`.await`**, with its own runtime.                               | Ergonomic if you want **familiar names**; ecosystem is **smaller** than Tokio’s for servers. Usually **one** primary runtime per process — do not mix two full runtimes blindly.                                                           |
| **[actix](https://crates.io/crates/actix)**                   | **Actor framework** (message-passing, `Arbiter`, supervision patterns) historically tied to the **Tokio** ecosystem.               | Higher-level concurrency model than raw `spawn` + channels. Distinct from `**actix-web`** (HTTP server framework **using** actors under the hood for request handling).                                                                    |
| **[thin_main_loop](https://crates.io/crates/thin_main_loop)** | **Experimental**, cross-platform **main loop** + futures **executor/reactor** bound to **OS APIs** suited for **native GUI** apps. | Supports callbacks and `**async`/`await`**; **not** aimed at datacenter HTTP services — use when integrating Rust async with **windowing/event loops** (GUI).                                                                              |
| **[async-task](https://crates.io/crates/async-task)**         | Minimal **task** abstraction (`Task`, wake, spawn hook) for **implementing executors**.                                            | **Infrastructure**: used by **async-std**, **smol**, and other runtimes. Application code rarely imports it unless you **author a custom executor** or embed a runtime.                                                                    |


**Related (often confused)**

- `**actix-web`** — Web framework (routing, extractors), not the same crate as `**actix`** (actors), though names and community overlap.
- `**smol**`, `**async-executor**`, `**futures` task pools** — other scheduling building blocks; see §13.7.

### 13.1 Tokio (runtime)

Source: [tokio CHANGELOG](https://github.com/tokio-rs/tokio/blob/master/tokio/CHANGELOG.md).


| Topic                                       | Notes                                                                                                                   |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `**LocalRuntime` stabilized** (≈1.51)       | First-class API for **single-thread / `!Send`** workloads; compare with `LocalSet` + `current_thread` in docs.          |
| `**worker_index()**`, **runtime `name`**    | Debugging and **multi-runtime** processes (metrics / logs).                                                             |
| `**JoinSet: Extend`** (≈1.49)               | Bulk add spawned tasks.                                                                                                 |
| `**runtime::id::Id`**, `**LocalSet::id()**` | Correlate telemetry with runtime / local set.                                                                           |
| `**is_rt_shutdown_err**` (≈1.50)            | Classify shutdown errors explicitly.                                                                                    |
| `**spawn_blocking` queue work** (≈1.52)     | Scalability tweak; **1.52.1** fixes a **hang regression** introduced in 1.52.0 — stay on **≥1.52.1** if you hit 1.52.0. |
| **I/O safety / pipes**                      | `AioSource::register_borrowed`; `unix::pipe` `try_io` (≈1.52).                                                          |
| `**wasm32-wasip2` networking** (≈1.51)      | Async HTTP servers on WASI P2.                                                                                          |
| **Unstable**                                | `io_uring` + `File`/`AsyncRead`, eager driver handoff, taskdump `trace_with` — behind cfg / features.                   |


**LTS / MSRV:** follow Tokio release posts — changelog mentions **LTS lines** and **MSRV** (e.g. 1.71 in ecosystem crates); verify when upgrading.

### 13.2 `hyper` (HTTP protocol)

Source: [hyper CHANGELOG](https://github.com/hyperium/hyper/blob/master/CHANGELOG.md).

- **1.9.x**: HTTP/1 keep-alive + chunked trailers; HTTP/2 client body cancel on drop; `Error::is_parse_version_h2`; `UpgradeableConnection::into_parts`; HTTP/2 max stream visibility / `max_local_error_reset_streams`.
- **1.8.x**: `Timer::now()` for custom time sources; fixes missed wakeups, HTTP/2 CONNECT upgrades; **breaking**: HTTP/2 client executor must be able to **spawn**; `Http2ClientConnExec` no longer **dyn-safe** (edge case).
- **1.7.x**: `Error::is_shutdown()`; HTTP/1 relaxed request-line parsing option; informational response callback (`ext::on_informational`).
- **Practical**: `hyper` is **not** a full client high-level API — pair with `**hyper-util`**, `**tower`**, or `**reqwest**`.

### 13.3 `axum` (routing / extractors)

Source: [axum CHANGELOG](https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md).

- **0.8.0** (major): path syntax `**/{param}`**, `**/{*rest}`**; `**Sync**` on handlers/services; custom extractors often **without `#[async_trait]`** (native AFIT); `Option` extractor semantics tightened.
- **0.8.5+**: stricter JSON (reject trailing junk); SSE/binary improvements; `**FusedStream` for `WebSocket`**; MSRV bumps (e.g. **1.78 → 1.80** by 0.8.9) — check toolchain.
- **0.8.9**: WebSocket subprotocol selection helpers; fixes multipart limit errors.
- **Unreleased (check before upgrade)**: nested **fallback merging**, `serve` + **hyper `header_read_timeout`**, `**ListenerExt::limit_connections**`, `serve` type / graceful-shutdown output tweaks — **plan migrations**.

### 13.4 `futures` (`futures-util` / `futures`)

Source: [futures-rs CHANGELOG](https://github.com/rust-lang/futures-rs/blob/master/CHANGELOG.md).

- **0.3.32**: soft-deprecate `**ready!`** → `**std::task::ready!`**; `**pin_mut!**` → `**std::pin::pin!**`; `FuturesOrdered::clear`; `mpsc` receivers `**recv` / `try_recv**`; `**try_next` deprecated** on receivers; `**Mutex::new` const**; MSRV bumps in utility crates; removed `**pin-utils`**, `**num_cpus`** deps.
- **0.3.31** (soundness): fixes `**FuturesUnordered`** drop + panic edge cases; `**waker_ref`** soundness; `**select!` parsing** stricter (may break odd macro uses).
- **Interop**: Tokio `**join!`/`select!`** ≠ `**futures::select!`** (different `Unpin`/`Fuse` rules) — document both in KB.

### 13.5 `reqwest` (HTTP client)

Source: [reqwest CHANGELOG](https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md).

- **0.13.0** (**breaking**): default TLS `**rustls`** (not `native-tls`); crypto provider default **aws-lc**; feature renames (`rustls-tls` → `rustls`); `**query` / `form` optional features**; DNS `**hickory-dns`** (old `trust-dns` gone); API renames for TLS builder methods.
- **0.12.x**: retries, unix sockets, named pipes, HTTP/3 knobs, `**tower-http`**-style internals — read changelog when jumping minor versions.

### 13.6 `tower` / `tower-http` (Service stack)

- `**Service` / `Layer`** abstractions: Axum handlers and `**tower-http**` middleware (trace, compression, limits) compose as **layers**.
- Version **pin per application** — no single global changelog here; treat as **middleware contract** with `hyper`/`axum` upgrades.

### 13.7 Other async runtimes & DB (one-liners)


| Crate                    | Role / note                                                                                                |
| ------------------------ | ---------------------------------------------------------------------------------------------------------- |
| **async-std**            | Alternative runtime; smaller ecosystem than Tokio for new servers.                                         |
| **smol**                 | Small executor; often embedded or combined with other stacks.                                              |
| **glommio** / **monoio** | Thread-per-core / io_uring-oriented — niche, different constraints.                                        |
| **sqlx**                 | Async DB: runtime features `**runtime-tokio`**, `**runtime-async-std`** — **must match** app runtime.      |
| **mio**                  | Low-level polling; **Tokio** builds on it — upgrade Tokio, not mio directly, unless you maintain a poller. |


### 13.8 Rust compiler (cross-cutting)

- **AFIT / RPITIT** stable since **1.75**; remaining issues: `**Send` on returned futures**, `**dyn` trait** object safety, **async closures** still evolving — track [areweasyncyet.rs](https://areweasyncyet.rs/) and Rust release notes.
- `**async Iterator` in std**: still distinct from `**futures::Stream`** / `**tokio_stream`** — three layers in the wild.

### 13.9 KB maintenance checklist

1. After any **major** `reqwest` / `axum` bump: re-read **TLS defaults** and **MSRV**.
2. After **hyper** bump: check **HTTP/2 executor** / custom connector code.
3. After **futures** 0.3.31+: audit `**select!`** macros and `**FuturesUnordered`** heavy use.
4. Tokio: prefer **LTS** lines for long-lived services; note **1.52.0 vs 1.52.1** if on the fence.

---

## Source index (fetched)

1. [https://rust-lang.github.io/async-book/](https://rust-lang.github.io/async-book/) — intro, book structure
2. [https://raw.githubusercontent.com/rust-lang/async-book/master/src/part-reference/pinning.md](https://raw.githubusercontent.com/rust-lang/async-book/master/src/part-reference/pinning.md) — full pinning chapter
3. [https://raw.githubusercontent.com/rust-lang/async-book/master/src/02_execution/02_future.md](https://raw.githubusercontent.com/rust-lang/async-book/master/src/02_execution/02_future.md) — Future trait
4. [https://raw.githubusercontent.com/rust-lang/async-book/master/src/02_execution/03_wakeups.md](https://raw.githubusercontent.com/rust-lang/async-book/master/src/02_execution/03_wakeups.md) — Waker
5. [https://raw.githubusercontent.com/rust-lang/async-book/master/src/06_multiple_futures/03_select.md](https://raw.githubusercontent.com/rust-lang/async-book/master/src/06_multiple_futures/03_select.md) — futures `select!`
6. [https://raw.githubusercontent.com/rust-lang/async-book/master/src/07_workarounds/03_send_approximation.md](https://raw.githubusercontent.com/rust-lang/async-book/master/src/07_workarounds/03_send_approximation.md) — Send approximation
7. [https://tokio.rs/tokio/tutorial/](https://tokio.rs/tokio/tutorial/)* — hello-tokio, spawning, shared-state, channels, io, framing, async, select, streams
8. [https://tokio.rs/tokio/topics/bridging](https://tokio.rs/tokio/topics/bridging) — bridging
9. [https://tokio.rs/tokio/topics/shutdown](https://tokio.rs/tokio/topics/shutdown) — graceful shutdown
10. [https://tokio.rs/tokio/topics/tracing](https://tokio.rs/tokio/topics/tracing) — tracing intro
11. [https://tokio.rs/tokio/topics/tracing-next-steps](https://tokio.rs/tokio/topics/tracing-next-steps) — console, OTel
12. [https://docs.rs/tokio/latest/tokio/runtime/index.html](https://docs.rs/tokio/latest/tokio/runtime/index.html) — runtime
13. [https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) — spawn_blocking
14. [https://docs.rs/tokio/latest/tokio/task/struct.LocalSet.html](https://docs.rs/tokio/latest/tokio/task/struct.LocalSet.html) — LocalSet
15. [https://docs.rs/tokio/latest/tokio/task/fn.block_in_place.html](https://docs.rs/tokio/latest/tokio/task/fn.block_in_place.html) — block_in_place
16. [https://raw.githubusercontent.com/rust-lang/async-book/master/src/part-guide/concurrency-primitives.md](https://raw.githubusercontent.com/rust-lang/async-book/master/src/part-guide/concurrency-primitives.md) — join/select fairness
17. [https://github.com/tokio-rs/tokio/blob/master/tokio/CHANGELOG.md](https://github.com/tokio-rs/tokio/blob/master/tokio/CHANGELOG.md) — Tokio releases
18. [https://github.com/hyperium/hyper/blob/master/CHANGELOG.md](https://github.com/hyperium/hyper/blob/master/CHANGELOG.md) — hyper
19. [https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md](https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md) — axum
20. [https://github.com/rust-lang/futures-rs/blob/master/CHANGELOG.md](https://github.com/rust-lang/futures-rs/blob/master/CHANGELOG.md) — futures
21. [https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md](https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md) — reqwest

**Note:** Pages such as `tokio/tutorial/graceful-shutdown` / `cancellation` are **not** separate tutorial URLs on the live site; use **Topics → shutdown** and Tokio `select!`/drop cancellation docs instead.

---

## Кратко по-русски (дополнение)

- `**LocalSet` + `spawn_local`**: единственный «правильный» путь для `**!Send`**-фьючерсов на Tokio без отправки между потоками; `run_until` нельзя вызывать из задачи, созданной обычным `tokio::spawn` — нужен отдельный поток/мост или другая архитектура.
- `**block_in_place**`: блокирует **текущий** worker, но на **multi-thread** рантайме остальные задачи могут уехать на другие воркеры; на `**current_thread`** — **panic**. Не отменяется при shutdown так же грустно, как `spawn_blocking`.
- `**join!` vs много `spawn`**: `join!` не даёт параллелизма на нескольких ядрах и может перекосить справедливость планировщика, если в одной задаче «упаковать» очень много фьючерсов; для большого числа независимых задач часто лучше `**spawn` + `JoinSet`**.
- `**Semaphore**`: типичный способ **ограничить** число одновременных `spawn_blocking` или тяжёлых операций.
- `**watch`**: удобен для **одного актуального значения** (конфиг, флаг остановки) без истории сообщений.
- **§13 / другие крейты**: `**reqwest` 0.13** — смена дефолтного TLS на **rustls** и пересбор фич; `**axum` 0.8** — новый синтаксис **путей** `/{id}`, осторожно с **MSRV**; `**futures` 0.3.32** — предпочитать `**std::pin::pin!`** и `**std::task::ready!`** вместо устаревающих макросов из `futures`; `**hyper` 1.8+** — нюансы HTTP/2 client + executor.
- **§13.0**: **Tokio** — основной рантайм для сети; **async-std** — свой рантайм и «как std»; **actix** — акторы (не путать с **actix-web**); **async-task** — кирпич для своих executor’ов; **thin_main_loop** — эксперимент под **GUI**, не под сервер.

---

## Cross-reference to existing cluster file

This workspace already contains an expanded static KB: `_research/cluster-06-async-tokio.md`. **This** file is the **source-linked** extraction pass from live fetched material (April 2026), including **§13 multi-crate changelog snapshot** (merged from the former standalone `_research/async-ecosystem-snapshot-recent.md`).