# Cluster 06 — Async Rust & Tokio (Expert Knowledge Base)

> Source material: Rust Async Book (https://rust-lang.github.io/async-book/), Tokio Tutorial (https://tokio.rs/tokio/tutorial), docs.rs/tokio.
> Note: Live web fetch was unavailable in this research session; content below is synthesized from deep knowledge of these canonical resources plus the Tokio crate docs as of Tokio 1.x and Rust 1.75+ (AFIT stable) through modern 2024/2025 state.
>
> Orientation: this file is fed to coding LLMs. Prefer exact trait signatures, cancellation-safety notes, and pitfall callouts over narrative prose.

---

## Table of Contents

- [0. Executive cheat-sheet](#0-executive-cheat-sheet)
- [1. Mental model — why async?](#1-mental-model--why-async)
- [2. Core machinery — the `Future` trait](#2-core-machinery--the-future-trait)
- [3. Executors, runtimes, and wakers](#3-executors-runtimes-and-wakers)
- [4. `Pin`, `Unpin`, and self-referential futures](#4-pin-unpin-and-self-referential-futures)
- [5. `async fn` / `await` desugaring](#5-async-fn--await-desugaring)
- [6. `Send` bounds and `!Send` futures](#6-send-bounds-and-send-futures)
- [7. Cancellation & cancellation safety](#7-cancellation--cancellation-safety)
- [8. `select!`, `join!`, `try_join!`, `FuturesUnordered`](#8-select-join-try_join-futuresunordered)
- [9. Streams (`Stream`, `StreamExt`)](#9-streams-stream-streamext)
- [10. Tokio runtime internals (07-async-concurrency primary)](#10-tokio-runtime-internals)
- [11. `tokio::spawn`, `spawn_blocking`, `block_in_place`, `LocalSet`](#11-tokiospawn-spawn_blocking-block_in_place-localset)
- [12. `tokio::sync` primitives — when each fits](#12-tokiosync-primitives)
- [13. I/O, framing, and codecs](#13-io-framing-and-codecs)
- [14. Graceful shutdown patterns](#14-graceful-shutdown-patterns)
- [15. Tracing & observability](#15-tracing--observability)
- [16. Actor pattern in Tokio](#16-actor-pattern-in-tokio)
- [17. Design patterns catalogue](#17-design-patterns-catalogue)
- [18. Anti-patterns — things that look OK but aren't](#18-anti-patterns)
- [19. Performance tuning](#19-performance-tuning)
- [20. Modern Rust async features (AFIT, RPITIT, TAIT)](#20-modern-rust-async-features-afit-rpitit-tait)
- [21. Ecosystem crate picks](#21-ecosystem-crate-picks)
- [22. Unsafe & FFI touchpoints for async](#22-unsafe--ffi-touchpoints-for-async)
- [23. Quick reference — decision tables](#23-quick-reference--decision-tables)

---

## 0. Executive cheat-sheet

Fast lookup table. Each row maps directly to a section below.

| Question | Short answer |
|---|---|
| "My program hangs briefly then everything stops." | You called a blocking API in async. Move to `spawn_blocking`. |
| "I got `MutexGuard<..>` is not `Send`." | Holding `std::sync::Mutex` across `.await`. Drop guard before `.await` or use `tokio::sync::Mutex`. |
| "`future cannot be sent between threads safely`" | Something non-`Send` lives across an `.await`. Find the culprit with `cargo rustc -- -Zdump-mir=..` or by binary search. Use `LocalSet` or restructure. |
| "My `select!` branch dropped data." | Branch wasn't cancel-safe. Use `tokio::pin!` a long-lived future or move state outside `select!`. |
| "How many threads does Tokio use?" | `new_multi_thread` = `num_cpus::get()` workers; `new_current_thread` = one (the calling thread). |
| "Spawn and forget ⇒ memory leak?" | Detached `JoinHandle` is fine; don't `mem::forget`. Use `JoinSet` when you need structured cancellation. |
| "Is `tokio::sync::Mutex` slower than `std::sync::Mutex`?" | Yes, noticeably. Use `std::sync::Mutex` if the guard never crosses `.await`. |
| "Should I `Box::pin` or `tokio::pin!`?" | `tokio::pin!` for stack-pinned locals, `Box::pin` when you need heap + `Send` as a value. |
| "Do I still need `#[async_trait]`?" | Rarely. Rust 1.75 stabilized AFIT; use plain `async fn` in trait. `#[async_trait]` only for `dyn Trait` + object-safety. |
| "Why is my throughput bad with many tasks?" | Fat futures; long `.await`-free hot loops blocking the worker; not using `tokio::task::yield_now()`. |

---

## 1. Mental model — why async?

### 1.1 The problem async solves

- **Thread-per-connection** cost: ≈1–8 MiB stack per OS thread; context switches kernel-mediated.
- **Async** turns every waiting computation into a state machine compiled by `rustc`. One OS thread drives thousands of state machines.
- Rust picks the **zero-cost stackless coroutine** model: no per-task heap stack, each future is only as large as its state.

```
10 000 TCP connections
├─ thread-per-conn  : 10 000 × 2 MiB = 20 GiB of stack reservation, 10 000 fds
└─ tokio::spawn     : 10 000 tasks × sizeof(future)   (often 200–500 bytes each)
```

### 1.2 Trade-offs (be honest)

Async is not uniformly better:

- **Latency-sensitive CPU-bound code** → threads often simpler, fewer gotchas.
- **Lots of blocking syscalls** → threads again (or a dedicated blocking pool).
- **Massive I/O fan-out, many connections, timers** → async wins, often by orders of magnitude.
- **"Function coloring"** → `async fn` can only be called from `async` or via a runtime. This is a language-level constraint — accept it, don't fight it.

### 1.3 What Rust async is NOT

- Not green threads. There's no scheduler preempting tasks between statements; tasks are preemption points only at `.await`.
- Not a runtime. `rustc` knows about `Future` and `.await`; it does NOT know about Tokio, async-std, smol, or any executor.
- Not built-in concurrency. You must use combinators (`join!`, `select!`, `FuturesUnordered`) or explicit `spawn` to get concurrency.

---

## 2. Core machinery — the `Future` trait

### 2.1 The trait, exact signature (std)

```rust
// core::future
pub trait Future {
    type Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
}

pub enum Poll<T> {
    Ready(T),
    Pending,
}
```

That's it. Three items. The entire async system rests on these.

### 2.2 Contract — what `poll` means

- `poll(Pending)` ⇒ "I'm not done. I've arranged for the waker in `cx` to be called when I *might* make progress."
- `poll(Ready(v))` ⇒ "Here's the value. **Do not poll me again** — panics and UB in many implementations."
- Idempotent `Ready`? **No.** Calling `poll` after `Ready` is a bug. Wrappers like `Fuse` exist to make it safe.
- Spurious wakeups are allowed — executors may re-poll even when nothing changed. `poll` must tolerate this.

### 2.3 `Context` / `Waker` semantics

```rust
pub struct Context<'a> { /* opaque; carries a &'a Waker */ }

impl Context<'_> {
    pub fn waker(&self) -> &Waker;
    // Since 1.83:
    pub fn ext(&mut self) -> &mut ContextExt<'_>;  // for extension fields
}

pub struct Waker { /* opaque */ }

impl Waker {
    pub fn wake(self);            // consumes
    pub fn wake_by_ref(&self);    // cheap, takes ref
    pub fn clone(&self) -> Waker; // Arc-like, bumps refcount
    pub fn will_wake(&self, other: &Waker) -> bool;
}
```

Key rules:

1. A future must **store the latest waker** before returning `Pending`. Old wakers may point to a task that no longer exists.
2. If the future re-polls itself with a different `Context`, compare `will_wake(old)`; if false, replace the waker. Optimization only — cloning is always safe.
3. Calling `wake()` does not synchronously re-poll; it schedules the task. The current poll should return `Pending` immediately after waking itself.
4. **Sending wakers across threads is fine.** `Waker: Send + Sync`.

### 2.4 Hand-rolled future (the shape you'll see)

```rust
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct TimerFuture {
    shared: Arc<Mutex<Shared>>,
}

struct Shared {
    completed: bool,
    waker: Option<Waker>,
}

impl Future for TimerFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut s = self.shared.lock().unwrap();
        if s.completed {
            Poll::Ready(())
        } else {
            // Store latest waker. Crucially: take(), don't just insert,
            // to avoid stale wakers hanging around.
            s.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl TimerFuture {
    pub fn new(dur: Duration) -> Self {
        let shared = Arc::new(Mutex::new(Shared { completed: false, waker: None }));
        let s2 = shared.clone();
        thread::spawn(move || {
            thread::sleep(dur);
            let mut g = s2.lock().unwrap();
            g.completed = true;
            if let Some(w) = g.waker.take() { w.wake(); }
        });
        Self { shared }
    }
}
```

Patterns embedded above:

- Lock-protected `completed + waker` — both written together on completion.
- Waker stored only when pending. On ready, no waker needed.
- `waker.take()` ensures we don't double-wake on spurious re-polls.

### 2.5 `Poll` helpers you'll see in real code

```rust
use std::task::{ready, Poll};

// `ready!` = try-operator for Poll
fn poll_next(cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
    let data = ready!(self.inner.poll_read(cx))?; // early-returns Pending
    // ... continue with `data`
    Poll::Ready(Ok(data.len()))
}
```

- `std::task::ready!` — stable since 1.64. Short for `match p { Ready(v) => v, Pending => return Pending }`.
- `futures::ready!` is the older ecosystem equivalent; identical behavior.

---

## 3. Executors, runtimes, and wakers

### 3.1 What an executor actually does

Minimum viable executor loop:

```rust
loop {
    // pick next task off a ready-queue
    let task = ready_queue.pop();
    // poll the task with a waker that re-queues it
    match task.future.as_mut().poll(&mut Context::from_waker(&task.waker)) {
        Poll::Ready(_) => { /* drop task */ }
        Poll::Pending => { /* task will re-queue itself via its waker */ }
    }
}
```

The loop is driven entirely by wakers pushing tasks back into `ready_queue`. No polling wheel, no timer — those are *drivers* the executor embeds.

### 3.2 Tokio's layered architecture

```
┌──────────────────────────┐
│  Your async fn / tasks   │
├──────────────────────────┤
│  Scheduler (multi_thread │
│  or current_thread)      │
├──────────────────────────┤
│  Driver (mio epoll /     │
│  kqueue / IOCP, timers)  │
└──────────────────────────┘
```

Key crates / components:

- **`tokio::runtime::Runtime`** — owns the scheduler + driver threads.
- **`tokio::runtime::Handle`** — cheap-clone, used to enter the runtime from outside.
- **`tokio::runtime::EnterGuard`** — RAII guard that makes the current thread "inside" a runtime.
- **I/O driver** — built on `mio`, runs on either a dedicated thread (current-thread) or interleaved with workers (multi-thread).
- **Time driver** — hierarchical timing wheel, O(1) insertion/removal.

### 3.3 Multi-thread vs current-thread

| | `new_multi_thread` | `new_current_thread` |
|---|---|---|
| Threads | N workers + blocking pool | Calling thread + blocking pool |
| Work stealing | Yes | No |
| Futures must be `Send` | Yes | No (LocalSet not required for `!Send` tasks spawned via `tokio::task::spawn_local` under a `LocalSet`; but runtime-root futures must still be `Send` unless you use `LocalSet::block_on` / `Runtime::block_on` with a `!Send` future on current-thread) |
| Fairness | Per-worker LIFO slot + global FIFO | FIFO |
| Best for | Servers, high-concurrency | Tests, single-threaded apps, GUI integration |
| Footprint | Heavier | Tiny |

Rule of thumb: start with `#[tokio::main]` (multi-thread). Drop to current-thread when you have proven thread contention is negative or you need `!Send` state.

### 3.4 Building a runtime manually

```rust
use tokio::runtime::Builder;

fn main() -> std::io::Result<()> {
    let rt = Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("app-worker")
        .max_blocking_threads(512)
        .enable_all()                // enable IO + time drivers
        .on_thread_start(|| { /* per-thread init */ })
        .on_thread_stop(|| { /* per-thread teardown */ })
        .event_interval(61)          // poll count before I/O check; odd primes avoid resonance
        .global_queue_interval(31)   // steal from global queue every N ticks
        .build()?;

    rt.block_on(async_main())
}
```

`.enable_all()` turns on **both** I/O and time drivers. `.enable_io()` / `.enable_time()` are the fine-grained versions — matters if you only want timers and not a socket driver.

### 3.5 LIFO slot and work stealing

Tokio's multi-thread runtime uses **local run queues with LIFO slot**:

- Each worker has a fixed-size ring buffer (256 tasks).
- Plus a single-slot LIFO cache — the *very* next scheduled task tends to go here, reducing cache-miss for short-lived wake/poll pairs.
- When your queue is empty, steal half of another worker's queue.
- Global injection queue: tasks spawned from outside any worker thread land here; workers poll it periodically (`global_queue_interval`).

Implication: `spawn` from a worker has different locality than `spawn` from a `block_in_place` region or from an OS thread.

### 3.6 The waker implementation in Tokio

Tokio wakers are essentially:
```
Arc<Task>   // refcount on a Task header
+ fn ptr to wake-into-scheduler
```

When `.wake()` is called:
- If task is already in the queue → no-op (idempotent; prevents double-scheduling).
- Else: push onto the originating worker's local queue (or global queue if cross-thread) and possibly notify a parked worker.

---

## 4. `Pin`, `Unpin`, and self-referential futures

### 4.1 Why Pin exists (the actual problem)

```rust
async fn f() {
    let x = [0u8; 128];
    let r = &x[..];            // borrows x
    some_io().await;            // suspends; state machine now holds both `x` AND `r`
    println!("{:?}", r);        // uses r after resume
}
```

The compiler generates a struct with fields `x: [u8;128]` and `r: &[u8]`. `r` points *into* `x` of the same struct — **self-referential**. Moving that struct invalidates `r`. Rust cannot generally express "this struct cannot be moved", so `Pin` gates the *access* instead: you only get `&mut Self` from a `Pin<&mut Self>` if you promise not to move.

### 4.2 The `Pin` contract

```rust
pub struct Pin<P> where P: Deref { /* private */ }

impl<P: Deref<Target = T>, T: ?Sized> Pin<P> {
    // Safe when T: Unpin
    pub fn new(p: P) -> Self where T: Unpin;
    // Unsafe constructor — "I promise to uphold the pin contract"
    pub unsafe fn new_unchecked(p: P) -> Self;
}
```

Rules:

1. **Once `Pin<&mut T>` or `Pin<Box<T>>` exists for a `T: !Unpin`, `T`'s memory must not move until `T` is dropped.**
2. `Drop` must run before memory is deallocated, even if the pinned value was about to be leaked — this is why you can't `mem::forget` inside `Pin::new_unchecked` regions except via structural pinning rules.
3. `Pin<&mut T>::as_mut()` and `get_mut()` only exist if `T: Unpin`. For `!Unpin`, you reach the underlying memory with `unsafe { self.get_unchecked_mut() }`.

### 4.3 When is `Unpin` automatic

- `Unpin` is an auto-trait. All primitive types are `Unpin`.
- `Box<T>`, `Vec<T>`, `Arc<T>` — always `Unpin` (the heap allocation doesn't move).
- `&T`, `&mut T` — `Unpin`.
- **`async fn` return types are `!Unpin`.** The compiler-generated state machine opts *out* of `Unpin`.
- Structs are `Unpin` iff all fields are.

### 4.4 When do you need manual pinning (and how)

Three common patterns:

**(a) Stack pinning** — for local use, no allocation:
```rust
let fut = some_async_fn();
tokio::pin!(fut);                 // makes `fut: Pin<&mut _>`
poll_something(fut.as_mut());
```

`tokio::pin!` expands roughly to:
```rust
let mut fut = some_async_fn();
// SAFETY: shadowing prevents moving the original binding.
#[allow(unused_mut)]
let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
```

`futures::pin_mut!` is the equivalent in the `futures` crate.

**(b) Heap pinning** — when you need to own and move the pinned pointer:
```rust
let boxed: Pin<Box<dyn Future<Output=()>>> = Box::pin(async move { /* ... */ });
```

This is the form that crosses function boundaries, goes in structs, etc.

**(c) Structural pinning via `pin-project`** — when your struct contains futures:

```rust
use pin_project::pin_project;

#[pin_project]
pub struct TimeoutStream<S> {
    #[pin] inner: S,       // field is pin-projected
    deadline: Instant,     // regular field
}

impl<S: Stream> Stream for TimeoutStream<S> {
    type Item = S::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        let this = self.project();
        // this.inner is Pin<&mut S>
        // this.deadline is &mut Instant
        this.inner.poll_next(cx)
    }
}
```

Rule (**structural pinning**): If field `f` is `#[pin]`, the struct's `Drop` must not move out of `f`, and you must not expose `&mut f`. `pin-project` enforces this at macro-expansion time.

Alternative: `pin-project-lite` — macro-only, no proc-macro dep, smaller compile time, slightly less flexible (no `#[pinned_drop]`).

### 4.5 Common Pin mistakes

- Implementing `Drop` on a `#[pin_project]` struct with `#[pin]` fields — triggers `E0277` because `Drop` on a `!Unpin` struct can't receive `Pin<&mut Self>` directly. Solution: `#[pinned_drop]` attribute.
- Putting `Pin<&mut T>` in a struct that then gets moved — `Pin<&mut T>` is `Unpin` itself (it's a reference), but the *referent* must not move. Making sure the referent lives long enough is on you.
- `Box::pin(fut)` and then `mem::forget(boxed)` — leaks and is safe, but breaks structural-pin invariants for future authors if they assumed `Drop` would run. It's allowed by the contract; just surprising.

---

## 5. `async fn` / `await` desugaring

### 5.1 What `async fn` becomes

```rust
async fn read_byte(s: &mut Socket) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    s.read_exact(&mut buf).await?;
    Ok(buf[0])
}
```

roughly desugars to:

```rust
fn read_byte<'a>(s: &'a mut Socket)
    -> impl Future<Output = io::Result<u8>> + 'a
{
    async move {
        let mut buf = [0u8; 1];
        s.read_exact(&mut buf).await?;
        Ok(buf[0])
    }
}
```

Key facts:

- Return type is `impl Future<Output=T> + 'lifetime` where `'lifetime` is the union of all input lifetimes. (This is the source of much pain with borrowing.)
- The inner `async move { .. }` block produces an anonymous state machine type.
- Captures are *by move* in `async move { }`, or by reference/move as needed in plain `async { }`.

### 5.2 The state machine

Every `.await` is a state. Two states per await:

1. **Before**: the sub-future hasn't been created/started.
2. **Polling**: hold the sub-future, poll it until Ready, collect its output.
3. **After**: move on.

The overall enum has one variant per suspension point plus a `Start` and `Done`. Compiler lays them out `union`-style so size = max size across variants, not sum.

### 5.3 Borrows across `.await`

The state machine holds all locals that live across `.await`. This means:

```rust
async fn bad(m: &std::sync::Mutex<u32>) {
    let g = m.lock().unwrap();    // MutexGuard borrows from `m`
    some_future().await;           // guard held across .await
    println!("{}", *g);
}
```

`MutexGuard` is `!Send`. The future captures a `!Send` value across `.await` ⇒ the future is `!Send` ⇒ `tokio::spawn(bad(m))` won't compile.

Fix: either use `tokio::sync::Mutex` (its guard *is* `Send`), or keep `std::sync::Mutex` but drop the guard before `.await`:

```rust
async fn good(m: &std::sync::Mutex<u32>) {
    let copy = {
        let g = m.lock().unwrap();
        *g
    };                              // guard dropped here
    some_future().await;
    println!("{}", copy);
}
```

### 5.4 `await` at the call site — idiom

Rust deliberately made `await` a **postfix**:

```rust
foo().await?.bar().await
//     ^^^^^ chains cleanly with `?` and method calls
```

compared to hypothetical prefix `await`:

```rust
(await (await foo())?).bar()     // prefix form — parens soup
```

Idiomatic patterns:

- `?` after `.await` is normal: `tokio::fs::read(path).await?`.
- Fluent chains: `client.get(url).send().await?.json::<T>().await?`.
- Don't write `let x = fut.await; x` — redundant.
- Don't wrap `async { fut.await }` around an existing future — no-op, just returns the same future type differently.

### 5.5 `async` blocks vs `async fn`

```rust
let fut1 = async {
    do_thing().await
};

async fn fut2() { do_thing().await }
```

Both produce `impl Future`. The block form:

- Has no function name (anonymous).
- Captures locals like a closure.
- Required to get `impl Future` as a value in statements where `async fn` syntax doesn't fit.
- `async move { .. }` forces by-move captures (most common for `tokio::spawn`).

### 5.6 Return-position `impl Trait` (RPIT) and async

```rust
fn compose() -> impl Future<Output = u32> {
    async { 42 }
}
```

- The return type is opaque. The caller only knows "some future with Output=u32".
- Requires the future to be `'static` unless you add a lifetime bound.
- Compared to `async fn compose() -> u32`: same thing, minus the explicit `Future`.
- Common use: returning a future from a non-async function (e.g. `fn build() -> impl Future<...>` in a `Service::call` pattern).

---

## 6. `Send` bounds and `!Send` futures

### 6.1 What `Send` really means here

A future is `Send` iff **every value it holds across any `.await`** is `Send`. The compiler computes this recursively. Types that make a future `!Send`:

- `Rc<T>`, `std::cell::RefCell<T>`, `std::cell::Cell<T>`.
- `MutexGuard` from `std::sync::Mutex` (yes — the `Send` bound here got subtle; `std::sync::MutexGuard<T>` is `!Send` because it implies releasing the lock on a different thread, and `pthread_mutex` semantics on some platforms forbid that).
- Any `!Send` user-defined type captured across `.await`.
- `*const T`, `*mut T`.

Values that do **not** cross an `.await` don't contribute to `Send`-ness. So you can use `Rc` freely between await points as long as it's dropped in between.

### 6.2 Diagnosing `!Send` futures

Common error:

```
error: future cannot be sent between threads safely
  --> note: future is not `Send` as this value is used across an await
  --> note: captures the following types: `Rc<Thing>`
help: consider using `Arc` instead of `Rc`
```

Debugging strategies:

1. Look at the "captures the following types" list — usually the answer.
2. Binary-search: comment out halves of the future body to isolate.
3. Extract sub-tasks into `async fn` with explicit bounds to narrow scope.

### 6.3 `Send` on `tokio::spawn`

```rust
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
```

- Future must be `Send + 'static` — runtime may move it between workers.
- Output must be `Send + 'static` — the `JoinHandle` can be `.await`ed from any worker.
- `'static` does NOT mean "lives forever" — just "borrows nothing with a shorter lifetime". `async move { .. }` that owns all its captures is `'static`.

### 6.4 Escape hatches for `!Send` — `LocalSet`

```rust
use tokio::task::LocalSet;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let local = LocalSet::new();
    local.run_until(async {
        let rc = std::rc::Rc::new(42);

        // Spawn !Send futures onto THIS THREAD via local set.
        tokio::task::spawn_local(async move {
            println!("{}", rc);
            tokio::task::yield_now().await;
            println!("{}", rc);
        }).await.unwrap();
    }).await;
}
```

`spawn_local` requires a `LocalSet` running; panics if called without one on the current thread. All local tasks run on the calling thread; no work stealing.

Use cases:
- GUI frameworks (must stay on main thread).
- FFI types that aren't `Send` (COM objects, OpenGL contexts).
- Reference-counted graphs that can't be made `Arc`.

### 6.5 `Sync` on futures

A future is `Sync` iff `&F` is `Send`. Rarely matters — you usually hold futures by value or `Pin<&mut _>`. `Sync` only becomes a consideration for futures stored in shared state.

---

## 7. Cancellation & cancellation safety

### 7.1 What "cancellation" means in async Rust

A future is cancelled simply by **dropping it** (not polling it again). Cancellation is:

- **Cooperative**: no preemption mid-poll. You can only cancel between polls.
- **Synchronous**: `Drop::drop` runs on the cancelling task. Any `Drop` impls on locals in the async body run in reverse declaration order.

No exceptions exist. You cannot catch cancellation; you can only observe it via `Drop`.

### 7.2 Cancellation safety — the concept

A future is **cancel-safe** if dropping it after partial progress loses only observable *input*, not committed state. Equivalently: it's safe to re-run a fresh copy.

Classic example — not cancel-safe:

```rust
// BAD in select!: if cancelled, bytes are lost.
sock.read_exact(&mut buf).await?
```

`read_exact` may have consumed some bytes into `buf` and be buffered in a partial-read state internally. Dropping loses those bytes.

Classic example — cancel-safe:

```rust
// Just reads what's available. Drop loses nothing not already in `buf`.
sock.read(&mut buf).await?
```

### 7.3 Tokio's cancellation-safety table (memorize this)

| API | Cancel-safe? | Notes |
|---|---|---|
| `tokio::time::sleep` | Yes | Timer is cancelled on drop. |
| `tokio::time::timeout(d, f)` | Same as `f` | Timeout wrapper is safe; inner must be. |
| `tokio::net::TcpStream::read(buf)` | Yes | At most returns one read or nothing. |
| `tokio::net::TcpStream::read_exact(buf)` | **No** | May lose partial data. |
| `tokio::net::TcpStream::read_to_end(buf)` | **No** | Partial writes into `buf` not recovered. |
| `tokio::net::TcpStream::write(buf)` | Yes | Returns how much was written. |
| `tokio::net::TcpStream::write_all(buf)` | **No** | Partial writes into socket, progress lost. |
| `tokio::sync::Mutex::lock()` | Yes | Safe to drop; no lock acquired unless guard returned. |
| `tokio::sync::Notify::notified()` | Yes | Subscription drops cleanly. |
| `tokio::sync::mpsc::Receiver::recv()` | Yes | Message remains in channel if dropped. |
| `tokio::sync::mpsc::Sender::send(v)` | **No** | Value `v` is consumed; if future dropped mid-send, value is lost. (Use `send_timeout` / `try_send` / or pre-reserve with `reserve().await`.) |
| `tokio::sync::broadcast::Receiver::recv()` | Yes | Won't lose messages delivered before drop... except via lag. |
| `tokio::sync::oneshot::Receiver::recv()` | Yes | Fine to drop. |
| `tokio::io::AsyncReadExt::read_u32()` and friends | **No** | Partial reads consumed. |
| `tokio::io::copy` / `copy_bidirectional` | **No** | Stateful buffering. |
| `tokio::signal::unix::Signal::recv()` | Yes | Signal delivery notifications. |

Rule: **if the API has "exact", "all", "to_end", "until", or buffers internally, assume not cancel-safe.**

### 7.4 The `select!` implication

`tokio::select!` cancels all losing branches by dropping their futures. Thus every branch inside `select!` **must be cancel-safe** (or you must rotate unsafe work into a sub-task).

Fix pattern for non-cancel-safe calls:

```rust
// Refactor: move the unsafe call into a sub-task so the JoinHandle becomes cancel-safe.
let write_task = tokio::spawn(async move {
    sock.write_all(&data).await
});

tokio::select! {
    res = write_task => { /* write completed */ }
    _ = shutdown.cancelled() => {
        // We dropped the JoinHandle — the inner write_all continues.
        // If you want to actually stop it: use CancellationToken and check inside.
    }
}
```

Or use `tokio::pin!` a single long-lived future and loop:

```rust
let write = sock.write_all(&data);
tokio::pin!(write);
loop {
    tokio::select! {
        res = &mut write => break res,   // re-polled, not dropped, on every loop
        _ = interval.tick() => { /* heartbeat */ }
    }
}
```

### 7.5 `CancellationToken` (tokio-util)

`tokio_util::sync::CancellationToken` is the standard "tree of cooperative cancellation":

```rust
use tokio_util::sync::CancellationToken;

let root = CancellationToken::new();
let child = root.child_token();      // cancelled when root is, OR independently

tokio::spawn(async move {
    tokio::select! {
        _ = child.cancelled() => {
            println!("cancelled");
        }
        _ = do_work() => {
            println!("done");
        }
    }
});

// Later:
root.cancel();     // cancels root and all descendants
```

Properties:
- `cancelled()` is cancel-safe and idempotent.
- Cloning the token is cheap (`Arc` bump).
- `.is_cancelled()` for polling rather than awaiting.
- `.cancel_and_wait_for_tasks()` pattern: pair with `JoinSet` (see Graceful Shutdown §14).
- `.drop_guard()` returns a guard that cancels on drop — handy for RAII.

---

## 8. `select!`, `join!`, `try_join!`, `FuturesUnordered`

### 8.1 `tokio::select!`

```rust
tokio::select! {
    biased;                                // optional: try branches in declaration order
    res = fut1 => { /* ... */ }
    Some(x) = rx.recv(), if !rx.is_closed() => { /* guard condition */ }
    _ = tokio::time::sleep(Duration::from_secs(1)) => { /* timeout */ }
    else => { /* all branches disabled */ }
}
```

Semantics:

- Polls all branches in a pseudo-random order (unless `biased;`), once each.
- First to return `Ready` wins; its value flows through the arrow.
- All losing branches are **dropped** (see cancel-safety §7).
- `if <guard>` disables a branch entirely — it's not even polled.
- `else =>` runs if every branch's pattern match fails or is disabled.

`biased;` use cases:
- Deterministic shutdown ordering (check cancellation first).
- Fairness-sensitive tests.
- Pre-emptible loops where you always want to drain a channel before doing more work.

### 8.2 `tokio::select!` patterns

**Timeout + work**:
```rust
tokio::select! {
    r = do_work() => Ok(r),
    _ = tokio::time::sleep(timeout) => Err(Elapsed),
}
```

(Or use `tokio::time::timeout(timeout, do_work()).await`.)

**Shutdown-aware loop**:
```rust
loop {
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => break,
        msg = rx.recv() => { process(msg).await; }
    }
}
```

**Multi-channel fan-in**:
```rust
loop {
    tokio::select! {
        Some(msg) = rx_a.recv() => handle_a(msg),
        Some(msg) = rx_b.recv() => handle_b(msg),
        else => break,                    // all channels closed
    }
}
```

### 8.3 `tokio::join!`

```rust
let (a, b, c) = tokio::join!(fut_a, fut_b, fut_c);
```

- Polls all futures concurrently **on the same task** (no spawning).
- Waits for all to complete.
- Returns a tuple of outputs.
- Error in one does **not** short-circuit the others.

Vs. `spawn`: `join!` does not parallelize across threads; use it for I/O concurrency without task overhead.

### 8.4 `tokio::try_join!`

```rust
let (a, b, c) = tokio::try_join!(fut_a, fut_b, fut_c)?;
```

- Each future returns `Result<T, E>` with a common `E`.
- Short-circuits on first `Err`; other futures are dropped (cancel-safety applies).
- Returns `Result<(Ta, Tb, Tc), E>`.

### 8.5 `futures::future::join_all` / `try_join_all`

```rust
use futures::future::{join_all, try_join_all};

let results: Vec<T> = join_all(vec_of_futures).await;
let results: Result<Vec<T>, E> = try_join_all(vec_of_result_futures).await?;
```

- Good for runtime-sized sets.
- All futures in memory simultaneously — watch fat-future memory.

### 8.6 `FuturesUnordered` — streaming completions

```rust
use futures::stream::{FuturesUnordered, StreamExt};

let mut in_flight: FuturesUnordered<_> = (0..100)
    .map(|i| fetch(i))
    .collect();

while let Some(result) = in_flight.next().await {
    // Process each as soon as it finishes, not in submission order.
}
```

Properties:
- Completion order, not insertion order.
- Add more tasks dynamically: `in_flight.push(new_fut)`.
- Used heavily inside server routers, batch processors, crawlers.
- **Caveat**: if you never `.next().await`, nothing gets polled — this is the #1 beginner mistake.
- `StreamExt::buffer_unordered(n)` is the capped version built on top.

### 8.7 `JoinSet` (Tokio) — spawn + collect + cancel

```rust
use tokio::task::JoinSet;

let mut set = JoinSet::new();
for i in 0..10 {
    set.spawn(async move { fetch(i).await });
}

while let Some(res) = set.join_next().await {
    match res {
        Ok(val) => { /* value */ }
        Err(e) if e.is_cancelled() => { /* was aborted */ }
        Err(e) if e.is_panic() => { /* task panicked */ }
        _ => {}
    }
}

// Or: set.shutdown().await — aborts all and drains.
```

Vs `FuturesUnordered`: `JoinSet` spawns onto the runtime (parallel if multi-thread), tracks `JoinHandle`s, supports `abort_all()`. `FuturesUnordered` polls in the current task.

---

## 9. Streams (`Stream`, `StreamExt`)

### 9.1 The `Stream` trait

```rust
// futures_core::stream
pub trait Stream {
    type Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;
    fn size_hint(&self) -> (usize, Option<usize>) { (0, None) }
}
```

- Like `Future<Output = Option<Item>>` that you poll repeatedly.
- `None` = stream exhausted. After `None`, calling `poll_next` again is **allowed to panic** (fuse with `.fuse()` to make it safe).
- As of writing `Stream` is in `futures_core`, re-exported from `futures`, and used by `tokio_stream`. There is a stable-`std`-`Stream` RFC in progress but it has not landed; use the ecosystem trait.

### 9.2 `StreamExt` idioms

```rust
use tokio_stream::StreamExt;  // or futures::StreamExt

// Map / filter / collect
let out: Vec<u32> = stream.map(|x| x * 2).filter(|&x| x > 10).collect().await;

// Concurrency-limited fan-out
stream.for_each_concurrent(16, |item| async move {
    process(item).await;
}).await;

// Buffered parallelism
stream
    .map(|item| async move { process(item).await })
    .buffered(8)                         // keep 8 in flight, yield in order
    .collect::<Vec<_>>()
    .await;

// Unordered variant — emit as completed
stream.map(|i| fetch(i)).buffer_unordered(16).collect::<Vec<_>>().await;

// Timeouts, chunking
stream.timeout(Duration::from_secs(1));
stream.chunks_timeout(100, Duration::from_millis(50));  // ⟵ tokio_stream only

// Take / skip / throttle
stream.take(5);
stream.throttle(Duration::from_millis(100));            // ⟵ tokio_stream only
```

### 9.3 Creating streams

```rust
// From iterator
let s = tokio_stream::iter(vec![1,2,3]);

// From repeated polling
let s = tokio_stream::unfold(0u64, |state| async move {
    if state > 5 { None } else { Some((state, state + 1)) }
});

// From channel receiver
let mut rx = tokio_stream::wrappers::ReceiverStream::new(rx);

// async-stream (macro-based)
use async_stream::stream;
let s = stream! {
    for i in 0..10 {
        yield i;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
};
tokio::pin!(s);
while let Some(item) = s.next().await {
    println!("{}", item);
}
```

`async_stream::stream!` is the easiest way to hand-write complex stateful streams with borrows — it compiles down to a generator-like state machine.

### 9.4 Stream-from-stream patterns

- **Backpressure**: the pull-based model gives natural backpressure — consumer pace dictates producer pace.
- **Pipeline**: chain `.map(async)` + `.buffered(n)` for bounded-parallel pipelines.
- **Fan-out + merge**: `StreamExt::merge(a, b)` or `tokio_stream::StreamMap` for many streams keyed by id.

---

## 10. Tokio runtime internals

### 10.1 Scheduler architecture (multi-thread)

```
 Global Injection Queue (unbounded, lock-protected Inject<Task>)
    │
    ▼
 Worker 0            Worker 1            Worker 2
 ┌───────────┐       ┌───────────┐       ┌───────────┐
 │ LIFO slot │       │ LIFO slot │       │ LIFO slot │
 ├───────────┤       ├───────────┤       ├───────────┤
 │ Local ring│◄──────│  STEAL    │─────► │ Local ring│
 │  (256)    │       │           │       │  (256)    │
 └───────────┘       └───────────┘       └───────────┘
     │                   │                   │
     ▼                   ▼                   ▼
  I/O Driver (mio)    Time Driver
```

Scheduling rules:

1. Poll LIFO slot (if any).
2. Pop from local ring.
3. Every `global_queue_interval` ticks, poll global queue instead.
4. Every `event_interval` ticks, poll I/O driver.
5. If all empty, try to steal (half) from another worker's ring.
6. If nothing to steal, park; wake on new task or I/O event.

### 10.2 Task structure

Each task is a heap-allocated:

```
┌──────────────────┐
│ Header (refcount,│  ← atomic ops for wake/drop
│  state, vtable)  │
├──────────────────┤
│ Scheduler handle │
├──────────────────┤
│ Future           │  ← your async state machine
├──────────────────┤
│ Output slot /    │
│ JoinHandle link  │
└──────────────────┘
```

One allocation per task. `tokio::spawn`'s overhead is dominated by that allocation + refcount setup. Tasks run on whichever worker wakes them.

### 10.3 Budgets and cooperative yielding

Tokio 0.3+ added **coop budgeting**:

- Every task starts a poll with ≈128 "coop budget" units.
- Most I/O ops deduct 1.
- When budget hits 0, a Tokio-aware call returns `Poll::Pending` even if data is ready, forcing the task to yield.

Effect: tight loops over a single socket can't monopolize a worker. Mandatory for fairness.

Caveat: custom futures that don't use Tokio's coop-aware wrappers can bypass this. Insert `tokio::task::yield_now().await` in CPU-dense loops.

### 10.4 Time driver

- Hierarchical timing wheel (6 levels × 64 slots default).
- O(1) insert / cancel of timers.
- Minimum resolution: 1 ms (configurable via `start_paused` / Tokio test).
- `tokio::time::Interval::tick()` vs `sleep(period)` in loop:
  - `Interval` accounts for time already elapsed — catches up on missed ticks (configurable via `MissedTickBehavior::{Burst, Delay, Skip}`).
  - `sleep` in a loop drifts (slower than period by accumulated overhead).

```rust
let mut interval = tokio::time::interval(Duration::from_millis(100));
interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
loop {
    interval.tick().await;
    do_work().await;    // if this takes > 100ms, we skip not burst
}
```

### 10.5 `tokio::main` macro

```rust
#[tokio::main]
async fn main() { /* ... */ }
// expands (roughly) to:
fn main() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { /* ... */ })
}
```

Variants:
- `#[tokio::main(flavor = "current_thread")]` — single-threaded runtime.
- `#[tokio::main(flavor = "multi_thread", worker_threads = 4)]` — pinned worker count.
- `#[tokio::test]` — test-scoped runtime. Each test gets its own runtime.
- `#[tokio::test(flavor = "current_thread", start_paused = true)]` — time auto-advances in tests, skipping `sleep` instantly.

### 10.6 Entering a runtime from sync code

```rust
let rt = tokio::runtime::Runtime::new()?;
// Option 1: block until done
let v = rt.block_on(async { do_stuff().await });
// Option 2: spawn, keep running
let handle = rt.handle().clone();
std::thread::spawn(move || {
    handle.spawn(async { /* inside tokio */ });
});
// Option 3: enter scope
let _guard = rt.enter();
tokio::spawn(async { /* OK because we're inside runtime */ });
```

`enter()` returns an `EnterGuard`. As long as it lives, `tokio::spawn` and `Handle::current()` work on this thread. Essential for bridging with non-async libraries that need to call Tokio APIs on init.

---

## 11. `tokio::spawn`, `spawn_blocking`, `block_in_place`, `LocalSet`

### 11.1 `tokio::spawn`

```rust
let handle: tokio::task::JoinHandle<T> = tokio::spawn(async {
    /* Send + 'static future */
    compute().await
});

match handle.await {
    Ok(v) => /* task returned v */,
    Err(e) if e.is_panic() => /* task panicked; get payload via e.into_panic() */,
    Err(e) if e.is_cancelled() => /* aborted */,
    Err(_) => unreachable!(),
}
```

- Returns immediately; the task runs on the runtime's workers.
- `JoinHandle` can be dropped — the task continues detached.
- `JoinHandle::abort()` requests cancellation (drops the future at its next poll).
- `JoinHandle::abort_handle()` gives an `AbortHandle` — cheap-clone token to abort later without keeping the `JoinHandle`.

### 11.2 `spawn_blocking` — for CPU/IO that blocks

```rust
let data = tokio::task::spawn_blocking(move || {
    std::fs::read("big.file").unwrap()          // sync file I/O
}).await?;
```

- Runs on the **blocking thread pool**, separate from workers.
- Default pool size: 512 (configurable via `max_blocking_threads`).
- Thread is created lazily; idle threads die after `thread_keep_alive` (default 10s).
- The closure is `FnOnce() -> R + Send + 'static`.
- Cannot be cancelled once started (the thread runs it to completion). `JoinHandle::abort()` only detaches — the work still happens.

Use for:
- `std::fs`, `std::process::Command::output`, CPU-heavy work.
- Calls to C libraries that block.
- `sqlite` synchronous connections (or use an async wrapper).

Anti-use:
- Never use it as "quick escape hatch" inside a tight loop — you'll exhaust the pool and block on thread spawns.

### 11.3 `block_in_place`

```rust
tokio::task::block_in_place(|| {
    // Executed on the current worker thread, but the worker's other tasks
    // are moved to siblings (work-stealing).
    expensive_cpu_work();
});
```

- **Only works on multi-thread runtime.** Panics on current-thread.
- Tells the scheduler "this thread will be busy; migrate my other work."
- Useful inside an async function when you occasionally need a short blocking call and don't want the `spawn_blocking` allocation.
- Still blocks the scheduler's ability to make *this worker* available for I/O polling — use sparingly.

### 11.4 `LocalSet` + `spawn_local`

Covered in §6.4. Extra notes:

- `LocalSet::run_until(fut)` — drives `fut` AND any local tasks until `fut` completes.
- `LocalSet` is itself a `Future` — you can `.await` it on a current-thread runtime until all local tasks finish.
- Mixing: `tokio::spawn` still works inside a `LocalSet` block (goes to the normal scheduler); `spawn_local` stays on the current thread.

### 11.5 Summary table

| API | Runs on | Future/Fn? | Use for |
|---|---|---|---|
| `tokio::spawn` | Worker pool | `Future + Send + 'static` | All normal async work |
| `tokio::task::spawn_blocking` | Blocking pool | `FnOnce -> R + Send + 'static` | Sync I/O, CPU hot path |
| `tokio::task::block_in_place` | Current worker (moved) | `FnOnce -> R` | Short blocking call in async fn |
| `tokio::task::spawn_local` (under `LocalSet`) | Current thread | `Future + 'static` (no `Send`) | `!Send` futures |
| `std::thread::spawn` | New OS thread | `FnOnce -> R + Send + 'static` | Truly independent thread, owned resources |

---

## 12. `tokio::sync` primitives

### 12.1 When to use which

Decision tree:

```
Sharing data between tasks?
├── Read-only for lifetime? → Arc<T> + no sync
├── Small, Copy, fast-changing? → Arc<AtomicXxx>
├── Guard never crosses .await? → Arc<std::sync::Mutex<T>>  (faster)
├── Guard crosses .await?       → Arc<tokio::sync::Mutex<T>>
├── Read-heavy, occasional write? → Arc<tokio::sync::RwLock<T>>
├── Just signal "event happened"? → tokio::sync::Notify
└── Limit concurrent X?          → tokio::sync::Semaphore

Passing messages?
├── One producer, one consumer, one-shot? → tokio::sync::oneshot
├── Many producers, one consumer?         → tokio::sync::mpsc
├── Many producers, many consumers, fanout? → tokio::sync::broadcast
└── One producer, many consumers, latest?   → tokio::sync::watch
```

### 12.2 `tokio::sync::Mutex`

```rust
use tokio::sync::Mutex;
use std::sync::Arc;

let m = Arc::new(Mutex::new(0u64));
let m2 = m.clone();
tokio::spawn(async move {
    let mut g = m2.lock().await;
    *g += 1;
    other_future().await;      // OK to hold across .await — guard is Send.
});
```

- `lock()` is async, waits if contended.
- `try_lock()` is sync, returns `TryLockError` on contention.
- Guard (`MutexGuard<'_, T>`) is `Send` (unlike `std`).
- No poisoning — if a task panics holding the lock, the next waiter gets it cleanly.
- **Slower than `std::sync::Mutex`**. Benchmarks suggest 2–4× overhead because of async machinery. Prefer `std` if the critical section doesn't `.await`.
- `OwnedMutexGuard`: `lock_owned()` — takes `Arc<Mutex<T>>`, yields a `'static` guard. Needed for spawning tasks that own the guard.

### 12.3 `tokio::sync::RwLock`

```rust
use tokio::sync::RwLock;

let cache = Arc::new(RwLock::new(HashMap::new()));
let r = cache.read().await;              // many readers at once
drop(r);
let mut w = cache.write().await;         // exclusive
w.insert("k", "v");
```

- Write-preferring (once a writer is queued, new readers wait).
- Writer starvation avoidable.
- Slower than parking-lot `RwLock`; only use when guards cross `.await`.
- `OwnedRwLockReadGuard` / `OwnedRwLockWriteGuard` are the `Arc`-holding variants.

### 12.4 `tokio::sync::Notify`

Zero-cost "wake one / wake all" without carrying data.

```rust
use tokio::sync::Notify;

let notify = Arc::new(Notify::new());
let n2 = notify.clone();

tokio::spawn(async move {
    n2.notified().await;                 // waits for a notification
    println!("wake!");
});

tokio::time::sleep(Duration::from_millis(100)).await;
notify.notify_one();                     // wakes one waiter (or records permit)
// notify.notify_waiters();              // wakes ALL current waiters; no permit
```

Subtleties:

- `notify_one` records a *permit* if no waiter — next `notified()` returns immediately.
- `notify_waiters` does NOT record a permit; only already-registered waiters get woken.
- The classic race: "register before missing the wake" — use the pattern:
  ```rust
  let fut = notify.notified();     // register
  tokio::pin!(fut);
  if check_condition() { return; }
  fut.as_mut().await;               // now safe from race
  ```
  Or use `Notified::enable()` explicitly (newer API).

### 12.5 `tokio::sync::Semaphore`

Counted permits.

```rust
use tokio::sync::Semaphore;

let sem = Arc::new(Semaphore::new(10));  // 10 concurrent "slots"
let permit = sem.clone().acquire_owned().await.unwrap();   // hold an owned permit
process().await;
drop(permit);                             // releases slot

// Rate-limiting pattern:
let sem2 = sem.clone();
tokio::spawn(async move {
    let _p = sem2.acquire().await.unwrap();
    do_bounded_work().await;
});
```

- `acquire()` / `try_acquire()` / `acquire_many(n)` / `acquire_owned()`.
- **Close a semaphore** to reject future acquirers: `sem.close()` — `acquire()` returns `AcquireError`.
- Used extensively inside Tokio for bounding things (`max_blocking_threads` is really a semaphore).

### 12.6 `tokio::sync::mpsc`

Many producers → one consumer, bounded/unbounded.

```rust
use tokio::sync::mpsc;

let (tx, mut rx) = mpsc::channel::<Msg>(32);   // bounded capacity 32

tokio::spawn(async move {
    while let Some(msg) = rx.recv().await {
        handle(msg).await;
    }
});

tx.send(msg).await?;    // backpressure: awaits if full
```

- Bounded `mpsc::channel(n)` provides backpressure. Unbounded `mpsc::unbounded_channel()` does not — use with caution.
- `Sender::send` is **not cancel-safe**: if you drop the future mid-send, `msg` is lost. Mitigations:
  - `Sender::try_send(msg)` (sync, `Result<(), TrySendError<T>>`).
  - `Sender::reserve().await` → returns a `Permit`; then `permit.send(msg)` is sync, can't lose the message.
  - `Sender::send_timeout(msg, dur)`.
- `rx.recv()` is cancel-safe.
- Closing: all `Sender`s dropped → `recv()` returns `None` after draining.

### 12.7 `tokio::sync::oneshot`

One-time transfer.

```rust
use tokio::sync::oneshot;

let (tx, rx) = oneshot::channel::<u64>();
tokio::spawn(async move {
    let result = expensive().await;
    let _ = tx.send(result);                // send takes self
});
match rx.await {
    Ok(v) => println!("{}", v),
    Err(_) => println!("sender dropped"),   // never got a value
}
```

- Sender is consumed on `send`.
- Receiver implements `Future`.
- Must-have for "spawn a task and get its one result back" when you don't want a `JoinHandle`.

### 12.8 `tokio::sync::broadcast`

SPMC-ish with bounded buffer.

```rust
let (tx, _) = tokio::sync::broadcast::channel::<Event>(16);
let mut rx1 = tx.subscribe();
let mut rx2 = tx.subscribe();

tx.send(Event::Foo).unwrap();

match rx1.recv().await {
    Ok(ev) => { /* got event */ }
    Err(broadcast::error::RecvError::Lagged(n)) => { /* we missed n events; ring-buffered */ }
    Err(broadcast::error::RecvError::Closed) => { /* sender dropped */ }
}
```

- Fixed-size ring buffer; slow consumers observe `Lagged(n)` and resume from latest retained.
- Each subscriber has an independent read cursor.
- Cheap per-subscriber cost; good for shutdown signalling, pub/sub events.

### 12.9 `tokio::sync::watch`

"Latest value" channel — single slot overwritten each send.

```rust
let (tx, mut rx) = tokio::sync::watch::channel("initial".to_string());

tokio::spawn(async move {
    while rx.changed().await.is_ok() {
        println!("current: {}", *rx.borrow_and_update());
    }
});

tx.send("second".into())?;
tx.send("third".into())?;    // intermediate values can be skipped if consumer is slow
```

- Sender count unlimited; receivers borrow the latest value.
- `rx.changed()` is cancel-safe; returns when sender updates (not on every value — just "something changed since last borrow").
- Perfect for config reload, shutdown-flag-with-context, current-leader.

### 12.10 `tokio::sync::SetOnce` / `OnceCell`

Async one-time init (stable in `tokio` itself):

```rust
use tokio::sync::OnceCell;

static CLIENT: OnceCell<reqwest::Client> = OnceCell::const_new();

async fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| async { reqwest::Client::new() }).await
}
```

### 12.11 Backpressure philosophy

Bounded channels = backpressure surface. An unbounded channel hides problems until you OOM. Default to bounded; size based on memory × worst-case message size. Start at 32 or 64 and tune under load.

---

## 13. I/O, framing, and codecs

### 13.1 `AsyncRead` / `AsyncWrite`

```rust
// tokio::io
pub trait AsyncRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>>;
}
pub trait AsyncWrite {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>>;
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
    fn poll_write_vectored(...)-> Poll<io::Result<usize>> { /* default: fallback */ }
    fn is_write_vectored(&self) -> bool { false }
}
```

Key differences from `std::io::Read/Write`:

- `ReadBuf` — uninit-aware buffer. Avoids zeroing before each read.
- Write returns `Poll<Result<usize>>`, not just `Result<usize>` — may buffer until `poll_flush`.
- `poll_shutdown` is distinct from drop — explicit half-close signal.

### 13.2 `AsyncReadExt` / `AsyncWriteExt`

Extension traits layer the familiar `.read()`, `.read_exact()`, `.write_all()`, `.read_to_end()` on top.

Cancel-safety table above (§7.3) applies.

### 13.3 Framed I/O — `tokio_util::codec`

Turn a byte stream into a stream of frames.

```rust
use tokio_util::codec::{Framed, LinesCodec};
use futures::{SinkExt, StreamExt};

let mut framed = Framed::new(tcp_stream, LinesCodec::new_with_max_length(65_536));

while let Some(line) = framed.next().await {
    let line = line?;
    framed.send("ACK".to_string()).await?;
}
```

Codecs are `Encoder` + `Decoder`:

```rust
pub trait Decoder {
    type Item;
    type Error: From<io::Error>;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error>;
    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> { .. }
}
pub trait Encoder<Item> {
    type Error: From<io::Error>;
    fn encode(&mut self, item: Item, dst: &mut BytesMut) -> Result<(), Self::Error>;
}
```

Built-in codecs: `LinesCodec`, `LengthDelimitedCodec` (length-prefixed frames), `BytesCodec`.

### 13.4 Split / join

```rust
let (r, w) = tokio::io::split(socket);               // for any AsyncRead+AsyncWrite
// Or, for TcpStream:
let (r, w) = tcp_stream.into_split();                // owned halves, no locking
```

`split` uses an internal lock; `into_split` is true ownership and is cheaper. Prefer `into_split` when available.

### 13.5 Copy / pipe

```rust
tokio::io::copy(&mut reader, &mut writer).await?;
tokio::io::copy_bidirectional(&mut a, &mut b).await?;     // useful for proxies
```

Both **not cancel-safe** — state lives inside. Put them in a dedicated task.

---

## 14. Graceful shutdown patterns

### 14.1 The canonical multi-component shutdown

```rust
use tokio_util::sync::CancellationToken;
use tokio::task::JoinSet;

struct App {
    shutdown: CancellationToken,
    tasks: JoinSet<()>,
}

impl App {
    fn new() -> Self {
        Self { shutdown: CancellationToken::new(), tasks: JoinSet::new() }
    }

    fn spawn<F: Future<Output=()> + Send + 'static>(&mut self, f: impl FnOnce(CancellationToken) -> F) {
        let child = self.shutdown.child_token();
        self.tasks.spawn(f(child));
    }

    async fn run(&mut self) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { /* SIGINT */ }
            _ = wait_for_sigterm() => { /* SIGTERM */ }
            _ = self.any_task_exits() => { /* early crash */ }
        }

        self.shutdown.cancel();
        // Drain with timeout
        tokio::time::timeout(Duration::from_secs(30), self.drain())
            .await
            .unwrap_or_else(|_| {
                eprintln!("shutdown timed out; aborting");
                self.tasks.abort_all();
            });
    }

    async fn drain(&mut self) {
        while self.tasks.join_next().await.is_some() {}
    }
}
```

### 14.2 `watch` channel as shutdown flag

Lighter weight than `CancellationToken` for simple cases:

```rust
let (tx, mut rx) = tokio::sync::watch::channel(false);

let worker = tokio::spawn({
    let mut rx = rx.clone();
    async move {
        loop {
            tokio::select! {
                biased;
                _ = rx.changed() => {
                    if *rx.borrow() { break; }
                }
                _ = work_tick() => {}
            }
        }
    }
});

// Shutdown
let _ = tx.send(true);
worker.await.unwrap();
```

### 14.3 Handling `SIGTERM`/`SIGINT`

```rust
#[cfg(unix)]
async fn wait_for_termination() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let mut sigint  = signal(SignalKind::interrupt()).unwrap();
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
    }
}
#[cfg(windows)]
async fn wait_for_termination() {
    tokio::signal::ctrl_c().await.unwrap();
}
```

### 14.4 Three-tier shutdown policy

1. **Stop accepting** — close listener; inflight requests continue.
2. **Drain** — signal workers to finish current work; don't start new work.
3. **Deadline abort** — if drain exceeds budget, call `abort_all()`.

Always log which phase timed out — helps diagnose stuck tasks.

---

## 15. Tracing & observability

### 15.1 `tracing` fundamentals

`tracing` is the async-first structured logging crate. Unlike `log`, spans survive `.await` (attached to the task).

```rust
use tracing::{info, warn, error, instrument, Instrument};

#[instrument(skip(client))]
async fn fetch(client: &Client, id: u64) -> Result<Data, Error> {
    info!(id, "starting fetch");
    let res = client.get(id).await?;
    info!(size = res.size(), "fetched");
    Ok(res)
}

// Subscriber setup
tracing_subscriber::fmt()
    .with_env_filter("info,my_crate=debug")
    .with_target(false)
    .init();
```

- `#[instrument]` attaches a span to an async fn; fields auto-captured.
- `skip(..)` excludes large/non-Debug args.
- `Instrument::instrument(span)` manually attaches a span to a future — required for `tokio::spawn` tasks, which otherwise inherit nothing:
  ```rust
  let span = tracing::info_span!("worker", id = 42);
  tokio::spawn(async move { /* ... */ }.instrument(span));
  ```

### 15.2 `tokio-console`

`#[tokio::main]` with `tokio_unstable` gives you task-level instrumentation:

```rust
// Cargo.toml
// tokio = { version = "1", features = ["full", "tracing"] }
// RUSTFLAGS="--cfg tokio_unstable"

console_subscriber::init();
```

- Shows all tasks, their state, poll duration histograms, self-wakes, resource contention.
- Essential for diagnosing "my task never wakes" or "my worker is at 100% CPU polling".

---

## 16. Actor pattern in Tokio

### 16.1 The canonical actor

```rust
enum Msg {
    Get { key: String, resp: oneshot::Sender<Option<String>> },
    Set { key: String, val: String, resp: oneshot::Sender<()> },
    Shutdown,
}

struct Store { map: HashMap<String, String>, rx: mpsc::Receiver<Msg> }

impl Store {
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                Msg::Get { key, resp } => { let _ = resp.send(self.map.get(&key).cloned()); }
                Msg::Set { key, val, resp } => { self.map.insert(key, val); let _ = resp.send(()); }
                Msg::Shutdown => break,
            }
        }
    }
}

#[derive(Clone)]
struct StoreHandle { tx: mpsc::Sender<Msg> }

impl StoreHandle {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(64);
        let actor = Store { map: HashMap::new(), rx };
        tokio::spawn(actor.run());
        Self { tx }
    }
    async fn get(&self, k: &str) -> Option<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let _ = self.tx.send(Msg::Get { key: k.into(), resp: resp_tx }).await;
        resp_rx.await.unwrap_or(None)
    }
    async fn set(&self, k: String, v: String) {
        let (resp_tx, resp_rx) = oneshot::channel();
        let _ = self.tx.send(Msg::Set { key: k, val: v, resp: resp_tx }).await;
        let _ = resp_rx.await;
    }
}
```

Why actors over `Arc<Mutex<T>>`:
- No lock acquisition cost on hot path (messaging overhead instead).
- Natural boundary for `!Send` state (run on its own thread).
- Easier testing — replace handle with a fake.
- Messages order is sequential inside the actor — easier to reason about invariants.

### 16.2 `oneshot` for replies

Every request-with-reply uses a `oneshot::channel` as the reply slot. Send the `Sender` with the request; await the `Receiver` for the response.

### 16.3 Supervision-light patterns

- Monitor actor with a `JoinHandle`; on panic/exit, restart with fresh state and log.
- Use a `broadcast` channel for events ("member joined", "config changed") that multiple actors consume.
- Avoid **actor cycles** (A sends to B, B sends to A) with full channels — bounded channels can deadlock.

### 16.4 When NOT to actor-ize

- Single-writer data structure with no outside messages — just use `Arc<Mutex<T>>` or `tokio::sync::Mutex`.
- Hot paths where the message alloc / channel overhead dominates. Benchmark.

---

## 17. Design patterns catalogue

### 17.1 Select-loop with shutdown

```rust
loop {
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => break,
        Some(msg) = rx.recv() => handle(msg).await,
        _ = heartbeat.tick() => send_heartbeat().await?,
    }
}
```

### 17.2 Bounded fan-out

```rust
use tokio::sync::Semaphore;

let sem = Arc::new(Semaphore::new(50));
let mut set = JoinSet::new();
for item in items {
    let sem = sem.clone();
    set.spawn(async move {
        let _permit = sem.acquire_owned().await.unwrap();
        process(item).await
    });
}
while let Some(r) = set.join_next().await { /* collect */ }
```

### 17.3 Retry with backoff

```rust
use std::time::Duration;

async fn with_retry<F, Fut, T, E>(mut f: F, attempts: u32) -> Result<T, E>
where F: FnMut() -> Fut, Fut: Future<Output = Result<T, E>>
{
    let mut delay = Duration::from_millis(50);
    for i in 0..attempts {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if i + 1 == attempts => return Err(e),
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
    unreachable!()
}
```

### 17.4 Timeout + fallback

```rust
match tokio::time::timeout(Duration::from_secs(1), primary()).await {
    Ok(Ok(v)) => Ok(v),
    Ok(Err(e)) => Err(e),
    Err(_elapsed) => fallback().await,
}
```

### 17.5 Debouncing

```rust
use tokio::time::{interval, MissedTickBehavior, Instant};

async fn debounce(mut rx: mpsc::Receiver<T>, dur: Duration) -> Option<T> {
    let mut latest = None;
    let deadline = tokio::time::sleep(dur);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            Some(v) = rx.recv() => {
                latest = Some(v);
                deadline.as_mut().reset(Instant::now() + dur);
            }
            _ = &mut deadline => return latest,
        }
    }
}
```

### 17.6 Token-bucket rate limit

```rust
// Refills `capacity` tokens per `period`.
struct TokenBucket { permits: Arc<Semaphore>, _refill: tokio::task::JoinHandle<()> }

impl TokenBucket {
    fn new(capacity: usize, period: Duration) -> Self {
        let permits = Arc::new(Semaphore::new(capacity));
        let p2 = permits.clone();
        let refill = tokio::spawn(async move {
            let mut iv = tokio::time::interval(period);
            iv.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                iv.tick().await;
                // Add tokens up to capacity (don't exceed).
                let available = p2.available_permits();
                if available < capacity {
                    p2.add_permits(capacity - available);
                }
            }
        });
        Self { permits, _refill: refill }
    }
    async fn take(&self) { let _p = self.permits.acquire().await.unwrap().forget(); }
}
```

### 17.7 Worker pool via channel

```rust
let (tx, rx) = async_channel::bounded::<Task>(1024);       // async_channel is mpmc
let rx = std::sync::Arc::new(tokio::sync::Mutex::new(rx)); // or use async_channel's mpmc directly

for _ in 0..workers {
    let rx = rx.clone();
    tokio::spawn(async move {
        while let Ok(task) = rx.recv().await { run(task).await; }
    });
}
```

Better: `async-channel` (from smol ecosystem) is natively MPMC and faster than a `Mutex<Receiver>` wrapper.

### 17.8 Request coalescing / single-flight

`tokio::sync::Notify` or crate `singleflight`:

```rust
use tokio::sync::{Mutex, Notify};
use std::collections::HashMap;

struct Coalesce<K, V> {
    state: Mutex<HashMap<K, Arc<Notify>>>,
    results: Mutex<HashMap<K, V>>,
}

// Caller: if key already being fetched, wait on Notify instead of re-firing.
```

---

## 18. Anti-patterns

Each labeled with why it's bad and what to do instead.

### 18.1 Blocking in async context

```rust
// BAD
async fn handle() {
    std::thread::sleep(Duration::from_secs(1));       // stops the worker
    std::fs::read("f").unwrap();                       // stops the worker
    let mut cpu = 0u64; for i in 0..100_000_000 { cpu += i; }  // starves all tasks
}
```

Fixes:
- `tokio::time::sleep` instead of `thread::sleep`.
- `tokio::fs::read` or `spawn_blocking(|| std::fs::read(..))`.
- For CPU: `spawn_blocking` or `rayon::spawn` with a oneshot back.

### 18.2 Holding `std::sync::Mutex` across `.await`

```rust
// BAD
async fn inc(m: &std::sync::Mutex<u64>) {
    let mut g = m.lock().unwrap();
    http_call().await;          // !Send future, also blocks worker if contended
    *g += 1;
}

// GOOD (option A — narrow the critical section)
async fn inc(m: &std::sync::Mutex<u64>) {
    let pre = { let g = m.lock().unwrap(); *g };
    let post = http_call(pre).await;
    { let mut g = m.lock().unwrap(); *g = post; }
}

// GOOD (option B — async mutex)
async fn inc(m: &tokio::sync::Mutex<u64>) {
    let mut g = m.lock().await;
    http_call().await;           // fine, but serialized across contenders
    *g += 1;
}
```

Rule: if the critical section contains `.await`, use `tokio::sync::Mutex`. Otherwise use `std::sync::Mutex`.

### 18.3 Spawning without tracking

```rust
// BAD: fire-and-forget — no path to detect panics, no shutdown
for task in work { tokio::spawn(run(task)); }
```

Tasks that panic with default config just log a warning (from the unwind handler) and die quietly. If you need reliability:

- Use `JoinSet` to collect handles and surface panics.
- Attach a `#[instrument]` span so traces carry context.
- Install a panic hook that hard-exits or reports to Sentry/etc.

### 18.4 Unbounded channels for untrusted producers

```rust
// BAD: each client can OOM you
let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
```

Use a bounded channel and intentionally slow producers via backpressure. If the system must *never* block, add a drop policy:

```rust
match tx.try_send(msg) {
    Ok(()) => {}
    Err(TrySendError::Full(_)) => metrics::counter!("dropped").increment(1),
    Err(TrySendError::Closed(_)) => shutdown_producer(),
}
```

### 18.5 Fat futures

Every level of `async fn` nesting enlarges the state machine. Deeply-nested `select!` branches or large stack locals (`[u8; 65536]`) blow up task size.

```rust
// BAD: 64KB of buffer in every task
async fn handler() {
    let mut buf = [0u8; 65_536];
    sock.read(&mut buf).await;
}
```

Fix:
- `Box::pin` big sub-futures.
- Move large buffers behind a `Vec` (heap-allocated) or a pool (`bytes::BytesMut` with reuse).
- Split deeply-nested async fns into smaller ones; each can be `Box::pin`-ed if needed.

Diagnose with `RUSTFLAGS="-Zprint-type-sizes"` (nightly) or `cargo +nightly rustc -- -Zprint-type-sizes | sort -k2 -n -r`.

### 18.6 Mixing runtimes

Never call `futures::executor::block_on(some_tokio_future)` from inside Tokio — different wakers, panics or hangs.
Inside Tokio, use `tokio::runtime::Handle::block_on` only from a `spawn_blocking` context.

### 18.7 `.await`-free hot loops

```rust
// BAD: never yields; starves workers
async fn crunch(data: &[u8]) -> u64 {
    let mut acc = 0u64;
    for b in data.iter() { acc = acc.wrapping_add(*b as u64); }
    acc
}
```

If data is big: `tokio::task::yield_now().await` periodically, or `spawn_blocking`.

### 18.8 `Arc<Mutex<Vec<Future>>>` antipattern

Trying to share a pool of futures under a lock is almost always wrong. Use `FuturesUnordered` (single-task access) or `JoinSet` (scheduler-owned).

### 18.9 Running CPU-bound on the async runtime

The async runtime is optimized for being almost always idle (awaiting I/O). CPU-bound work should run in a dedicated thread pool (`rayon`) or via `spawn_blocking`. Mixing them ruins tail latency.

### 18.10 Ignoring `JoinError`

```rust
// BAD
let _ = handle.await;              // silently discards panics + cancel
```

A `JoinError` says the task either panicked or was aborted. Both need action.

---

## 19. Performance tuning

### 19.1 Runtime sizing

- **IO-bound**: workers = `num_cpus::get()`, often 2× cores is *worse* because of steal contention.
- **Mixed CPU/IO**: reserve a share of cores for a `rayon` pool; give Tokio the rest.
- **Many short tasks**: increase `event_interval` (e.g. 128→1024) to reduce I/O-poll overhead.
- **Low-latency**: decrease `event_interval` to 31 or so; poll I/O more often.

### 19.2 Memory

- Task is ~300 bytes header + size_of(future). Fat futures blow this up quadratically (sum of subfuture sizes × calling points).
- Box large sub-futures (`Box::pin`) to cap state machine size.
- Pool buffers (`bytes::BytesMut`, `crossbeam::queue::ArrayQueue`) — avoid per-task allocation.

### 19.3 Fairness

- Default fairness (LIFO slot + coop budget + periodic global-queue sweep) handles most cases.
- If one task starves others: instrument with `tokio-console`, look for "self-wakes" >> 0 (sign of busy polling).
- Use `tokio::task::consume_budget().await` in hot loops for explicit yield points.

### 19.4 I/O batching

- `write_all` with small chunks is slow — buffer in user space (`BufWriter`) or batch to vectored writes.
- `copy` uses `poll_write_vectored` when supported.
- For many small reads, use `BufReader`.

### 19.5 Avoiding per-request allocations

- Reuse `String` / `Vec` buffers across a worker's requests.
- `bytes::Bytes` / `BytesMut` for zero-copy cloning.
- `Arc<str>` / `Arc<[u8]>` for read-only shared data.

### 19.6 Benchmark harness

Use `criterion` for micro-bench, `tokio::runtime::Builder` with `enable_time().start_paused(true)` for deterministic time-driven tests. `hyperfine` for end-to-end throughput.

### 19.7 `LocalSet` for hot single-thread work

If your server has ~1 CPU and you're copying lots of data between tasks, a current-thread runtime with `LocalSet` avoids cross-thread atomics and can outperform multi-thread on microbenchmarks.

### 19.8 Picking a channel

| Scenario | Pick | Why |
|---|---|---|
| 1 producer, 1 consumer, cheap clone | `flume` or `async_channel` | Faster than tokio::mpsc in some benchmarks |
| MPMC | `async_channel` / `flume` | Native mpmc |
| Async + ordered MPSC | `tokio::sync::mpsc` | Default |
| Event fan-out | `tokio::sync::broadcast` | Ring buffer, lag |
| Latest config | `tokio::sync::watch` | Overwrite semantics |
| Bursty backpressure | Bounded `mpsc` + `try_send` with drop policy | Deliberate loss |

---

## 20. Modern Rust async features (AFIT, RPITIT, TAIT)

### 20.1 AFIT — async fn in trait (stable 1.75+)

```rust
trait Database {
    async fn get(&self, key: &str) -> Option<Vec<u8>>;
    async fn put(&self, key: &str, val: Vec<u8>);
}

impl Database for RocksDb { /* ... */ }
```

History:
- Pre-1.75: `#[async_trait]` crate boxed every async call (`Pin<Box<dyn Future + Send + '_>>`).
- 1.75+: native AFIT. No heap allocation.

Caveats of native AFIT:
- The associated future is `impl Future` — callers get an anonymous type.
- **No implicit `Send` bound**. This matters for `tokio::spawn`. Two solutions:

```rust
// Option 1: require Send on the returned future via RPITIT bound.
trait Db {
    fn get(&self, k: &str) -> impl std::future::Future<Output = Option<Vec<u8>>> + Send;
}

// Option 2 (1.75+): trait-associated type bound.
trait Db2 {
    async fn get(&self, k: &str) -> Option<Vec<u8>>;
}
// Caller adds bound:
fn spawnable<T: Db2 + 'static>(db: T) where T::get(..): Send { /* nightly-ish */ }
```

In practice for 2024/2025 libraries:
- Internal APIs: use native AFIT.
- Public traits that need `dyn Trait` **still need `#[async_trait]`** — AFIT traits are not object-safe (until RPITIT-for-dyn lands stably, see dyner / trait-variant work).
- `trait-variant` crate is the bridging tool: generate both `Foo` and `FooSend` variants with one attribute.

### 20.2 RPITIT — return-position `impl Trait` in trait

Same stabilization as AFIT. Lets you return `impl Future` (or `impl Iterator`, etc.) from a trait method:

```rust
trait Handler {
    fn handle(&self, req: Request) -> impl Future<Output = Response> + Send;
}
```

Semantics:
- The concrete returned type is associated with the impl.
- Different `impl`s yield different concrete types — no dynamic dispatch.
- Captures all input lifetimes by default. Use `+ use<'a, 'b>` precision (1.82+) to narrow.

### 20.3 TAIT — type alias impl Trait (unstable)

```rust
#![feature(type_alias_impl_trait)]
type MyFuture = impl Future<Output = u32>;
fn mk() -> MyFuture { async { 7 } }
```

Lets you name an anonymous future type — useful for storing it in a struct. Stabilization pending, likely in a "MSRV-later" sense. `impl_trait_in_associated_types` handles the trait-implementing cases.

### 20.4 `Box<dyn Future>` vs `impl Future` vs `Pin<Box<dyn Future>>`

| Form | Where to use | Cost |
|---|---|---|
| `impl Future` (return position) | Async fns, single-impl generic code | No allocation; type inferred |
| `Box<dyn Future<Output=T>>` | Not useful directly — missing `Pin` | — |
| `Pin<Box<dyn Future<Output=T> + Send + 'static>>` | Heterogeneous collections, trait objects | One heap alloc; dynamic dispatch |
| `BoxFuture<'a, T>` (from `futures`) | Shorthand for the above | Same |
| `LocalBoxFuture<'a, T>` | Same but without `Send` | Same |

```rust
use futures::future::BoxFuture;

struct Router {
    handlers: Vec<Box<dyn Fn(Req) -> BoxFuture<'static, Resp> + Send + Sync>>,
}
```

Prefer `impl Future` unless you absolutely need `dyn`.

### 20.5 `async` closures

As of writing, stable `async` closures landed in 1.85 under the name `AsyncFn*` traits. Before that, you returned a future from a regular closure:

```rust
// Pre-1.85 idiom — returning a future from a regular closure
let f = |x: u32| async move { fetch(x).await };
let result = f(42).await;

// Post-1.85
let f = async |x: u32| { fetch(x).await };
```

Key traits: `AsyncFn`, `AsyncFnMut`, `AsyncFnOnce` — parallels to `Fn`/`FnMut`/`FnOnce`. Accept them in APIs instead of `FnMut() -> impl Future`.

---

## 21. Ecosystem crate picks

### 21.1 Foundations

- **`tokio`** — reference runtime. Use `features = ["full"]` to start, trim later.
- **`futures`** — trait definitions (`Future`, `Stream`), combinators (`join_all`, `select!` via `futures::select!`), executors (rare), `FuturesUnordered`. Often paired with Tokio.
- **`tokio-util`** — codecs, `CancellationToken`, `DelayQueue`, `PollSemaphore`, sync adapters.
- **`tokio-stream`** — `StreamExt` for `tokio::sync::mpsc::Receiver` etc. and Tokio-specific stream combinators (`throttle`, `chunks_timeout`).

### 21.2 Pinning utilities

- **`pin-project`** — proc-macro generating structural-pin safe `.project()`.
- **`pin-project-lite`** — smaller, macro_rules-based; prefer for leaf libraries to reduce compile time.

### 21.3 Async traits / dyn

- **`async-trait`** — still relevant for `dyn` object safety until native dyn-AFIT stabilizes.
- **`trait-variant`** — authoring pattern: generate `Foo` + `FooSend` from one attribute.
- **`dyner`** (experimental) — object-safety wrapper for AFIT traits.

### 21.4 Network / protocols

- **`hyper`** (1.x) — HTTP/1 + HTTP/2 core. Unopinionated, low-level.
- **`reqwest`** — high-level client on top of hyper.
- **`axum`** — ergonomic HTTP server on hyper + tower.
- **`tower`** — middleware abstraction (`Service` trait); used by axum, tonic, reqwest-middleware.
- **`tonic`** — gRPC on hyper + prost.
- **`rustls`** / **`tokio-rustls`** — pure-Rust TLS.
- **`quinn`** — QUIC + HTTP/3.

### 21.5 Database

- **`sqlx`** — compile-time SQL checked, async. Backends: Postgres, MySQL, SQLite.
- **`sea-orm`** — active-record-ish ORM on sqlx.
- **`deadpool`** / **`bb8`** — async connection pooling (deadpool is newer, fewer deps).
- **`mongodb`** — official async driver.
- **`redis`** — `redis-rs` has an async API via `tokio` or `async-std` feature.

### 21.6 Testing

- **`tokio::test`** / `#[tokio::test]` — test-scoped runtime.
- **`tokio-test`** — `io::Builder` for mock IO, `task::spawn` helpers, `time` advancement.
- **`mockall`** — mocks for traits (incl. async).
- **`wiremock`** — HTTP mocking for integration tests.
- **`proptest`** / **`quickcheck`** — property tests; async requires `#[tokio::test]` or similar.

### 21.7 Utilities

- **`anyhow`** / **`thiserror`** — error handling (applications / libraries respectively).
- **`color-eyre`** — anyhow with pretty panic reports.
- **`tracing`** + **`tracing-subscriber`** — logging.
- **`tracing-opentelemetry`** — trace export.
- **`metrics`** / **`metrics-exporter-prometheus`** — metrics.
- **`bytes`** — `Bytes` / `BytesMut` zero-copy buffers.
- **`dashmap`** — concurrent hashmap (sync; fine to read from async).
- **`parking_lot`** — faster sync `Mutex`/`RwLock` than `std`.
- **`crossbeam`** — sync primitives, queues, channels for sync code.
- **`flume`** / **`async-channel`** — MPMC channels with async support.

### 21.8 Alternative runtimes

- **`smol`** — small, modular; often paired with `async-std`-ish APIs.
- **`async-std`** — tokio-analogous but API-compatible with `std`; less active.
- **`glommio`** — thread-per-core, io_uring-based, Linux-only; high-throughput storage workloads.
- **`monoio`** — similar philosophy to glommio, io_uring.
- **`embassy`** — embedded async runtime.

Cross-runtime code: use `futures`-only traits and the `async-executor` crate, or accept that most serious libraries target Tokio as the de facto default.

---

## 22. Unsafe & FFI touchpoints for async

### 22.1 `Pin` safety obligations

When implementing `Future` manually for a `!Unpin` type and using `unsafe`:

- `get_unchecked_mut` is safe **iff** you never move out of the returned `&mut`.
- Structural pinning: if you access field `f: &mut SubFut`, you must do so through `Pin::new_unchecked(&mut self.f)` — and you must not expose `&mut self.f` safely.
- `Drop::drop` for `!Unpin` types: receives `&mut Self`. Rust cannot express that this is a "pinned &mut" — so you must uphold the contract manually: treat it as pinned.

### 22.2 Self-referential state machines

Compiler-generated async state machines rely on `Pin` to safely hold self-references. Any hand-written analog must:

- Not expose safe moves.
- Only write self-refs in pinned contexts.
- Use `MaybeUninit` or `ManuallyDrop` for fields initialized late.

Example — `pinned` self-ref sketch:

```rust
use std::marker::PhantomPinned;
use std::ptr::NonNull;

struct SelfRef {
    data: String,
    ptr: Option<NonNull<String>>,
    _pin: PhantomPinned,     // opt out of Unpin
}

impl SelfRef {
    fn init(mut self: Pin<&mut Self>) {
        unsafe {
            let this = self.as_mut().get_unchecked_mut();
            this.ptr = Some(NonNull::from(&this.data));
        }
    }
}
```

`PhantomPinned` is the standard way to make a type `!Unpin` without adding unsafe impl `!Unpin` (which is itself unstable).

### 22.3 FFI futures

Wrapping an async-callback C API:

```rust
struct CAsyncOp { state: Arc<Mutex<Option<Waker>>>, result: Arc<Mutex<Option<i32>>> }

extern "C" fn on_complete(user: *mut c_void, res: i32) {
    let op = unsafe { Arc::from_raw(user as *const (Mutex<...>, Mutex<...>)) };
    *op.1.lock().unwrap() = Some(res);
    if let Some(w) = op.0.lock().unwrap().take() { w.wake(); }
}

impl Future for CAsyncOp {
    type Output = i32;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<i32> {
        if let Some(r) = self.result.lock().unwrap().take() {
            Poll::Ready(r)
        } else {
            *self.state.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
```

Key points:
- Arc cloning ensures C side's `user` pointer stays valid.
- Waker stored before returning Pending.
- Callback runs from some C thread — `Waker` is `Send + Sync`, so `.wake()` is safe from there.

### 22.4 `unsafe impl Send/Sync` on futures

Never needed for compiler-generated async fns. Only for hand-written futures that carry raw pointers. Standard rules apply: the pointed-to data must uphold `Send`/`Sync` invariants.

### 22.5 Cancellation on drop in unsafe FFI

If your FFI future represents an outstanding C operation, its `Drop` must cancel that operation. Otherwise dropping the future (e.g. in `select!`) would leak the C state or cause the callback to fire into freed memory.

```rust
impl Drop for CAsyncOp {
    fn drop(&mut self) {
        unsafe { c_cancel(self.handle); }      // synchronous cancel if possible
        // If the C API's cancel is also async, you may need to leak the op
        // or use a detached watchdog task.
    }
}
```

---

## 23. Quick reference — decision tables

### 23.1 "Which spawn primitive?"

| Scenario | Use |
|---|---|
| Async work, needs parallelism | `tokio::spawn` |
| Sync work that blocks | `tokio::task::spawn_blocking` |
| Async work, `!Send` state | `tokio::task::spawn_local` under `LocalSet` |
| Brief blocking call in async fn, multi-thread rt | `tokio::task::block_in_place` |
| True isolated OS thread | `std::thread::spawn` |
| Many short CPU tasks | `rayon::spawn` + oneshot |

### 23.2 "Which sync primitive?"

| Need | Pick |
|---|---|
| Protect value, cross `.await` | `tokio::sync::Mutex` |
| Protect value, no `.await` inside | `std::sync::Mutex` or `parking_lot::Mutex` |
| Read-heavy + `.await` | `tokio::sync::RwLock` |
| "Something happened" signal | `tokio::sync::Notify` |
| Limit concurrent uses | `tokio::sync::Semaphore` |
| Request/response with workers | `mpsc::channel` + `oneshot::channel` |
| Broadcast events | `tokio::sync::broadcast` |
| Latest-value updates | `tokio::sync::watch` |
| One-shot value | `tokio::sync::oneshot` |
| Shared cached value | `Arc<T>` + `OnceCell` |
| Counter | `std::sync::atomic::*` |

### 23.3 "Which combinator?"

| Need | Use |
|---|---|
| "All of these, concurrent, wait for all" | `tokio::join!` |
| "All of these, error short-circuits" | `tokio::try_join!` |
| "Whichever finishes first" | `tokio::select!` |
| "Stream of runtime-sized futures, completion order" | `FuturesUnordered` |
| "Stream of futures, but ordered" | `.buffered(n)` on a Stream |
| "Spawn N tasks, collect results" | `JoinSet` |
| "Collect Vec<Fut<Result>> → Result<Vec>" | `futures::future::try_join_all` |

### 23.4 "Cancel-safety in `select!`"

| Inside `select!` | Safe? | Fix if not |
|---|---|---|
| `sleep`, `timeout` | Yes | — |
| `rx.recv()` (mpsc/broadcast/watch/oneshot) | Yes | — |
| `tx.send(v)` (mpsc) | **No** | `.reserve().await` then send; or pre-send in inner task |
| `read_exact` / `write_all` | **No** | Wrap in `tokio::spawn` + await `JoinHandle` |
| `Mutex::lock().await` | Yes | — |
| `Notify::notified()` | Yes | — |
| `CancellationToken::cancelled()` | Yes | — |
| `futures::stream::next()` | Depends on the stream | — |

### 23.5 "Error trait on async fn"

```rust
// Preferred: thiserror for libraries
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyError {
    #[error("io: {0}")]       Io(#[from] std::io::Error),
    #[error("timeout")]       Timeout(#[from] tokio::time::error::Elapsed),
    #[error("channel closed")] Closed,
}

// Applications: anyhow
async fn run() -> anyhow::Result<()> {
    let bytes = tokio::fs::read("f").await?;
    tokio::time::timeout(Duration::from_secs(1), other()).await??;
    Ok(())
}
```

### 23.6 "Compile-time-only async rules"

1. `async fn` in trait requires Rust 1.75+ for native support.
2. `impl Future` in return position requires 1.39+ (async/await MVP) and respects lifetime capture rules (use `+ use<..>` to narrow since 1.82+).
3. `Pin` is stable since 1.33.
4. `tokio::spawn` future must be `Send + 'static`; output also `Send + 'static`.
5. `tokio::test` requires `#[tokio::test]`; `#[test] async fn` doesn't work.
6. `#[tokio::main]` requires a binary crate's `main`; cannot be applied to library entry points used by other runtimes.

### 23.7 Error patterns specific to async

- `JoinError` — check `.is_panic()`, `.is_cancelled()`, potentially `.into_panic()`.
- `tokio::sync::mpsc::error::SendError<T>` — channel closed; `T` is the value you were trying to send (recoverable).
- `tokio::sync::mpsc::error::TrySendError<T>::Full(T)` / `::Closed(T)` — try_send variants.
- `tokio::sync::oneshot::error::RecvError` — sender was dropped.
- `tokio::time::error::Elapsed` — timeout fired.

### 23.8 "Shape of typical Tokio service `main`"

```rust
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, error};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Init
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // 2. Config
    let cfg = Config::load()?;
    let shutdown = CancellationToken::new();

    // 3. Shared state
    let state = Arc::new(AppState::new(&cfg).await?);

    // 4. Start server
    let listener = TcpListener::bind(cfg.addr).await?;
    let server = tokio::spawn({
        let shutdown = shutdown.child_token();
        let state = state.clone();
        async move { run_server(listener, state, shutdown).await }
    });

    // 5. Signal handling
    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("SIGINT"),
        _ = wait_for_sigterm() => info!("SIGTERM"),
        res = &mut server => {
            error!(?res, "server exited early");
            shutdown.cancel();
            return Ok(());
        }
    }

    // 6. Drain
    shutdown.cancel();
    match tokio::time::timeout(std::time::Duration::from_secs(30), server).await {
        Ok(Ok(_))  => info!("clean shutdown"),
        Ok(Err(e)) => error!(?e, "server panicked"),
        Err(_)     => error!("shutdown timeout; aborting"),
    }
    Ok(())
}
```

---

## Appendix A — Full `Future` / `Pin` chapter map (Async Book mirror)

1. *Getting Started* — why async, hello world, green-thread vs stackless.
2. *Under the Hood: Executing Futures and Tasks* — building a mini executor on `std::task`, waker implementation walk.
3. *`async`/`.await`* — desugaring, captures, borrowing rules.
4. *Pinning* — why, contracts, `Pin` projections.
5. *The Stream Trait* — pollable iteration.
6. *Executing Multiple Futures at a Time* — `join`, `select`, `spawn`.
7. *Workarounds* — recursion needs `Box::pin`, `?` in async.
8. *The async ecosystem* — runtimes, futures crate.
9. *Final Project: a simple HTTP server* — tying it all together.

## Appendix B — Full Tokio Tutorial map (topic index)

1. *Hello Tokio* — setup, first `async` main.
2. *Spawning* — `tokio::spawn`, `JoinHandle`, `Send + 'static`.
3. *Shared State* — `Arc<Mutex<T>>`, when to avoid.
4. *Channels* — mpsc, oneshot, broadcast, watch basics.
5. *I/O* — `AsyncRead`/`AsyncWrite`, echo server.
6. *Framing* — `tokio_util::codec`, `LengthDelimitedCodec`.
7. *Async in Depth* — how polling works, waker internals.
8. *Select* — `select!` semantics, cancellation inside branches.
9. *Streams* — `Stream`, `StreamExt`, building, consuming.
10. *Bridging with sync code* — `block_on`, `Handle::enter`, calling Tokio from sync.
11. *Graceful shutdown* — signals, fanout, JoinSet.
12. *Tracing* — `tracing`, `#[instrument]`, tokio-console.
13. *Cancellation* — CancellationToken, cancel-safety, drop semantics.

---

## Appendix C — Tokio crate modules skim

| Module | Purpose |
|---|---|
| `tokio::runtime` | `Runtime`, `Builder`, `Handle`, `EnterGuard` |
| `tokio::task` | `spawn`, `spawn_blocking`, `spawn_local`, `JoinHandle`, `JoinSet`, `LocalSet`, `yield_now`, `block_in_place` |
| `tokio::sync` | `Mutex`, `RwLock`, `Notify`, `Semaphore`, `mpsc`, `oneshot`, `broadcast`, `watch`, `OnceCell`, `SetOnce` |
| `tokio::io` | `AsyncRead`, `AsyncWrite`, `AsyncReadExt`, `AsyncWriteExt`, `BufReader`, `BufWriter`, `copy`, `split` |
| `tokio::net` | `TcpListener`, `TcpStream`, `UdpSocket`, `UnixListener`/`UnixStream` |
| `tokio::time` | `sleep`, `interval`, `timeout`, `Instant`, `Duration`, `Interval`, `Sleep` |
| `tokio::fs` | Async wrappers over `std::fs` (uses blocking pool under the hood) |
| `tokio::signal` | `ctrl_c`, `unix::signal`, `windows::ctrl_shutdown` |
| `tokio::process` | `Command`, `Child` — async process management |
| `tokio::stream` (tokio-stream crate) | `StreamExt`, `wrappers::ReceiverStream`, throttle, chunks_timeout |

---

## Appendix D — Annotated gotcha list (one-liners)

- `.await` holds every local alive; prefer block expressions to drop early.
- Default `tokio::spawn` output is `()` — capture via `JoinHandle<T>` if you need the return.
- `tokio::sync::Mutex` cannot be used in `lazy_static` — use `OnceCell::const_new()` instead.
- Don't `Box::pin(async { .. })` inside a hot loop — allocations add up. Prefer `tokio::pin!`.
- `tokio::io::BufReader::new(tcp_stream)` → can shadow `into_split`; if you need split halves, split first then buf.
- `async fn main() -> Result<(), Box<dyn Error>>` works but loses structured errors. Prefer `anyhow::Result<()>`.
- `tokio::test` with `start_paused` — `sleep` resolves instantly and time auto-advances to the next awaited deadline. Useful but confusing; use only when you actively want that.
- `tokio_stream::wrappers::ReceiverStream` yields `None` when sender drops — pair with explicit shutdown or use `BroadcastStream` for fanout.
- `reqwest::Client` is cheap-clone; make ONE per app and share via `Arc`. Don't make one per request.
- `hyper::client` (raw) requires explicit connection pooling; `reqwest` handles it.
- `axum::Router` cannot be cloned after it's wrapped in `Server::serve` — clone the `Router` *before* conversion.

---

## Appendix E — Minimum-awareness trait summary

```rust
// core::future::Future
pub trait Future {
    type Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
}

// core::future::IntoFuture  (stable 1.64)
pub trait IntoFuture {
    type Output;
    type IntoFuture: Future<Output = Self::Output>;
    fn into_future(self) -> Self::IntoFuture;
}

// futures_core::stream::Stream
pub trait Stream {
    type Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;
    fn size_hint(&self) -> (usize, Option<usize>) { (0, None) }
}

// futures_sink::Sink
pub trait Sink<Item> {
    type Error;
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn start_send(self: Pin<&mut Self>, item: Item) -> Result<(), Self::Error>;
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
}

// tokio::io::AsyncRead
pub trait AsyncRead {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>>;
}

// tokio::io::AsyncWrite
pub trait AsyncWrite {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>>;
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

// AsyncFn / AsyncFnMut / AsyncFnOnce  (stable 1.85)
pub trait AsyncFn<Args> { /* like Fn, async result */ }
```

---

## Appendix F — Copy-pasteable spawn/shutdown skeleton

```rust
// All imports needed for a production Tokio service with shutdown and tracing.
use std::{sync::Arc, time::Duration};

use tokio::{
    net::TcpListener,
    signal,
    sync::watch,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, Instrument};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let shutdown = CancellationToken::new();
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    let mut tasks: JoinSet<()> = JoinSet::new();

    // Accept loop
    {
        let shutdown = shutdown.child_token();
        tasks.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    res = listener.accept() => match res {
                        Ok((sock, _)) => {
                            let sc = shutdown.child_token();
                            tokio::spawn(
                                async move { serve_conn(sock, sc).await }
                                    .instrument(tracing::info_span!("conn"))
                            );
                        }
                        Err(e) => error!(?e, "accept"),
                    }
                }
            }
        }.instrument(tracing::info_span!("accept")));
    }

    // Wait for shutdown signal
    tokio::select! {
        _ = signal::ctrl_c() => info!("ctrl-c"),
        _ = wait_sigterm() => info!("SIGTERM"),
    }
    shutdown.cancel();

    // Drain
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = &mut deadline => { error!("drain timeout"); tasks.abort_all(); break; }
            next = tasks.join_next() => match next {
                None => break,
                Some(Ok(())) => {},
                Some(Err(e)) if e.is_panic() => error!("task panicked"),
                Some(Err(_)) => {},
            }
        }
    }

    info!("bye");
    Ok(())
}

#[instrument(skip(sock))]
async fn serve_conn(mut sock: tokio::net::TcpStream, shutdown: CancellationToken) {
    tokio::select! {
        res = handle_requests(&mut sock) => if let Err(e) = res { error!(?e); }
        _ = shutdown.cancelled() => {
            let _ = sock.shutdown().await;
        }
    }
}

#[cfg(unix)]
async fn wait_sigterm() {
    let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate()).unwrap();
    sig.recv().await;
}
#[cfg(not(unix))]
async fn wait_sigterm() { std::future::pending::<()>().await; }

# async fn handle_requests(_: &mut tokio::net::TcpStream) -> anyhow::Result<()> { Ok(()) }
```

---

## Appendix G — Patterns-to-check lint list (LLM coding aid)

When reviewing async Rust, scan for these red flags in this order:

1. `std::sync::Mutex`/`RwLock` with a `.await` between `.lock()` and drop of the guard.
2. `tokio::spawn(async { .. })` where the future clearly contains non-`Send` types (check for `Rc`, `RefCell`, `*const`).
3. `.await?` on a future created inside a `select!` branch that does partial writes (`read_exact`, `write_all`, `read_to_end`, `copy`, `copy_bidirectional`).
4. Unbounded channels (`mpsc::unbounded_channel`, `async_channel::unbounded`) in request-handling paths.
5. `std::thread::sleep`, `std::fs::*`, `std::process::Command::output` in async fns.
6. `tokio::spawn` in a loop with `for` and no `JoinSet`/`Semaphore` — unbounded fan-out.
7. Tight CPU loops inside `async fn` with no `.await` or `yield_now()`.
8. `Box::pin` inside a hot loop.
9. `lazy_static!` + `tokio::sync::Mutex::new` (won't compile; needs `OnceCell::const_new()` or `LazyLock`).
10. `JoinHandle`s discarded (`let _ = tokio::spawn(..)`) when the task may panic.
11. `select!` without `biased;` where a cancel branch should always win.
12. `reqwest::Client::new()` inside a request handler (should be app-scoped).
13. Multiple distinct runtimes created via `Runtime::new()` in the same process — usually a mistake.
14. `Pin<Box<dyn Future>>` used without `Send` bound, then spawned via `tokio::spawn`.
15. Storing a `Waker` behind a lock but never `clone()`-ing it before returning `Pending`.

---

## Appendix H — Tokio crate feature flag map

| Feature | Enables |
|---|---|
| `full` | All of: `rt-multi-thread`, `macros`, `sync`, `time`, `io-util`, `io-std`, `net`, `fs`, `signal`, `process`, `parking_lot` |
| `rt` | `Runtime`, `Handle`, tasks, `spawn_blocking`. No I/O driver. |
| `rt-multi-thread` | Adds multi-thread scheduler |
| `macros` | `#[tokio::main]`, `#[tokio::test]`, `select!`, `join!`, `try_join!`, `pin!` |
| `sync` | `sync::Mutex`, `mpsc`, `oneshot`, `broadcast`, `watch`, `Notify`, `Semaphore` |
| `time` | `time::sleep`, `interval`, `timeout`, `Instant` |
| `io-util` | `AsyncReadExt`, `AsyncWriteExt`, `BufReader`, `BufWriter` |
| `io-std` | `io::stdin()`/`stdout()`/`stderr()` |
| `net` | `TcpStream`, `TcpListener`, `UdpSocket`, `UnixStream`/`UnixListener` |
| `fs` | `tokio::fs::*` |
| `process` | `tokio::process::Command` |
| `signal` | `tokio::signal` |
| `parking_lot` | Use `parking_lot` internally for faster mutexes |
| `tracing` | Emit spans for tasks/resources (needed for tokio-console) |

Minimum realistic set: `macros`, `rt-multi-thread`, `sync`, `time`, `io-util`, `net`. Skip `full` in libraries.

---

## Appendix I — Fast pointer on language/runtime versioning

| Feature | Stable since |
|---|---|
| `async`/`.await` syntax | Rust 1.39 (Nov 2019) |
| `Pin` in std | Rust 1.33 (Feb 2019) |
| `std::task::ready!` | Rust 1.64 (Sep 2022) |
| `async fn` in trait (AFIT) | Rust 1.75 (Dec 2023) |
| RPITIT (return-position impl Trait in trait) | Rust 1.75 (Dec 2023) |
| `std::future::IntoFuture` | Rust 1.64 (Sep 2022) |
| `let-else` (used a lot in async code) | Rust 1.65 (Nov 2022) |
| `impl Trait` precise capturing (`+ use<'a>`) | Rust 1.82 (Oct 2024) |
| `async` closures & `AsyncFn*` traits | Rust 1.85 (Feb 2025) |
| Tokio 1.0 | Dec 2020; API frozen for 5-year stability window |

This gives you a quick "is this MSRV realistic for feature X?" lookup.

---

## Appendix J — Misc testing tips

- `#[tokio::test]` creates a fresh current-thread runtime per test. Override with `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`.
- Use `start_paused = true` to make time deterministic:
  ```rust
  #[tokio::test(start_paused = true)]
  async fn my_test() {
      let start = tokio::time::Instant::now();
      tokio::time::sleep(Duration::from_secs(60)).await;   // instant
      assert_eq!(start.elapsed(), Duration::from_secs(60));
  }
  ```
- `tokio::time::advance(dur)` — manually advance time in paused mode.
- `tokio::task::yield_now().await` — commonly needed in tests to let spawned tasks run one step.
- `tokio_test::io::Builder` — scripted AsyncRead/AsyncWrite for testing codecs.

---

## Appendix K — Bridging sync/async recipe

### Sync caller, async lib

```rust
// Caller is not async.
let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();

let result = rt.block_on(async {
    reqwest::get("https://example.com").await?.text().await
});
```

### Async caller, sync lib

```rust
// Inside tokio task.
let res = tokio::task::spawn_blocking(|| {
    blocking_db_query("SELECT 1")
}).await??;
```

### Callback-style sync API to future

Use `oneshot`:

```rust
fn query_async() -> impl Future<Output = Result<i32, Error>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let r = blocking_query();
        let _ = tx.send(r);
    });
    async move { rx.await? }
}
```

Or — better — use `spawn_blocking`:

```rust
fn query_async() -> tokio::task::JoinHandle<Result<i32, Error>> {
    tokio::task::spawn_blocking(|| blocking_query())
}
```

---

## Appendix L — Typed-summary for LLMs (structured)

```yaml
future_trait:
  signature: "fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>"
  rules:
    - poll_after_ready_is_undefined_behavior
    - store_latest_waker_before_returning_pending
    - pending_must_arrange_wakeup_unless_noop_future
    - wakers_are_send_sync

pin:
  purpose: "enforce that self-referential state machines don't move"
  structural_pinning:
    - field_marked_#[pin]_gets_pinned_projection
    - field_not_marked_is_regular_&mut
    - drop_must_not_move_out_of_pin_fields

cancellation:
  mechanism: "drop the Future"
  safety_definition: "safe to drop after partial progress without losing data"
  not_cancel_safe:
    - "tx.send(v)"
    - "read_exact / read_to_end"
    - "write_all / copy / copy_bidirectional"

spawn:
  tokio_spawn:
    bounds: "Send + 'static"
    returns: "JoinHandle<T>"
  spawn_blocking:
    bounds: "FnOnce -> R + Send + 'static"
    runs_on: "blocking pool (default max 512)"
  spawn_local:
    requires: "LocalSet context"
    bounds: "Future + 'static"   # no Send
  block_in_place:
    requires: "multi-thread runtime"
    effect: "migrate sibling tasks, then run closure"

sync_primitives:
  tokio_mutex: "Send guard; crosses .await; slower than std"
  tokio_rwlock: "read-heavy; write-preferring"
  std_mutex: "faster; guard is !Send; cannot cross .await safely"
  notify: "parameterless signal; notify_one sets permit, notify_waiters doesn't"
  semaphore: "bounded permits; can be closed"
  mpsc: "bounded recommended; send not cancel-safe; reserve+send is"
  oneshot: "one-shot transfer; Receiver is Future"
  broadcast: "ring buffer; Lagged error on slow receiver"
  watch: "overwrite semantics; changed() awaits updates"

afit:
  stable_since: "1.75"
  no_send_bound_implicit: true
  for_dyn_use: "still need async-trait until dyn-AFIT stabilizes"

performance:
  default_workers: "num_cpus::get()"
  coop_budget: "128 per poll; inserts implicit yields"
  task_size: "header ~300B + future state size"
  diagnose_fat_futures: "cargo +nightly rustc -- -Zprint-type-sizes"
```

---

## Appendix M — Common interview-style Q&A

**Q: How does a waker know which task to wake?**
A: The waker is an opaque pointer (typically `Arc<Task>`) plus a vtable. `wake()` looks up the task, marks it ready, and pushes it onto the originating scheduler's run queue.

**Q: Why does `tokio::spawn` require `Send`?**
A: Because the runtime may migrate the task between worker threads for load balancing. `'static` is required because the task's lifetime is independent of any borrow.

**Q: Can I `await` the same future twice?**
A: No. Futures are one-shot — after `Ready`, re-polling is UB per the contract. `.clone()` a future only if it implements `Clone` (rare). Common workaround: `futures::future::Shared` to turn a future into a cloneable handle.

**Q: What's the difference between `yield_now` and `sleep(Duration::ZERO)`?**
A: `yield_now` explicitly yields to the scheduler; it's a no-IO cooperative yield. `sleep(0)` registers with the time driver (still basically instant) — more overhead. Prefer `yield_now`.

**Q: Is `async fn` zero-cost?**
A: Per call, yes (no heap alloc, state machine is stack-embedded). But **state machine size** can balloon with deeply-nested `async fn`. Use `Box::pin` strategically to cap size.

**Q: Why can't I make `async fn` object-safe?**
A: Each `impl` returns a different anonymous future type. `dyn Trait` needs a single vtable with fixed types. The workaround is `Pin<Box<dyn Future + Send>>` via `#[async_trait]` or manual boxing.

**Q: How do I time out a cancellation-unsafe operation?**
A: Wrap in a spawned task; await the `JoinHandle` under `timeout`. If timeout fires, call `handle.abort()`. Accept that the inner op may have partially completed.

**Q: What happens if my future never returns `Pending`?**
A: The scheduler's coop budget will eventually force a `Pending` return (for Tokio-aware futures). For hand-rolled ones without coop integration, you'll block the worker. Always either return `Pending` or use `yield_now` in long loops.

**Q: `tokio::sync::Mutex` vs `parking_lot::Mutex`?**
A: Different purposes. `tokio::sync::Mutex` is async — `.lock().await`. `parking_lot::Mutex` is sync, drop-in for `std::sync::Mutex` with better performance. Use `parking_lot` for purely-sync critical sections that don't touch `.await`.

---

## Appendix N — Summary of "what fails to compile and why"

| Symptom | Cause | Fix |
|---|---|---|
| `future cannot be sent between threads safely` | `!Send` type lives across `.await` in a spawned future | Use `Arc`/`Mutex`/`tokio::sync::*`; or use `spawn_local` under `LocalSet` |
| `the trait bound ... is not satisfied` on `tokio::spawn` | Future isn't `'static` (borrows) | `async move { .. }` with owned captures |
| `recursion in async fn requires boxing` | Indirect self-call | `fn recurse() -> BoxFuture<'_, T> { Box::pin(async { .. }) }` |
| `Pin<...>: does not implement ...` when moving | Pinned pointer cannot be moved into something | `Pin<Box<T>>` to move through function boundaries |
| `cannot return value referencing local variable` | Trying to return a future that borrows a local | Move the local into the future (`async move`) |
| `Drop cannot be implemented on a structurally pinned type` | Plain `impl Drop` on `#[pin_project]` with `#[pin]` fields | Use `#[pinned_drop]` attribute |
| `async function cannot be called recursively without boxing` | same as recursion | box |

---

## Appendix O — Production config templates

### Minimal `Cargo.toml` for a Tokio web service

```toml
[package]
name = "svc"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "io-util", "net", "signal"] }
tokio-util = { version = "0.7", features = ["rt"] }
tokio-stream = "0.1"
futures = "0.3"
pin-project-lite = "0.2"

axum = "0.7"
hyper = "1"
tower = "0.5"
tower-http = { version = "0.5", features = ["trace", "cors", "timeout"] }

tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

anyhow = "1"
thiserror = "1"

serde = { version = "1", features = ["derive"] }
serde_json = "1"

sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres"] }

[profile.release]
lto = "fat"
codegen-units = 1
```

### Production `#[tokio::main]`

```rust
#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> { /* ... */ }

// vs explicit Builder for control:
fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(std::thread::available_parallelism()?.into())
        .max_blocking_threads(1024)
        .thread_name("svc-worker")
        .enable_all()
        .build()?;
    rt.block_on(app_main())
}
```

---

*End of cluster-06-async-tokio.md.*
