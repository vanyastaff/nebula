# Cluster 05 — Rust Atomics and Locks (Mara Bos)

Source: Mara Bos, *Rust Atomics and Locks: Low-Level Concurrency in Practice* (O'Reilly, 2023). Online: https://marabos.nl/atomics/.

Dense expert notes for a coding-LLM knowledge base. Organized by chapter but cross-tagged with the target taxonomy:
`[02-language-rules]`, `[04-design-patterns]`, `[05-anti-patterns]`, `[07-async-concurrency]`, `[08-unsafe-and-ffi]`, `[09-performance]`.

### Taxonomy quick index

| Tag | Topics covered in this cluster |
|-----|--------------------------------|
| `[02-language-rules]` | `Send`/`Sync`, `Mutex` vs `RwLock` bounds, interior mutability (`Cell`/`RefCell`/`UnsafeCell`), data race = UB, happens-before, modification order |
| `[04-design-patterns]` | `thread::scope`, `Arc`/`Weak`, channels (oneshot, blocking), spin lock, RCU indirection, per-thread sharding |
| `[05-anti-patterns]` | Wrong ordering, `let _ = lock()`, DCL with `Relaxed`, ABA, false sharing, holding `MutexGuard` across `.await`, `unsafe impl Send` |
| `[07-async-concurrency]` | Why `std::sync::Mutex` + `.await` is hazardous; use `tokio::sync::Mutex` or shrink critical sections |
| `[08-unsafe-and-ffi]` | `UnsafeCell` in primitives, `unsafe impl Sync`, pthread movability, signal-handler constraints |
| `[09-performance]` | `Relaxed` vs stronger orderings, `compare_exchange_weak` on LL/SC, cache lines, x86 vs ARM, `SeqCst` cost |

### Primary sources (free online, same text as O’Reilly)

| Ch. | Topic | URL |
|-----|--------|-----|
| 1 | Basics of Rust Concurrency | https://marabos.nl/atomics/basics.html |
| 2 | Atomics | https://marabos.nl/atomics/atomics.html |
| 3 | Memory Ordering | https://marabos.nl/atomics/memory-ordering.html |
| 4 | Building Our Own Spin Lock | https://marabos.nl/atomics/building-spinlock.html |
| 5 | Building Our Own Channels | https://marabos.nl/atomics/building-channels.html |
| 6 | Building Our Own Arc | https://marabos.nl/atomics/building-arc.html |
| 7 | Understanding the Processor | https://marabos.nl/atomics/hardware.html |
| 8 | Operating System Primitives | https://marabos.nl/atomics/os-primitives.html |
| 9 | Building Our Own Locks | https://marabos.nl/atomics/building-locks.html |
| 10 | Ideas and Inspiration | https://marabos.nl/atomics/inspiration.html |

Supplementary figures / sidebars: `memory-ordering.html` links to `alt/3-1.html` … `alt/3-5.html`; `building-spinlock.html` → `alt/4-1.html`; `hardware.html` → `alt/7-1.html`; `building-locks.html` → `alt/9-1.html`, `alt/9-2.html`.

---

## Chapter 1 — Basics of Rust Concurrency

### 1.1 Threads in Rust

- `std::thread::spawn(f)` spawns an OS thread. Takes `FnOnce() -> T` where `T: Send + 'static` and `F: Send + 'static`. Returns `JoinHandle<T>`.
- `JoinHandle::join()` returns `thread::Result<T>` — `Err` only if the thread panicked. `join` always blocks until the thread finishes.
- Spawned threads run until they return OR until the whole process exits. When `main` returns, the process exits and any still-running threads are killed mid-work. `[05-anti-patterns]` Never rely on side effects of an un-joined thread running past `main`'s return — explicitly `join()` them.
- A closure passed to `spawn` must be `'static` because the OS thread may outlive the current stack frame. Moving local references in is a borrow-checker error — use `move` and pass owned data (e.g. `Vec`, `Arc<T>`), or use `thread::scope` for scoped threads.
- `thread::current()` gives a `Thread`. `thread::current().id()` yields a `ThreadId` (unique per thread, never reused). `Thread::name()` returns `Option<&str>` (unset unless named via `Builder::name`).
- `thread::Builder::new().stack_size(n).name(s).spawn(f)` returns `io::Result<JoinHandle<T>>`. Use when you need OS-error handling (stack too big, thread limit reached) or a name for debugging.
- **Panics do not propagate automatically.** A panic in a spawned thread unwinds that thread only. The main thread sees it only via `JoinHandle::join() -> Result<_, Box<dyn Any + Send>>`. `[05-anti-patterns]` Ignoring a `JoinHandle` silently swallows thread panics.

```rust
use std::thread;

let t = thread::spawn(|| {
    println!("hello from thread {:?}", thread::current().id());
    42_u32
});
let v = t.join().unwrap(); // v: u32
```

### 1.2 Scoped Threads (`std::thread::scope`) `[04-design-patterns]`

- Stable since 1.63. Allows threads to borrow non-`'static` data from the enclosing scope.
- `scope(|s| { s.spawn(|| ...); })`. The `scope` call blocks until **every** thread spawned on `s` has finished — enforced by the scope signature, not by library convention. Never returns before all threads join.
- The spawn closure must satisfy `F: Send + 'env`, not `'static`. Borrows of stack-local variables are sound because the scope guarantees no thread escapes the stack frame.
- `s.spawn(...)` returns a `ScopedJoinHandle<'scope, T>`; calling `.join()` early is allowed (also `is_finished()`). If you do not call `.join()`, panics inside a scoped thread are **re-raised from the scope at the end** — the scope itself panics. This is different from `thread::spawn`, where dropping the handle silently swallows panics.
- If any thread inside the scope panics and its handle is never joined, `scope(...)` itself panics once all threads finish. Joining a handle consumes the panic so the scope will not re-panic for that thread.

```rust
use std::thread;
let numbers = vec![1, 2, 3];
thread::scope(|s| {
    s.spawn(|| println!("len {}", numbers.len())); // immutable borrow
    s.spawn(|| println!("first {}", numbers[0]));  // another immutable borrow
});
// scope blocks here until both children finish
```

- `[05-anti-patterns]` You **cannot** have one scoped thread mutably borrow data that another scoped thread also borrows — the borrow checker enforces Rust's aliasing rules across threads because both closures are checked against the same environment.

### 1.3 Shared Ownership & Reference Counting

- **Statics** (`static X: T = ...;`) live forever and have `'static` references freely available.
- **Leaking** (`Box::leak`) produces a `&'static mut T` — useful for initializing shared state that truly never dies. Memory is never reclaimed, so use only at program startup.
- `Rc<T>`: cheap, **not `Send`**, **not `Sync`**. Clone increments a non-atomic counter. Trying to `spawn` a closure capturing an `Rc<T>` fails the `Send` bound at compile time.
- `Arc<T>`: atomic reference counting, `Send + Sync` if `T: Send + Sync`. Clone increments via `fetch_add(1, Relaxed)`; drop uses `fetch_sub(1, Release)` + an `Acquire` fence on the last decrement. See Ch. 6.
- Naming convention: `let a = Arc::new(x); let b = Arc::clone(&a);` — never `a.clone()` when you want to show readers a reference count bump; prefer associated-function form for clarity.

```rust
use std::sync::Arc;
let a = Arc::new([1, 2, 3]);
let b = Arc::clone(&a);
thread::spawn(move || dbg!(b));
thread::spawn(move || dbg!(a));
```

### 1.4 Borrowing & Data Races `[02-language-rules]` `[05-anti-patterns]`

- Rust's rules: any number of `&T` OR exactly one `&mut T`. This statically forbids data races on ordinary memory.
- A **data race** is UB in Rust: two threads access the same memory, at least one writes, with neither synchronization nor atomic ops.
- Interior mutability types (`Cell`, `RefCell`, `Mutex`, `RwLock`, atomics, `UnsafeCell`) let you mutate through `&T`, but each enforces the sharing invariant its own way (Cell/RefCell = single-threaded, Mutex/Atomic = thread-safe).

### 1.5 Interior Mutability Taxonomy `[02-language-rules]`

- `Cell<T>`: `get` (requires `T: Copy`), `set`, `replace`, `into_inner`. No references ever handed out — you copy values in/out. Single-threaded only (`!Sync`).
- `RefCell<T>`: dynamic borrow checking at runtime. `borrow()` / `borrow_mut()` return `Ref`/`RefMut` guards. Panics on conflicting borrows. `!Sync`.
- `UnsafeCell<T>`: the **only** legal way to obtain `&mut T` from `&UnsafeCell<T>`. Every interior-mutability primitive in Rust is built on `UnsafeCell`. Using `&T -> &mut T` any other way (e.g. `transmute`) is UB.
- `Mutex<T>` / `RwLock<T>`: thread-safe interior mutability via blocking.
- Atomics: `AtomicU8/U16/U32/U64/Usize/Bool/Ptr`, `AtomicI*`. Thread-safe, non-blocking, but limited to types that fit a word (or double-word on some platforms).

### 1.6 Thread Safety: `Send` and `Sync` `[02-language-rules]`

- `Send`: value ownership can be transferred across threads. Auto-trait.
- `Sync`: `&T` is `Send` — the value can be shared (via shared references) across threads. Auto-trait.
- Rules:
  - Primitives are both. References `&T` are `Send` iff `T: Sync`. `&mut T` is `Send` iff `T: Send`.
  - `Rc<T>` is `!Send` and `!Sync`. `Arc<T>` is `Send + Sync` iff `T: Send + Sync`.
  - `Cell<T>` / `RefCell<T>` are `Send` iff `T: Send`, but `!Sync`.
  - `Mutex<T>` and `RwLock<T>` are `Send + Sync` iff `T: Send` (the `Sync` bound on the inner `T` is NOT needed because the lock serializes access). `[02-language-rules]` This is why you can put a non-`Sync` `T` inside a `Mutex<T>` and share it.
  - `MutexGuard<'a, T>`: `Sync` iff `T: Sync`; **not** `Send` (the underlying OS mutex may require the same thread to unlock). Do not send a guard across a thread boundary.
  - Raw pointers `*const T`, `*mut T`: `!Send` and `!Sync` by default. Wrap in a newtype and `unsafe impl Send/Sync` when you've proven safety.
- Opting out: `impl !Send for MyType {}` (unstable) or equivalently a `PhantomData<Rc<()>>` field.
- Opting in on types with raw pointers:

```rust
struct X(*mut u8);
unsafe impl Send for X {} // you must audit this manually
```

- `[05-anti-patterns]` Writing `unsafe impl Send for X {}` without an argument for why it is safe — especially for types containing raw pointers into allocator-owned memory — is a classic source of UB.

### 1.7 Thread Parking `[04-design-patterns]`

- `thread::park()`: blocks the current thread until someone calls `unpark` on its `Thread` handle. Each thread has a single "parking token" — `unpark` sets the token, `park` consumes it.
- `Thread::unpark()` on a thread is **sticky**: if called when the thread is not parked, the next `park()` will return immediately. This avoids a lost wake-up when the producer runs before the consumer parks.
- `unpark` tokens do **not** stack — multiple unparks between two `park` calls are coalesced into one. The count is effectively a single boolean.
- `park_timeout(dur)` returns after the timeout even without an unpark.
- Canonical parking loop: always re-check a condition **in a loop**, because parking can spuriously return.

```rust
use std::collections::VecDeque;
use std::sync::Mutex;
use std::thread;

let queue = Mutex::new(VecDeque::new());
thread::scope(|s| {
    let t = s.spawn(|| loop {
        let item = queue.lock().unwrap().pop_front();
        if let Some(item) = item {
            dbg!(item);
        } else {
            thread::park();
        }
    });
    for i in 0.. {
        queue.lock().unwrap().push_back(i);
        t.thread().unpark();
        thread::sleep(std::time::Duration::from_secs(1));
    }
});
```

- `[05-anti-patterns]` The race the code must avoid: producer pushes and unparks **before** consumer parks — because the unpark token is sticky, the next `park()` consumes the token and returns, so you re-check and see the item. Without the sticky-token guarantee (e.g. on a naive condvar-less primitive), you would lose the wake-up.

### 1.8 Mutex `[02-language-rules]` `[05-anti-patterns]`

- `Mutex<T>` wraps `T`. `lock()` returns `LockResult<MutexGuard<'_, T>>`. Guard is `Deref<Target = T>` / `DerefMut`. Drop unlocks.
- **Poisoning**: if a thread panics while holding the lock, the `Mutex` is marked poisoned. Future `lock()` calls return `Err(PoisonError { .. })`. `PoisonError::into_inner()` gives the guard anyway — useful when you know the state is salvageable.
- Poisoning is **not** free per-lock; it's the default `std::sync::Mutex` behavior. `parking_lot::Mutex` has no poisoning and is slightly faster; pick poisoning only if corrupted state should halt downstream callers.
- `Mutex<T>: Send + Sync` when `T: Send`. The wrapped type does **not** need to be `Sync` — that's the whole point of the lock.
- Holding a lock across `await` in async code causes deadlocks if the executor runs multiple futures on one thread. Use `tokio::sync::Mutex` if you need to await while holding a lock. `[07-async-concurrency]`
- Shrink the critical section. Do not hold the lock over I/O or sleeps. `[09-performance]`

```rust
let n = Mutex::new(0);
thread::scope(|s| {
    for _ in 0..10 {
        s.spawn(|| {
            let mut g = n.lock().unwrap();
            for _ in 0..100 { *g += 1; }
        }); // guard dropped here -> unlock
    }
});
assert_eq!(n.into_inner().unwrap(), 1000);
```

- `[05-anti-patterns]` Temporary that holds the guard in `let _ = m.lock().unwrap();` — this drops **immediately** at the end of the statement (for `_` binding in `let _`). Use `let _guard = m.lock().unwrap();` to keep it until scope end. The book explicitly warns: `let _ = ...` drops now; `let _guard = ...` drops at end of scope. This is a recurring subtle bug.
- Nested locks: deadlock risk when threads lock `A` then `B` and others lock `B` then `A`. Establish a global ordering. `[05-anti-patterns]`

### 1.9 RwLock `[02-language-rules]`

- Allows many readers **or** one writer. `read()` / `write()` -> `RwLockReadGuard` / `RwLockWriteGuard`.
- `RwLock<T>: Send + Sync` when `T: Send + Sync` (note: `T: Sync` also required, unlike `Mutex`, because readers hand out `&T` to multiple threads concurrently).
- Implementation fairness varies. std's `RwLock` may allow writer starvation or reader starvation depending on platform; `parking_lot::RwLock` provides fair variants.
- Upgrades from read to write are **not** supported in std. Drop the read guard, then `write()`.

### 1.10 Condvar `[04-design-patterns]`

- Pair with a `Mutex`. `Condvar::wait(guard)` releases the mutex, blocks, then re-acquires on wake.
- `notify_one()` / `notify_all()` wake one / all waiters.
- `wait_while(guard, |state| predicate)` handles spurious wake-ups by looping.
- `[05-anti-patterns]` Always call `wait` in a loop checking the predicate — spurious wake-ups are allowed.

```rust
use std::sync::{Mutex, Condvar};
let m = Mutex::new(Vec::new());
let cv = Condvar::new();
thread::scope(|s| {
    s.spawn(|| {
        let mut g = m.lock().unwrap();
        while g.is_empty() { g = cv.wait(g).unwrap(); }
        let _work = g.pop();
    });
    m.lock().unwrap().push(1);
    cv.notify_one();
});
```

- Prefer `wait_while` for correctness.

### 1.11 mpsc Channels

- `std::sync::mpsc::channel()` -> `(Sender<T>, Receiver<T>)`. `Sender: Clone` (multi-producer). `Receiver`: single consumer.
- `send` returns `Err(SendError<T>)` if all receivers dropped. `recv` returns `Err(RecvError)` if all senders dropped.
- `try_recv`, `recv_timeout` available.
- `sync_channel(cap)` gives a bounded channel where `send` blocks when full.
- Prefer `crossbeam-channel` for MPMC or better performance.

### 1.12 Notes from the book’s Chapter 1 narrative `[02-language-rules]`

- Returning from `main` terminates the **whole process**; spawned threads do not keep it alive. Join or use scoped threads / channels to bound lifetime of work.
- `JoinHandle::join()` yields `Result<T, Box<dyn Any + Send>>` — the `Err` arm carries the panic payload from the child thread.
- Scoped threads (`thread::scope`): the closure receives a `Scope` `s`; **`scope` blocks until every `s.spawn` completes** (auto-join). Panics in child threads propagate: if a handle is not joined, `scope` will panic after threads finish.
- `Rc<T>` clone is cheap but the refcount update is not atomic → not `Send`. `Arc` uses atomic RMWs on the refcount.
- Shared references vs “immutable”: after interior mutability, prefer **shared** vs **exclusive** terminology (`&T` shared, `&mut T` exclusive).
- `Cell`/`RefCell` are single-threaded; `Mutex`/`RwLock` are the multi-threaded analogues of `RefCell` (blocking instead of panic on conflict).
- Thread naming: `Builder::name` helps debuggers and logs; stack size configurable when defaults are insufficient.

---

## Chapter 2 — Atomics

### 2.1 Atomic Types

- `std::sync::atomic::AtomicUsize` (and friends): represents a single word-sized atomically mutable value. All ops take `&self` (interior mutability via `UnsafeCell` + hardware atomic instructions).
- `new(v)`, `load(order)`, `store(v, order)`, `swap(v, order)`, `compare_exchange(cur, new, success, failure)`, `compare_exchange_weak(..)`, `fetch_add/sub/and/or/xor/max/min/update`, `get_mut(&mut self) -> &mut T` (exclusive), `into_inner(self) -> T`.
- Platform coverage: `AtomicU64`/`AtomicI64` may not exist on 32-bit-only targets. `AtomicU128` is unstable. Use `cfg(target_has_atomic = "64")` to gate.

### 2.2 Memory Ordering Overview

Every atomic op takes one of: `Relaxed`, `Release`, `Acquire`, `AcqRel`, `SeqCst`. Choosing the right one is the core of the book.

- `Relaxed`: no synchronization, only atomicity.
- `Acquire`: on load; nothing after this load can be reordered before it.
- `Release`: on store; nothing before this store can be reordered after it.
- `AcqRel`: on RMW; combines both sides.
- `SeqCst`: all SeqCst ops form a single total order.

### 2.3 `load` and `store` — Relaxed

- `store(v, Relaxed)` / `load(Relaxed)`: atomic but no ordering guarantees against other memory.
- Useful for counters and flags whose values don't gate access to other memory.

```rust
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::thread;

static COUNT: AtomicUsize = AtomicUsize::new(0);
thread::scope(|s| {
    for _ in 0..4 { s.spawn(|| COUNT.fetch_add(1, Relaxed)); }
});
assert_eq!(COUNT.load(Relaxed), 4);
```

### 2.4 Stop-flag Pattern `[04-design-patterns]`

```rust
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
static STOP: AtomicBool = AtomicBool::new(false);

thread::scope(|s| {
    let worker = s.spawn(|| {
        while !STOP.load(Relaxed) { do_work(); }
    });
    for line in std::io::stdin().lines() {
        match line.unwrap().as_str() {
            "help" => eprintln!("commands: help, stop"),
            "stop" => break,
            _ => eprintln!("unknown"),
        }
    }
    STOP.store(true, Relaxed);
});
```

`Relaxed` suffices for a bare flag because neither side reads other state that depends on the flag. If the flag gates reading shared data (e.g. a value initialized before setting the flag), you need Release/Acquire.

### 2.5 Progress Reporting `[04-design-patterns]`

- Worker writes `num_done` with `Relaxed`; main thread displays. Occasional slight staleness acceptable.
- Use `thread::park` / `unpark` or `thread::sleep` on the reporter side to avoid busy-looping.

### 2.6 Lazy Initialization `[04-design-patterns]` `[05-anti-patterns]`

- Racy init: multiple threads may each compute the value and race to store it. If computing is cheap & idempotent (e.g. a constant), that's fine with `Relaxed`.
- If computation is expensive or has side effects, use `std::sync::OnceLock` / `LazyLock` (stable 1.80). The book builds its own with CAS.
- `[05-anti-patterns]` Reading a partially-initialized value. If the stored value is a pointer to a complex structure, `Relaxed` alone is UB — you need `Release` on the store and `Acquire` on the successful load so the structure's fields are visible.

### 2.7 `fetch_add` and RMW Operations `[04-design-patterns]`

- `fetch_add(n, Relaxed) -> T` returns previous value. Useful for counters.
- `fetch_sub`, `fetch_and`, `fetch_or`, `fetch_xor`, `fetch_max`, `fetch_min`, `fetch_nand` (unstable).
- `swap(new, order) -> T` returns previous, always stores new.
- `fetch_update(success, failure, |cur| Option<T>) -> Result<T, T>`: retries on contention; returns `Err(current)` if your closure returns `None`.

```rust
fn allocate_new_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    NEXT.fetch_add(1, Relaxed)
}
```

- `[05-anti-patterns]` Overflow. `fetch_add` wraps silently on release builds. If IDs must not wrap, use a CAS loop that panics on overflow or use `fetch_update` with a `None` return to stop.

```rust
fn allocate_new_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    NEXT.fetch_update(Relaxed, Relaxed, |n| n.checked_add(1))
        .expect("ID overflow")
}
```

### 2.8 `compare_exchange` vs `compare_exchange_weak` `[04-design-patterns]` `[09-performance]`

- `compare_exchange(expected, new, success, failure) -> Result<T, T>`: atomically sets to `new` if current == `expected`; on success returns `Ok(expected)`, on failure returns `Err(actual)`.
- `compare_exchange_weak`: same semantics but **may spuriously fail** even when the current value equals `expected`. On ARM and other LL/SC (load-linked/store-conditional) platforms this corresponds to the native `LDREX/STREX` pair, which can fail due to external events (interrupts, context switches) even without contention. On x86 the two are equivalent (native `lock cmpxchg`).
- **Rule**: inside a retry loop, always prefer `compare_exchange_weak`. It avoids an inner LL/SC retry loop the hardware would otherwise do.
- Outside a loop (single-shot CAS), use `compare_exchange`.
- `success` ordering applies on success (and covers the entire operation); `failure` ordering applies on failure (just a load). `failure` must be `Acquire`, `Relaxed`, or `SeqCst` and cannot be stronger than `success`.

```rust
fn increment(v: &AtomicU32) {
    let mut cur = v.load(Relaxed);
    loop {
        let new = cur + 1;
        match v.compare_exchange_weak(cur, new, Relaxed, Relaxed) {
            Ok(_) => return,
            Err(observed) => cur = observed,
        }
    }
}
```

### 2.9 Example: One-time Initialization via CAS `[04-design-patterns]`

```rust
fn get_key() -> u64 {
    static KEY: AtomicU64 = AtomicU64::new(0);
    let k = KEY.load(Relaxed);
    if k == 0 {
        let new = generate_random_key();
        match KEY.compare_exchange(0, new, Relaxed, Relaxed) {
            Ok(_) => new,
            Err(actual) => actual, // someone else won; use theirs
        }
    } else { k }
}
```

- This works with `Relaxed` because `k` is a single `u64`, not a pointer to further state.
- `[05-anti-patterns]` If you instead stored `*mut T` pointing to a heap `T`, `Relaxed` is UB — other threads might load the pointer before the `T` fields are visible. Use `Release`/`Acquire`.

### 2.10 Notes on `AtomicPtr` and `AtomicUsize` for Tagged Pointers

- `AtomicPtr<T>` offers pointer atomic operations. Cast to `AtomicUsize` for bit-tagging in lock-free structures (store low bits as flags since pointers are aligned).
- `fetch_ptr_add` / `fetch_ptr_sub` exist on `AtomicPtr` (stable 1.86+).
- Tagged pointer pitfall: ensure alignment reserves enough bits. `repr(align(8))` reserves 3 bits.

---

## Chapter 3 — Memory Ordering

This is the densest chapter. The mental model is: every atomic creates **happens-before** edges in the program's execution; the compiler and CPU can reorder anything as long as these edges are preserved.

### 3.1 The Reordering Problem `[09-performance]`

- Compilers reorder for optimization; CPUs reorder for pipelining; caches defer writes.
- Rust's model inherits from C++20. Any observable reorder that contradicts the source must be blocked by ordering.

### 3.2 Happens-Before `[02-language-rules]`

- Within one thread: source order = happens-before order (program order).
- `thread::spawn`: the spawning statement happens-before everything the spawned thread does.
- `thread::join`: everything the spawned thread does happens-before `join` returns in the parent.
- **Release store synchronizes-with an Acquire load that reads from it** — this creates a cross-thread happens-before edge: everything before the Release store happens-before everything after the Acquire load.
- Happens-before is transitive.

```rust
static A: AtomicU64 = AtomicU64::new(0);
static B: AtomicU64 = AtomicU64::new(0);

// Thread 1
A.store(10, Relaxed);
B.store(20, Relaxed);

// Thread 2
let b = B.load(Relaxed);
let a = A.load(Relaxed);
// Observing b == 20 does NOT imply a == 10 under Relaxed.
```

With `Release`/`Acquire` on `B`'s store/load respectively, observing `b == 20` would imply `a == 10`.

### 3.3 Relaxed `[02-language-rules]`

- Gives atomicity only.
- Guarantees **total modification order**: for each atomic variable, every thread agrees on the sequence of values it took. Two threads cannot see two different histories of the same variable. (But there is no cross-variable ordering.)
- Sufficient for counters (only the value matters), stats, stop flags that don't gate other state.

### 3.4 Release & Acquire `[02-language-rules]`

- `Release` store on `X` + `Acquire` load on `X` that reads the Release-stored value = happens-before edge.
- Once edge is established, **all** writes before the Release are visible to **all** reads after the Acquire.
- Example: publishing a pointer.

```rust
use std::sync::atomic::{AtomicPtr, Ordering::*};
static PTR: AtomicPtr<Data> = AtomicPtr::new(std::ptr::null_mut());

// Producer
let boxed = Box::new(Data::compute());
PTR.store(Box::into_raw(boxed), Release);

// Consumer
let p = PTR.load(Acquire);
if !p.is_null() { unsafe { dbg!(&*p); } }
```

- The `Release` guarantees that the data writes that constructed `Data` are visible after the `Acquire` load.
- `AcqRel`: used on RMW ops that both publish and consume (e.g. `compare_exchange` when taking ownership of a slot).

### 3.5 Release-Acquire Caveat: Causality Chain `[04-design-patterns]`

- A `Relaxed` RMW sits in the modification order and "carries" previously-released data. If thread A `Release`-stores `X=1`, thread B does `X.fetch_add(0, Relaxed)` and observes 1, thread C `Acquire`-loads `X` and reads B's value — thread C still gets the synchronization from A because of the release sequence. The rule in the book: **release sequence is made of RMWs**, which propagate the happens-before.
- Mental shortcut: Release/Acquire only pairs when the Acquire load reads a value written by some Release store, possibly through a chain of RMW ops.

### 3.6 SeqCst (Sequentially Consistent) `[02-language-rules]` `[09-performance]`

- All SeqCst operations form a single total order consistent with every thread's program order.
- Only ordering that prevents IRIW (independent reads of independent writes) anomalies.
- Expensive: on x86 a `SeqCst` store is a full `MFENCE` or `xchg`; on ARM each SeqCst op is a `DMB ISH` fence.
- **Use SeqCst only if Release/Acquire is genuinely insufficient.** Most patterns do not need it. The book's mantra: if you're reaching for SeqCst, prove you cannot express the pattern with Release/Acquire + a well-designed data dependency.

Classic case that needs SeqCst (the book discusses): Dekker-like mutual exclusion where thread A stores `a=1` then reads `b`, and thread B stores `b=1` then reads `a`, and at least one must see the other's write. With Release/Acquire alone, both could see zero.

### 3.7 Fences `[08-unsafe-and-ffi]`

- `std::sync::atomic::fence(order)` — a standalone memory barrier.
- `fence(Release)`: upgrades subsequent Relaxed stores into Release stores for the purpose of a happens-before edge.
- `fence(Acquire)`: upgrades prior Relaxed loads into Acquire loads.
- Fences are coarser than the equivalent per-op orderings and typically used to batch synchronization (e.g. one Acquire fence after checking several Relaxed flags).
- `compiler_fence(order)`: prevents compiler reordering only, no CPU barrier. Use for single-threaded signal handlers.

Canonical use in an Arc-like destructor:

```rust
if old_count == 1 {
    // ensure all prior Relaxed decrements are visible
    std::sync::atomic::fence(Acquire);
    // drop the data
}
```

The matching `fetch_sub(1, Release)` on the decrement side + `fence(Acquire)` before freeing means all previous users' writes are visible before drop.

### 3.8 Out-of-Thin-Air Values `[02-language-rules]`

- The C++/Rust model technically permits "out-of-thin-air" reads in some Relaxed + dependency loops. In practice no real CPU exhibits them and Rust does not mandate them.
- Concretely: don't rely on causal-dependency arguments with Relaxed only. Add Release/Acquire if correctness depends on ordering.

### 3.9 Total Modification Order `[02-language-rules]`

- Even with `Relaxed`, every atomic variable has a single global modification order observed identically by all threads. You cannot see `x == 2` then later `x == 1` if another thread wrote `1, 2, 3`.
- Multiple variables: no such global order under Relaxed.

### 3.10 When to Use Each Ordering (Cheat Sheet) `[02-language-rules]`

- **Relaxed**: counters, stats, flags that do not gate other memory access; fast-path existence checks.
- **Release** (store / RMW): publish data that other threads may read after loading this.
- **Acquire** (load / RMW): consume a published flag/pointer; subsequent reads see producer's prior writes.
- **AcqRel** (RMW): both publish and consume — e.g. taking ownership of a slot, CAS for lock acquisition.
- **SeqCst**: you need a global order across multiple atomics. Rare; reach for only when you've proven Release/Acquire is insufficient.

### 3.11 Example: Release/Acquire Lock `[04-design-patterns]`

- Lock: `compare_exchange(false, true, Acquire, Relaxed)`. Success orders subsequent reads of protected data.
- Unlock: `store(false, Release)`. Ensures prior writes are visible to the next Acquire.
- See Chapter 4 spin lock.

### 3.12 Common Ordering Mistakes `[05-anti-patterns]`

- Using `Relaxed` when publishing a pointer or any data that must be visible. Classic UB.
- Using `Acquire` on a store or `Release` on a load — these are nonsensical and panic at compile/runtime in the std API (only valid orderings are accepted per op).
- Using `SeqCst` for everything "to be safe" — performance tax with no correctness gain over Release/Acquire in most cases.
- Forgetting the `failure` ordering on `compare_exchange` must be ≤ `success` and ≠ `Release`/`AcqRel`.
- Assuming that ordering in one thread affects ordering in another without a synchronizing pair. Orderings are always about the edge created between two ops on **the same atomic**.

---

## Chapter 4 — Building Our Own Spin Lock `[08-unsafe-and-ffi]` `[04-design-patterns]`

### 4.1 Minimal Spin Lock

```rust
use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering::{Acquire, Release}};
use std::hint::spin_loop;

pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}

pub struct Guard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> SpinLock<T> {
    pub const fn new(v: T) -> Self {
        Self { locked: AtomicBool::new(false), value: UnsafeCell::new(v) }
    }

    pub fn lock(&self) -> Guard<'_, T> {
        while self.locked.swap(true, Acquire) {
            spin_loop();
        }
        Guard { lock: self }
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { unsafe { &*self.lock.value.get() } }
}
impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.lock.value.get() } }
}
impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) { self.lock.locked.store(false, Release); }
}
```

### 4.2 Key Points

- `swap(true, Acquire)`: atomically tries to take the lock. Acquire ensures subsequent data reads do not move before the acquisition.
- `store(false, Release)`: unlock. Release ensures prior data writes are visible to the next Acquire.
- `spin_loop()` (`std::hint::spin_loop`) hints CPU to back off (PAUSE on x86) during busy-wait.
- `unsafe impl<T: Send> Sync`: we guarantee thread-safe shared access via the lock. `T: Send` because we hand out `&mut T` to other threads via the guard. We do **not** require `T: Sync` because access is serialized.
- `Guard` is neither `Send` nor `Sync` by default. The book makes it `Send`/`Sync` as in std's `MutexGuard` (`MutexGuard` is `!Send`, `Sync` if `T: Sync`).
- `Guard` has a lifetime tied to `&'a SpinLock<T>` so you cannot outlive the lock.
- `const fn new` so it can be used in a `static`.

### 4.3 Why Not a Bare `Cell<T>` Inside

- `Cell<T>` is `!Sync`. `UnsafeCell<T>` is the only way to expose mutation through `&T` to another thread — you provide the synchronization.

### 4.4 Pitfalls `[05-anti-patterns]`

- Using `Relaxed` for `swap` / `store`: data race UB because reads inside the critical section can be reordered out.
- Busy-waiting without `spin_loop()`: worse perf; may cause thermal throttle; no hyperthreading yield.
- Un-bounded spin: on user-space contention, prefer a sleep/park-based mutex (Ch. 9) after a few hundred spins.

---

## Chapter 5 — Building Our Own Channels

### 5.1 A Naïve Unsafe One-Shot Channel (teaching example)

```rust
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, Ordering::{Acquire, Release}};

pub struct Channel<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
}
unsafe impl<T: Send> Sync for Channel<T> {}

impl<T> Channel<T> {
    pub const fn new() -> Self {
        Self { message: UnsafeCell::new(MaybeUninit::uninit()), ready: AtomicBool::new(false) }
    }
    /// Safety: only call once.
    pub unsafe fn send(&self, m: T) {
        (*self.message.get()).write(m);
        self.ready.store(true, Release);
    }
    pub fn is_ready(&self) -> bool { self.ready.load(Acquire) }
    /// Safety: only call after `is_ready()`, and only once.
    pub unsafe fn receive(&self) -> T {
        (*self.message.get()).assume_init_read()
    }
}
```

- `MaybeUninit<T>` is the idiomatic way to hold a "not yet initialized" `T`. Reading it before init is UB.
- `ready` is the handshake: Release store on sender, Acquire load on receiver → all writes in `write(m)` happen-before the receive.
- The `send`/`receive` are `unsafe`: the caller proves the protocol (one send, one receive, receive only after ready).

### 5.2 Safety via Runtime Checks

Upgrade by inspecting `ready` flags and panicking on misuse:

```rust
pub fn send(&self, m: T) {
    if self.in_use.swap(true, Relaxed) { panic!("can't send more than one message"); }
    unsafe { (*self.message.get()).write(m); }
    self.ready.store(true, Release);
}
pub fn receive(&self) -> T {
    if !self.ready.swap(false, Acquire) { panic!("no message available"); }
    unsafe { (*self.message.get()).assume_init_read() }
}
```

- `in_use` flag prevents double-send; `swap(false)` in receive prevents double-receive.

### 5.3 Safety via Types (Send/Receive as Separate Types)

- Split the API: `Sender<T>` and `Receiver<T>` are **non-Clone, non-Copy** owned halves that consume themselves on use. `channel()` returns the pair referencing the same `Arc<Channel<T>>`.
- Use a builder pattern so statically you can call `send` once and `receive` once.

```rust
use std::sync::Arc;

pub struct Sender<T> { inner: Arc<Inner<T>> }
pub struct Receiver<T> { inner: Arc<Inner<T>> }

struct Inner<T> { message: UnsafeCell<MaybeUninit<T>>, ready: AtomicBool }
unsafe impl<T: Send> Sync for Inner<T> {}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let a = Arc::new(Inner {
        message: UnsafeCell::new(MaybeUninit::uninit()),
        ready: AtomicBool::new(false),
    });
    (Sender { inner: a.clone() }, Receiver { inner: a })
}

impl<T> Sender<T> {
    pub fn send(self, m: T) { // consumes self -> single send
        unsafe { (*self.inner.message.get()).write(m); }
        self.inner.ready.store(true, Release);
    }
}
impl<T> Receiver<T> {
    pub fn is_ready(&self) -> bool { self.inner.ready.load(Relaxed) }
    pub fn receive(self) -> T {
        if !self.inner.ready.load(Acquire) { panic!("not ready"); }
        unsafe { (*self.inner.message.get()).assume_init_read() }
    }
}
```

- `Sender::send(self, ...)` by value → compiler enforces "send called at most once".
- Drop impl on `Inner` must call `assume_init_drop()` on the message if `ready == true` to avoid leaking.

### 5.4 Borrowing-Based Oneshot Channel

- Alternative: instead of `Arc`, tie `Sender` / `Receiver` to `&Channel<T>` with a lifetime — and use `thread::scope` to let them live on the stack. Avoids `Arc` allocation; all zero-cost when you know the lifetime.

```rust
pub struct Channel<T> { /* as before */ }
pub struct Sender<'a, T> { c: &'a Channel<T> }
pub struct Receiver<'a, T> { c: &'a Channel<T> }

impl<T> Channel<T> {
    pub fn split(&mut self) -> (Sender<'_, T>, Receiver<'_, T>) {
        *self = Channel::new();
        (Sender { c: self }, Receiver { c: self })
    }
}
```

- `&mut self` in `split` ensures no other borrow is active. Both halves borrow the same `&Channel<T>` immutably.

### 5.5 Blocking Receive via Thread Parking

- Add an `Option<Thread>` in the channel to park/unpark on send. Receiver writes its `Thread` handle before loading `ready`; sender reads the handle after storing the message and unparks.

```rust
pub struct Inner<T> {
    message: UnsafeCell<MaybeUninit<T>>,
    ready: AtomicBool,
    receiver_thread: UnsafeCell<Option<Thread>>,
}

impl<T> Sender<'_, T> {
    pub fn send(self, m: T) {
        unsafe { (*self.c.message.get()).write(m); }
        self.c.ready.store(true, Release);
        if let Some(t) = unsafe { (*self.c.receiver_thread.get()).take() } {
            t.unpark();
        }
    }
}
impl<T> Receiver<'_, T> {
    pub fn receive(self) -> T {
        unsafe { *self.c.receiver_thread.get() = Some(thread::current()); }
        while !self.c.ready.swap(false, Acquire) { thread::park(); }
        unsafe { (*self.c.message.get()).assume_init_read() }
    }
}
```

- `UnsafeCell<Option<Thread>>` is OK here because only the receiver writes it, and only before setting ready. The atomic `ready` ensures the sender observes the thread-handle write.

### 5.6 MPSC / SPSC / MPMC — general channels

- Ring buffers with head/tail atomics. Producer increments head on push; consumer increments tail on pop.
- Bounded: use modular indices; handle full/empty edge cases with an extra "is_full" bit.
- Pros: very fast, lock-free. Cons: correct memory ordering is hard; bugs cause UB.
- The book shows patterns but recommends `crossbeam-channel` or `flume` in production.

### 5.7 Channel Pitfalls `[05-anti-patterns]`

- Forgetting to run the destructor for a partially-initialized slot: leaks drop.
- Using `Relaxed` on the ready flag: data race on the message (UB).
- Parking without a pre-check: lost wake-up if send beats park. Always write thread-handle before loading/checking state, then re-check after parking (the loop).

### 5.8 Mutex + `Condvar` channel (book’s “simplest” MPMC) `[04-design-patterns]`

- **Pattern**: `Mutex<VecDeque<T>>` + `Condvar`. `send` locks, `push_back`, `notify_one`. `receive` locks, `pop_front` in a **`while let None` loop** with `wait` on empty queue.
- **Why it’s nice**: no `unsafe`, no manual `Send`/`Sync` proofs — the compiler derives sharing from `Mutex` + `Condvar`.
- **Costs** (from the book): every operation serializes on one mutex; `VecDeque` growth can block everyone during reallocation; queue can **grow without bound** if producers outpace consumers — may need explicit bounds or back-pressure elsewhere.

```rust
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

pub struct Channel<T> {
    queue: Mutex<VecDeque<T>>,
    ready: Condvar,
}

impl<T> Channel<T> {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        }
    }
    pub fn send(&self, msg: T) {
        self.queue.lock().unwrap().push_back(msg);
        self.ready.notify_one();
    }
    pub fn receive(&self) -> T {
        let mut g = self.queue.lock().unwrap();
        loop {
            if let Some(m) = g.pop_front() {
                return m;
            }
            g = self.ready.wait(g).unwrap();
        }
    }
}
```

### 5.9 `is_ready` with `Relaxed` vs `receive` with `Acquire` `[02-language-rules]`

- After `receive` (or another method) does an **Acquire** load/swap on `ready`, you may lower **`is_ready`** to **`Relaxed`** if it is only “hint for polling UI.”
- **Reason**: **total modification order** on `ready` means once any thread observes `true`, a later Acquire in `receive` on the same atomic cannot “go backwards” to not seeing the store — you cannot get `is_ready() == true` and then a panicking `receive` because of ordering choice on `is_ready` alone.

### 5.10 `Drop` for `MaybeUninit` storage `[05-anti-patterns]`

- If a message is **sent but never received**, `MaybeUninit<T>` will **not** drop `T` automatically — implement `Drop` on the channel `Inner` that, if `ready` was set and the value not taken, runs `assume_init_drop` or manual drop logic.
- Leaking is memory-safe in Rust but usually undesirable for channel APIs.

### 5.11 Splitting ownership: `Sender` / `Receiver` consuming methods `[04-design-patterns]`

- The book progresses to **type-state** APIs: `send(self, m)` and `receive(self) -> T` **consume** the endpoint so the compiler proves **at most one** send/receive.
- `Arc` to both halves is still one allocation; scoped-borrow variants avoid `Arc` when lifetimes allow (`thread::scope`).

---

## Chapter 6 — Building Our Own Arc `[08-unsafe-and-ffi]` `[04-design-patterns]` `[05-anti-patterns]`

### 6.1 Basic Arc — No Weak

```rust
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering::*, fence};
use std::ptr::NonNull;

struct ArcData<T> {
    ref_count: AtomicUsize,
    data: T,
}

pub struct Arc<T> { ptr: NonNull<ArcData<T>> }

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Arc<T> {
    pub fn new(data: T) -> Self {
        let ad = Box::new(ArcData { ref_count: AtomicUsize::new(1), data });
        Arc { ptr: NonNull::from(Box::leak(ad)) }
    }
    fn data(&self) -> &ArcData<T> { unsafe { self.ptr.as_ref() } }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        if self.data().ref_count.fetch_add(1, Relaxed) > usize::MAX / 2 {
            std::process::abort(); // overflow guard
        }
        Arc { ptr: self.ptr }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.data().data }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        if self.data().ref_count.fetch_sub(1, Release) == 1 {
            fence(Acquire);
            unsafe { drop(Box::from_raw(self.ptr.as_ptr())); }
        }
    }
}
```

### 6.2 Why These Orderings

- **Clone**: `Relaxed` suffices because cloning only increments a counter; no other memory synchronization is needed for the cloner itself.
- **Drop**: `Release` on `fetch_sub` ensures prior uses (data mutations, if any via interior mutability) are visible. When decrement returns 1, we are the last — `fence(Acquire)` pairs with all other threads' `Release` decrements to ensure we observe all their prior writes before freeing.
- Could use `AcqRel` on the last decrement directly, but the `Release` + `fence(Acquire)` split is the canonical optimization: only the last drop pays for the Acquire barrier.
- `process::abort` on overflow: if 2^63 references exist, we assume malicious code is trying to wrap the counter and UAF.

### 6.3 Weak References

- Add a `weak` count, splitting ownership:
  - Strong count: number of `Arc`s sharing ownership of `data`.
  - Weak count: number of `Weak`s + 1 if strong > 0 (to keep the ArcData alive while any Arc exists).
- When strong reaches 0, we drop the `T` (not the ArcData). ArcData itself is dropped when weak reaches 0.

```rust
struct ArcData<T> {
    // number of Arcs
    data_ref_count: AtomicUsize,
    // number of Weak (plus 1 if there are any Arcs)
    alloc_ref_count: AtomicUsize,
    // None once data is dropped
    data: UnsafeCell<std::mem::ManuallyDrop<T>>,
}
```

- `Weak::upgrade`: CAS loop `data_ref_count` from nonzero to +1; fail if 0. `Relaxed` success; check for zero.
- `Arc::downgrade`: increment `alloc_ref_count`; `Relaxed`.
- Strong drop: if `data_ref_count` goes to 0, drop the `T` (need `Acquire` + `Release`), then decrement `alloc_ref_count` by 1.
- Weak drop: decrement `alloc_ref_count`; if it goes to 0, dealloc the `ArcData` box.

### 6.4 Cycle Avoidance `[04-design-patterns]`

- `Arc<T>` with interior mutability that stores another `Arc<T>` can form a cycle → memory leak, not UB.
- Pattern: Parent holds `Arc<Child>`; Child holds `Weak<Parent>`. The Weak breaks the cycle.
- Rc/Arc never collect cycles. Cycle-safe alternative: `Gc<T>` (`gc` crate) or a typed arena.

### 6.5 `Arc::make_mut` and Copy-on-Write

- `Arc::make_mut(&mut Arc<T>) -> &mut T`: if strong count is 1 and weak count is 1, returns exclusive ref. Otherwise clones `T` into a fresh Arc. Classic COW pattern.
- `Arc::get_mut(&mut Arc<T>) -> Option<&mut T>`: returns `Some` only if strong == 1 and weak == 0.

### 6.6 ABA Pitfall `[05-anti-patterns]`

- In any CAS loop that compares pointer values, an ABA problem arises when a pointer P is replaced by Q and then reinstated as P. CAS succeeds thinking nothing changed, but the object behind P may be different.
- Mitigation: **hazard pointers**, **epoch-based reclamation** (`crossbeam-epoch`), or tagged pointers (use low bits as a counter that's incremented on every swap).

### 6.7 Common Arc Mistakes `[05-anti-patterns]`

- Double drop: forgetting that `Box::from_raw` consumes the pointer; never read `self.ptr` after converting to Box in drop.
- Releasing memory before the `fence(Acquire)`: UB because other threads' writes may not be visible.
- Using `Acquire` on fetch_sub in drop for every decrement: wastes fence; use `Release` + single `fence(Acquire)` on last.
- Bumping refcount when strong is 0 in Weak::upgrade via `fetch_add`: wrong — use CAS to avoid resurrection.

---

## Chapter 7 — Understanding the Processor `[09-performance]`

### 7.1 Processor ISA Basics

- x86-64: a **strong-memory** model (TSO — total store order). Loads are never reordered with later loads; stores never with later stores; stores can be buffered and reordered with later loads only (store-load). That's why `SeqCst` store needs a full `mfence` (or `xchg`) on x86.
- ARM64 (AArch64): a **weak-memory** model. Almost any reordering allowed; explicit acquire/release/barrier needed. Uses `ldar` (load-acquire), `stlr` (store-release), `dmb ish` / `dmb sy` for fences.

### 7.2 Cache Lines

- A cache line is 64 bytes on nearly every modern CPU. On some ARM server chips, 128 bytes. Pointer Authentication/TLB tagging can change effective alignment.
- Atomic operations are **cache-line-granular**. Two unrelated atomics in the same line contend — called **false sharing**.
- Prevent with `#[repr(align(64))]` on hot atomics or a dedicated padded wrapper.

```rust
#[repr(align(64))]
struct Padded<T>(T);

struct Counters {
    a: Padded<AtomicU64>,
    b: Padded<AtomicU64>,
}
```

- The book recommends `crossbeam::utils::CachePadded<T>`.

### 7.3 False Sharing `[05-anti-patterns]` `[09-performance]`

- Two counters on the same cache line, each mutated by a different thread, ping-pong the line between cores — often 10-100x slower than when padded.
- Detection: profiler will show high cache-miss rate or stalls on the atomics; benchmark with padding added to confirm.
- Rule of thumb: per-thread counters/flags should be on their own cache line.

### 7.4 True Sharing — Inherent Contention

- Multiple threads writing the same atomic always contends; padding doesn't help.
- Reduction patterns: per-thread local counters aggregated on read/at end.
- Skewed hot-spots: shard the structure (one counter per CPU → read sums them).

### 7.5 Atomic Instructions

- x86 `lock xadd` (fetch_add), `lock cmpxchg` (CAS), `xchg` (swap, implicit lock).
- ARM uses LL/SC pair: `ldxr` / `stxr`. This is why `compare_exchange_weak` is better on ARM — one LL/SC attempt per loop iteration, no hardware retry.
- ARMv8.1+: LSE (Large System Extensions) — `ldadd`, `cas` as single instructions, much faster under contention.

### 7.6 Fences Compilation

- On x86:
  - `Relaxed`, `Release`, `Acquire`, `AcqRel` loads/stores = plain `mov` (because of TSO).
  - `SeqCst` store = `xchg` or `mov` + `mfence`.
  - Fences `Release`/`Acquire` = compiler barrier only.
  - `SeqCst` fence = `mfence`.
- On ARM:
  - `Relaxed` = `ldr`/`str`.
  - `Acquire` load = `ldar`. `Release` store = `stlr`.
  - `AcqRel` RMW = `ldaxr`/`stlxr` pair.
  - `SeqCst` typically = `dmb ish` plus the op.

### 7.7 Allocator Awareness

- Small allocations can end up on the same cache line. Padding or `Box::leak` with explicit alignment helps.
- `jemalloc` / `mimalloc` sometimes colocate; check `repr(align(64))` at the type level.

### 7.8 Instruction-Level Hints

- `std::hint::spin_loop()` emits PAUSE on x86, YIELD on ARM — reduces power and helps SMT siblings.
- `std::hint::black_box` prevents LLVM from optimizing away benchmark code.

### 7.9 Why the compiler still cares (book, Ch. 7) `[02-language-rules]` `[09-performance]`

- On **x86-64** and **ARM64**, a **`Relaxed` load/store** on an `AtomicI32` often compiles to the **same machine instruction** as a plain load/store on `i32` — the CPU already performed a single indivisible access at that granularity.
- **Rust’s distinction** is not meaningless: the compiler **must not** tear, duplicate, or merge atomic accesses the way it can for non-atomic `&mut` in single-threaded reasoning; and data-race rules still forbid mixing atomic and non-atomic concurrent accesses to the same memory.
- **Non-atomic read-modify-write** on RISC (ARM) is typically **3 instructions** (load / op / store) → not atomic without explicit LL/SC or atomics. On **x86**, a single `add [mem]` still isn’t a safe substitute for `fetch_add` in concurrent code — micro-ops and preemption break atomicity across threads.
- **`compare_exchange_weak` on ARM**: maps to **LL/SC** (`ldxr`/`stxr`); spurious failure is normal → use in **loops**. On **x86**, `lock cmpxchg` rarely spuriously fails → `compare_exchange` vs `weak` often equivalent at hardware level, but **idiom** stays the same: loop uses `weak`.

### 7.10 Compiler Explorer / `cargo-show-asm` `[09-performance]`

- The book recommends **Godbolt** (`rustc --emit=asm`, `-O`, `--target=x86_64-unknown-linux-musl` vs `aarch64-unknown-linux-musl`) to connect Rust orderings to emitted instructions — essential when tuning hot atomics or auditing codegen for weak vs strong CAS.

---

## Chapter 8 — Operating System Primitives `[08-unsafe-and-ffi]`

### 8.1 Why OS Primitives

- Spin locks are bad under contention — a spinning thread wastes its quantum. Beyond a few hundred iterations, we must sleep the thread.
- OS-level: **futex** (Linux), **WaitOnAddress** (Windows), **ulock** (macOS darwin syscalls). All share the same idea — atomically check a value in user memory and sleep if it matches.

### 8.2 Linux futex API

- `futex(addr, FUTEX_WAIT, expected, timeout, ...)`: if `*addr == expected`, sleep. Otherwise return `EAGAIN`.
- `futex(addr, FUTEX_WAKE, n)`: wake up to `n` threads waiting on `addr`.
- The **atomic check** of `*addr == expected` inside `FUTEX_WAIT` is what avoids the lost-wakeup race: another thread must store the new value + call `FUTEX_WAKE`, and if our thread hasn't yet slept, the kernel sees `*addr != expected` and returns.

### 8.3 Windows & macOS Counterparts

- Windows: `WaitOnAddress(addr, comparand, size, timeout)` and `WakeByAddressSingle`/`WakeByAddressAll`. Supports sizes 1, 2, 4, 8.
- macOS: `__ulock_wait`, `__ulock_wake` (private, not API-stable; std/parking_lot use them).

### 8.4 The `atomic-wait` Crate

- The book uses `atomic-wait` (Mara Bos's crate): `wait(a: &AtomicU32, expected: u32)`, `wake_one(a: &AtomicU32)`, `wake_all(a: &AtomicU32)`. Abstracts over OS.

### 8.5 Futex-Based Mutex Sketch

```rust
use atomic_wait::{wait, wake_one};
use std::sync::atomic::{AtomicU32, Ordering::*};

// 0 = unlocked, 1 = locked no waiters, 2 = locked with waiters
pub struct Mutex { state: AtomicU32 }

impl Mutex {
    pub fn lock(&self) {
        if self.state.compare_exchange(0, 1, Acquire, Relaxed).is_err() {
            while self.state.swap(2, Acquire) != 0 {
                wait(&self.state, 2);
            }
        }
    }
    pub fn unlock(&self) {
        if self.state.swap(0, Release) == 2 {
            wake_one(&self.state);
        }
    }
}
```

- Three states encode "fast path" (no waiter) vs "slow path" (must wake).
- Unlock only calls `wake_one` when the state was 2 → fewer syscalls.

### 8.6 Scalable Wait Queues

- Real implementations (std's `parking_lot`) maintain per-address hashed wait queues in user space, reducing kernel entries.

---

## Chapter 9 — Building Our Own Locks

### 9.1 Mutex (Ch. 8 recap + optimizations)

- Three-state + futex as above.
- `MutexGuard`: Drop calls unlock. Must not be `Send` on Linux.
- Use `lock_api` (crate) to implement `RawMutex` and get `MutexGuard`, poison wrappers, etc., for free.

### 9.2 Condvar Implementation

- State: `counter: AtomicU32` incremented on each `notify_*`, and `num_waiters: AtomicUsize` for the "skip wake if none" optimization.
- `wait(guard)`: read `counter`, unlock the mutex, futex-wait on `counter` with expected = read value, relock the mutex.
- `notify_one`: increment `counter` (with `Release`), `wake_one(counter)`.
- `notify_all`: increment `counter`, `wake_all(counter)`.
- Correctness: any wake issued after the `counter` read will cause wait to return immediately (counter != expected).

```rust
pub struct Condvar {
    counter: AtomicU32,
    num_waiters: AtomicUsize,
}

impl Condvar {
    pub fn wait<'a, T>(&self, mut g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        self.num_waiters.fetch_add(1, Relaxed);
        let c = self.counter.load(Relaxed);
        let m = g.mutex; // hypothetical: access the parent mutex
        drop(g);
        wait(&self.counter, c);
        self.num_waiters.fetch_sub(1, Relaxed);
        m.lock()
    }
    pub fn notify_one(&self) {
        if self.num_waiters.load(Relaxed) > 0 {
            self.counter.fetch_add(1, Relaxed);
            wake_one(&self.counter);
        }
    }
    pub fn notify_all(&self) {
        if self.num_waiters.load(Relaxed) > 0 {
            self.counter.fetch_add(1, Relaxed);
            wake_all(&self.counter);
        }
    }
}
```

- Waiters-count optimization: skip the syscall when no one's waiting. Safe because `fetch_add` above is `Relaxed` and we only use it to skip the syscall — correctness is still guaranteed by the counter increment.

### 9.3 RwLock Implementation

- State machine: single atomic where the low bits encode writer-held / waiters, and an `AtomicU32` reader count. Or: a single `u32` with reader count and a high bit for writer.
- Reader lock:
  - CAS `readers + 1` if no writer.
  - On contention, wait on the state.
- Writer lock:
  - CAS to a "writer held" sentinel.
  - If readers > 0 or writer held, wait.
- Writer preference vs reader preference: tradeoff — unfair but fast reader-preferred vs starvation-safe writer-preferred. std's current RwLock is platform-dependent; `parking_lot::RwLock` offers both.

### 9.4 Busy/Spin Optimization Hybrid

- Real mutexes spin for N iterations before calling futex. N ~= 100–1000. Saves kernel entries for very short critical sections.

---

## Chapter 10 — Ideas and Inspiration

### 10.1 Semaphores, Barriers, Latches

- Semaphore: atomic `u32` + futex. Decrement on acquire (wait if zero), increment on release (wake_one).
- Barrier: atomic counter; last thread to arrive wakes all (counter wraps on reset).
- Latch: one-shot barrier; simpler than Condvar.

### 10.2 Blocking Queue / Lock-Free Queue

- Michael-Scott queue: linked list with head/tail atomics + tagged pointers to handle ABA.
- SPSC bounded: two atomic indices (head + tail) + padding to avoid false sharing.
- MPMC bounded: bounded ring with seq counters per slot (Vyukov).

### 10.3 Sequence Locks (seqlock)

- Single writer, many readers. Writer bumps an odd counter, writes data, bumps to even. Readers read counter, data, counter again; retry if mismatched or odd.
- Ideal for large read-mostly data where readers shouldn't ever block writers.

### 10.4 Hazard Pointers / Epoch-Based Reclamation

- Safely reclaim memory in lock-free structures when you can't guarantee no other thread is reading.
- `crossbeam-epoch` crate for production use.

### 10.5 Teaching Takeaways

- Always prove ordering; always match Release with Acquire.
- Start with the simplest correct primitive (std's Mutex); optimize only when benchmarks show contention.

---

## Supplement — Chapter 3: Fences, SeqCst, and Common Myths `[02-language-rules]` `[09-performance]`

### Decomposing Release/Acquire with `fence`

- **Release store** `a.store(x, Release)` can be expressed as `fence(Release); a.store(x, Relaxed)` (same-thread ordering).
- **Acquire load** `a.load(Acquire)` can be expressed as `let v = a.load(Relaxed); fence(Acquire);` (use `v` after the fence).
- A **fence is not tied to one atomic variable**: one `fence(Release)` followed by several relaxed stores to `A`, `B`, `C` can synchronize with another thread that loads those atomics with relaxed loads and then executes **one** `fence(Acquire)` — if any load sees the corresponding store, the release fence happens-before the acquire fence.
- **Conditional acquire**: if the null pointer case is hot, `PTR.load(Relaxed)` + `if !p.is_null() { fence(Acquire); ... }` avoids paying for acquire ordering when `p` is null.
- **SeqCst fence vs SeqCst operations**: `fence(SeqCst)` participates in the global `SeqCst` total order. **Important**: a **single** `SeqCst` load or store cannot be split into `Relaxed` + `fence(SeqCst)` the same way Release/Acquire can — the book stresses this distinction.

### SeqCst pattern (Dekker-style) — when `Release`/`Acquire` on one variable is not enough

The book gives a minimal example: two threads each set their own `AtomicBool` to `true`, then load the other’s flag; only if the other is still `false` may they touch shared non-atomic `S`. With **only** `Release`/`Acquire` on those flags, both could observe the other flag as `false` and both access `S` → data race. **`SeqCst`** on those flag ops (or a `SeqCst` fence pattern) forces a single total order so at most one thread proceeds. In practice this pattern is rare; `Mutex` or a single atomic “turn” is usually simpler.

```rust
// Illustrative only — prefer Mutex in production for this mutual exclusion.
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

static A: AtomicBool = AtomicBool::new(false);
static B: AtomicBool = AtomicBool::new(false);
// static mut S: String = ... // guarded by the protocol above
```

### Chapter 3 “Common Misconceptions” (themes from the book)

- **Stronger ordering ≠ faster cross-thread visibility.** The model defines *order*, not *time*. Weaker orderings do not mean “updates might never arrive.”
- **Disabling optimizations (`-C opt-level=0`) does not remove the need for correct ordering** — the compiler may still transform code, and the CPU still has a memory model.
- **Even single-core / in-order CPUs** can require atomics: the compiler’s single-thread reasoning can break code that uses wrong orderings for *intended* cross-thread communication.
- **`Relaxed` is not “free” under contention** — coherence traffic and cache invalidation dominate when multiple cores hammer the same location.
- **`SeqCst` as a “safe default”** is misleading: (1) an algorithm can be wrong regardless of ordering; (2) `SeqCst` documents a *global* dependency on all `SeqCst` ops — harder to review than `Release`/`Acquire` on a specific variable.
- **`SeqCst` does not invent “acquire-store” or “release-load”.** Those combinations are invalid; `Release` applies to stores, `Acquire` to loads.

### Summary list (end of Chapter 3 in the book)

- No single global order of *all* atomics unless you use `SeqCst` (or acceptable weaker patterns).
- **Each** atomic variable still has a **total modification order** — all threads agree on the sequence of values of that variable.
- Happens-before: intra-thread order; spawn/join; unlock/lock mutex; release/acquire pair when acquire-load observes release-store (including via RMW chains).
- Consume ordering would be weaker than acquire but **is not exposed in Rust** — compilers effectively promote it to acquire.

---

## Supplement — Chapter 8: POSIX, movability, and futex intuition `[08-unsafe-and-ffi]`

### Why wrapping `pthread_mutex_t` in Rust was awkward

- C pthread types may be **non-movable** (self-referential or address-sensitive). Rust **moves** values freely → historical Unix `std::sync::Mutex` put the pthread object in a **`Box`** so its address stays fixed (pre–Rust 1.62 on Unix).
- Costs: extra allocation; `Mutex::new` could not be `const`; static mutexes harder.
- **Dropping while locked**: `std::mem::forget(mutex_guard)` leaves the OS mutex locked; destroying a locked `pthread_mutex` is UB. Safe Rust usually prevents this, but `forget` is safe — implementations may try to recover or panic on drop.
- **Recursive locking**: pthread mutex can be configured (`PTHREAD_MUTEX_RECURSIVE`, etc.). Rust’s `std::sync::Mutex` is **not** reentrant — second `lock()` from same thread deadlocks.

### Futex mental model (Linux)

- `FUTEX_WAIT(addr, expected)`: kernel checks `*addr == expected` **atomically** with respect to deciding to sleep — closes the lost-wakeup race when paired with a store + `FUTEX_WAKE`.
- Same idea under Windows **`WaitOnAddress`** / **`WakeByAddress*`**; macOS private ulock APIs used by std / libraries.

### `atomic_wait` / C++20 `atomic_wait` / `atomic_notify`

- The book’s `atomic-wait` crate abstracts “wait on atomic value” across OSes — same conceptual layer C++20 standardized for portable futex-like behavior.

---

## Supplement — Chapter 10: Ideas and Inspiration (expanded) `[04-design-patterns]` `[05-anti-patterns]` `[09-performance]`

### Semaphore

- Up/down (signal/wait) on a counter; binary semaphore (max 1) can mimic mutex or condition-signal patterns.
- **Book warning**: semaphore can be built from `Mutex` + `Condvar`, and mutex can be built from semaphore — **do not** circularly implement one with the other in production (inefficient, confusing).

### RCU (read-copy-update)

- **Read** pointer → **copy** struct → **modify** copy → **CAS** pointer to new allocation. **Reclaiming** the old allocation requires hazard pointers, epoch reclamation (`crossbeam-epoch`), `Arc`, leaking, or GC — readers may still hold the old pointer after the CAS.

### Lock-free linked list

- Insert at head: allocate node, point to current head, CAS head. **Remove** and **concurrent edits** to neighbors need careful ordering; easiest escape hatch: **mutex for writers only** while readers stay lock-free loads.
- Deallocation after removal still matches RCU reclamation problem.

### Queue-based locks

- Maintain **explicit queue of waiters** (e.g. `Thread` handles) instead of relying solely on kernel wait queues; can integrate with **thread parking**. Windows **SRW** locks are related.

### Parking lot–based locks (WebKit → `parking_lot`)

- **Idea**: mutex is tiny (often **one byte** or bits in a pointer); **global hash map** from **address of mutex** → queue of parked threads (“parking lot”).
- Generalizes to **condvar**-like and **futex-like** behavior on OSes without native futex.
- **Origin**: WebKit “Locking in WebKit” (2015); Rust **`parking_lot`** crate implements this family of algorithms — **compact, fast, no poisoning** (see cross-cutting notes for API differences vs `std`).

### Sequence lock (seqlock)

- Writer: odd sequence → mutate data → even sequence. Reader: read seq, read data, read seq; **retry** if odd or mismatch.
- **Memory-model note from the book**: concurrent non-atomic reads and writes to the same bytes are **still UB** in Rust even if you “know” the sequence number protects you logically — mitigations include **atomic per-byte** access (`AtomicPerByte` RFC discussion), documented crates, or keeping data in types that preserve atomicity guarantees.

### Teaching / tooling

- The book encourages creating teaching materials — concurrency education remains sparse relative to need.

---

## Additional Cross-Cutting Notes

### A. Choosing Primitives `[04-design-patterns]` `[09-performance]`

- **Shared counter, no correlation with other data**: `AtomicU64::fetch_add(_, Relaxed)`.
- **One-shot initialization**: `OnceLock<T>` (std) or `LazyLock<T, F>` (std, 1.80+).
- **Shared mutable state with complex invariants**: `Mutex<T>`.
- **Read-heavy shared state**: `RwLock<T>`.
- **Atomic snapshots**: `ArcSwap<T>` (`arc-swap` crate).
- **Producer/consumer pipelines**: `crossbeam-channel` (MPMC) or `tokio::sync::mpsc` in async.
- **Signal work ready**: thread parking (simple) or Condvar (rich predicate).

### B. std vs parking_lot `[09-performance]` `[04-design-patterns]`

- **`parking_lot` design lineage (book, Ch. 10)**: global **parking lot** map from mutex **address** → waiter queue; mutex holds minimal state. Inspired by **WebKit** locking (2015). See crate docs: https://docs.rs/parking_lot/
- `parking_lot::Mutex`:
  - No poisoning (smaller, faster).
  - `lock()` returns `MutexGuard<T>` directly (no `LockResult`).
  - Typically **1 byte** (or few bits) of inline state vs std’s larger inline/`Box` layouts on some platforms.
  - Adaptive: spins briefly, then parks (same *idea* as modern std mutexes on Linux).
  - `MutexGuard` is **not** `Send` (same platform-specific reasoning as std — do not send a held guard across threads).
- `std::sync::Mutex`:
  - Poisoning: `lock()` returns `LockResult`.
  - **Linux (1.62+)**: futex-based user-space fast path + kernel wait — conceptually close to “parking lot + futex,” but different implementation details than `parking_lot`.
  - Windows: `SRWLock`; macOS: unfair lock / pthread layer.
- **When to pick `parking_lot`**:
  - Measured contention / allocation overhead matters; you want **smaller** mutex types or **optional** fair `RwLock` policies.
  - You explicitly want **no poisoning** behavior.
- **When to stay on `std`**:
  - You want **poisoning** as a signal for inconsistent state after panics.
  - You minimize dependencies; std’s mutex is “good enough” on modern Linux for many apps.
- **Neither replaces**: correct `Ordering` on your **own** atomics; both libraries only implement their internal synchronization.

### C. Lock Poisoning `[02-language-rules]`

- Pattern to handle poisoning gracefully: `let g = mutex.lock().unwrap_or_else(|e| e.into_inner());` — ignores poisoning, uses the guard anyway.
- For libraries: re-export `PoisonError::into_inner` paths; propagate `Result` only when necessary.
- `parking_lot::Mutex` has no poisoning; a panic with guard held simply releases and the next locker sees normal state.

### D. Holding Locks Across `.await` `[07-async-concurrency]` `[05-anti-patterns]`

- Holding `std::sync::MutexGuard` across `.await` is a lint (`clippy::await_holding_lock`) and can deadlock a multi-threaded executor or starve the executor on a single-thread runtime.
- Use `tokio::sync::Mutex` for async-aware mutex whose guard is `Send` across awaits and yields to the scheduler on contention.
- Better: structure code so lock is released before any `.await`. Pattern:

```rust
let data = {
    let g = state.lock().unwrap();
    g.snapshot()
}; // drop guard
handle_data(data).await;
```

### E. Send/Sync Invariants to Remember `[02-language-rules]`

| Type | Send | Sync | Notes |
|------|------|------|-------|
| `&T` | iff `T: Sync` | iff `T: Sync` | |
| `&mut T` | iff `T: Send` | iff `T: Sync` | |
| `Rc<T>` | no | no | Single-thread only |
| `Arc<T>` | iff `T: Send + Sync` | iff `T: Send + Sync` | |
| `Cell<T>` | iff `T: Send` | no | |
| `RefCell<T>` | iff `T: Send` | no | |
| `Mutex<T>` | iff `T: Send` | iff `T: Send` | No Sync bound needed |
| `RwLock<T>` | iff `T: Send` | iff `T: Send + Sync` | Readers alias; need Sync |
| `MutexGuard<'_, T>` | no | iff `T: Sync` | Cannot cross threads |
| `UnsafeCell<T>` | iff `T: Send` | no | Manual Sync impl needed |
| `*const T` / `*mut T` | no | no | Manual impl required |
| `JoinHandle<T>` | iff `T: Send` | iff `T: Send` | |

### F. Out-of-Thin-Air Informal Rule

- If two atomics `X`, `Y` start at 0 and:
  - Thread A: `let a = X.load(Relaxed); Y.store(a, Relaxed);`
  - Thread B: `let b = Y.load(Relaxed); X.store(b, Relaxed);`
- In abstract model could produce `a == b == 42` (out of thin air). In practice no platform does this.
- Rust mirrors C++20 and doesn't require hardware/compilers to rule this out, but does forbid you from relying on absurd values. Write code that's correct under *any* allowed reordering.

### G. Release/Acquire Cookbook `[04-design-patterns]`

- **Publish a pointer**: `p.store(ptr, Release)` on producer, `p.load(Acquire)` on consumer. All producer's writes before the store are visible.
- **Take a slot**: `p.compare_exchange(null, ptr, AcqRel, Acquire)` — success acquires and publishes; failure acquires visibility of winner's data.
- **Hand off ownership**: last decrement of Arc uses Release + fence(Acquire).

### H. Testing & Sanitizers `[05-anti-patterns]`

- Rust's `loom` crate: exhaustive permutation tester for lock-free code. Runs your test under many interleavings and orderings.
- ThreadSanitizer (TSAN): compile with `-Zsanitizer=thread` (nightly) or `RUSTFLAGS="-Z sanitizer=thread"`. Detects actual data races at runtime.
- Miri: catches UB in constants/tests; handles atomics with (limited) weak-memory modeling behind `-Zmiri-strict-provenance` and `-Zmiri-disable-isolation`.

### I. False Sharing Benchmark Pattern `[09-performance]`

```rust
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use crossbeam_utils::CachePadded;

struct NotPadded { a: AtomicU64, b: AtomicU64 }
struct Padded { a: CachePadded<AtomicU64>, b: CachePadded<AtomicU64> }
// Threads T1 writes .a, T2 writes .b.
// Benchmark difference: Padded can be 10-50x faster under heavy contention.
```

### J. Reading and Writing Hot Atomics

- Read-heavy hot atomics: cache-line share reads fine.
- Write-heavy atomics: each writer invalidates all readers. Prefer sharded counters aggregated on read.
- `fetch_add(1, Relaxed)` in one thread != `fetch_add(1, Relaxed)` in many threads for perf. Contention → LL/SC retries or cache ping-pong.

### K. `compare_exchange` success/failure ordering rules

- `success` can be any ordering (`Relaxed`, `Acquire`, `Release`, `AcqRel`, `SeqCst`).
- `failure` can be `Relaxed`, `Acquire`, `SeqCst`. Cannot include Release (failure is a load).
- `failure` must be ≤ `success` in "strength".

### L. Mutex Deadlock Patterns `[05-anti-patterns]`

- Lock ordering: always acquire A before B.
- Reentrant locks: std `Mutex` is **not** reentrant. Calling `lock()` twice from the same thread deadlocks. `parking_lot::ReentrantMutex` available but rarely correct.
- Guard drop timing: `if cond { let g = m.lock().unwrap(); ... }` — guard dropped at end of `if`. Use `drop(g)` for explicit release.
- Iterator mutations: `for x in m.lock().unwrap().iter() { ... }` — guard held for the entire loop; if loop body calls code that wants the lock, deadlock.

### M. `parking_lot::Mutex` Patterns

- `mutex.lock()` returns `MutexGuard<T>`; no `LockResult`.
- `try_lock`, `try_lock_for(duration)`, `try_lock_until(instant)` available (not in std for `Mutex`, though `try_lock` is stable).
- `MutexGuard` has a `parking_lot::MutexGuard::unlocked(|| ...)` that drops the lock for the closure and re-locks. Useful inside nested code.
- `RwLock` supports upgradable reads via `upgradable_read() -> RwLockUpgradableReadGuard`.

### N. Building Primitive Safety Checklist `[08-unsafe-and-ffi]`

Before writing `unsafe impl Sync` or `unsafe impl Send`:

1. Identify every way `&self` is used to mutate — must be synchronized by atomics/locks.
2. Verify no destructor ordering hazards — e.g. drop of protected data cannot alias with outstanding handles.
3. Verify drop happens-after all uses. Typical pattern: `Release` on last decrement + `Acquire` on drop.
4. Audit for aliasing XOR mutability: if your type hands out `&mut` via interior mutability, only one handler at a time.
5. Document invariants in a `// SAFETY:` comment on the `unsafe impl` line.

### O. Atomic + Non-atomic Mixed Access — UB `[05-anti-patterns]`

- Rust forbids concurrent non-atomic and atomic access to the same memory location (`AtomicU32` aliased as `*mut u32`). This is a data race.
- If you need "atomic or not" switching (rare), wrap in an enum or use `fence` boundaries with careful ordering.

### P. The "Weak = Non-Owning" Pattern `[04-design-patterns]`

- `Weak<T>` holds no strong reference. `upgrade() -> Option<Arc<T>>` returns `Some` only if strong count is still > 0.
- Typical use:
  - Observers (event emitters hold `Weak<Listener>` to avoid keeping listeners alive).
  - Parent-child hierarchies (child holds `Weak<Parent>`).
  - Caches (hold `Weak<Resource>` and re-upgrade on access; evict when none).

### Q. `Arc<Mutex<T>>` vs `Mutex<Arc<T>>`

- `Arc<Mutex<T>>`: canonical shared-mutable pattern. Multiple owners, coordinated mutation.
- `Mutex<Arc<T>>`: exclusive ownership of the Arc pointer (rare). Useful when you want to atomically swap the inner Arc, but `ArcSwap` is better.

### R. Send Escape Hatch: `thread::scope`

- Use `thread::scope` when you want to borrow local data across threads without `Arc`. Makes `'static` not required, saves allocations.
- Combine with channels using `&Channel<T>` references (see §5.4) for zero-alloc pipelines.

### S. Correctly Naming Orderings for Readers `[09-performance]`

Canonical intent-expressing phrasing:
- "publish via Release store": producer side.
- "observe via Acquire load": consumer side.
- "sequence with SeqCst total order": global consistency required.
- "counter via Relaxed": independent state.

### T. Common Constants & Limits

- `AtomicUsize::new(0)` — `const fn` suitable for statics.
- `Once`, `OnceLock`, `LazyLock` — all have `const fn new` where applicable.
- Type layouts: `Mutex<T>` is `size_of::<T>() + WORD_SIZE`; `Arc<T>` is pointer-sized (with heap allocation of `ArcData<T>`).

### U. Atomic Volatile and FFI Memory `[08-unsafe-and-ffi]`

- MMIO / device registers — use `read_volatile`/`write_volatile` via pointer, not atomics.
- Signal handlers — use `compiler_fence` + atomics. Do not call anything that takes a lock.
- `unsafe extern "C"` callbacks from C to Rust — ensure the C side shares the memory model (typically POSIX `pthread` primitives align with Rust's `SeqCst` semantics for cross-FFI signaling).

### V. Ownership of OS Primitives `[08-unsafe-and-ffi]`

- OS mutexes (`pthread_mutex_t`, `CRITICAL_SECTION`) cannot move after initialization. Hence `std::sync::Mutex` used to require pinning; since Rust 1.62 the futex-based impl removed that constraint on Linux.
- If wrapping raw OS primitives, mark structs as `!Unpin` or store behind `Box` to prevent moving.

### W. Atomic Choice Decision Tree `[09-performance]`

1. Is the access single-threaded? → use non-atomic + `Cell`/`RefCell`.
2. Multiple threads, simple counter with no cross-memory invariants? → `Atomic*` with `Relaxed`.
3. Publishing data through a flag or pointer? → `Release` (store/RMW) + `Acquire` (load/RMW).
4. Taking over a slot, possibly updating? → `compare_exchange_weak(...)` in a loop with `AcqRel` on success, `Acquire` on failure.
5. Requires global total order (e.g. Dekker, observers)? → `SeqCst` and prove necessity.
6. Cross-cache-line contention? → pad to cache line, consider sharding.
7. Cannot avoid blocking semantics? → `Mutex`/`RwLock`/`Condvar`/`Semaphore` — stop reaching for atomics.

### X. Mixing Orderings — the "Strongest Wins" Gotcha

- A chain of atomic ops on the same variable: Release stores can be observed by Acquire loads regardless of intervening Relaxed ops (release sequence rule for RMWs only). Non-RMW stores break the chain — later Acquire loads won't see earlier Release.
- Mentally: think of Release/Acquire as creating edges; Relaxed RMWs forward the edge; Relaxed stores do not.

### Y. Weak-Ordering Gotchas in Example Code `[05-anti-patterns]`

- Double-Check Locking (DCL) in Rust:
  - Correct: `if p.load(Acquire).is_null() { /* take lock, compute, Release-store */ }`.
  - Incorrect: `Relaxed` on both loads/stores → the constructor's writes may not be visible.
- Singleton init via spin: same rule — Acquire after CAS in read path, Release when initializing.

### Z. Diagnosing Concurrency Bugs `[05-anti-patterns]`

- Symptom: occasional UB / invalid reads / segfaults → suspect data race or ordering bug.
- Tool pass: `cargo miri test`, `cargo +nightly test -Z sanitizer=thread`, `loom` for lock-free structures.
- Code-review pass: mark every `unsafe impl Send/Sync` line; for each atomic op, write the Ordering intent in the commit message.

---

## Appendix: std Atomic API Reference Summary

Methods available on every `Atomic*` type:

| Method | Signature | Orderings |
|--------|-----------|-----------|
| `new` | `const fn new(val: T) -> Self` | n/a |
| `load` | `fn load(&self, ord: Ordering) -> T` | Relaxed/Acquire/SeqCst |
| `store` | `fn store(&self, val: T, ord: Ordering)` | Relaxed/Release/SeqCst |
| `swap` | `fn swap(&self, val: T, ord: Ordering) -> T` | any |
| `compare_exchange` | `fn compare_exchange(&self, cur: T, new: T, s: Ordering, f: Ordering) -> Result<T,T>` | see rules |
| `compare_exchange_weak` | same signature, may spuriously fail | see rules |
| `fetch_add/sub` | `fn fetch_add(&self, val: T, ord: Ordering) -> T` | any |
| `fetch_and/or/xor` | bitwise | any |
| `fetch_max/min` | numeric | any |
| `fetch_update` | closure-based; retries until success | any |
| `get_mut` | `fn get_mut(&mut self) -> &mut T` | (exclusive) |
| `into_inner` | `fn into_inner(self) -> T` | (consumes) |

Only on `AtomicPtr<T>`: `fetch_ptr_add`, `fetch_ptr_sub`, `fetch_byte_add`, `fetch_byte_sub`, `fetch_or`, `fetch_and`, `fetch_xor` (stable 1.86+ for ptr arith).

---

## Appendix: Complete Mental Model Cheatsheet `[02-language-rules]`

### Rust's synchronization graph rules:

1. Every program execution has a **happens-before** partial order.
2. Edges:
   - program order within a thread
   - thread spawn: `spawn(f)` line hb `f`'s start
   - thread join: `f`'s end hb `join()` return
   - Release-Acquire synchronizes-with: `store(Release)` → `load(Acquire)` that reads from it → edge
   - SeqCst: additional total order among SeqCst ops, consistent with hb.
3. A **data race** = two accesses to the same byte, at least one writing, not ordered by hb, and at least one non-atomic. Data races are UB.
4. Atomic reads without Acquire observe the value but do NOT establish hb.
5. `Release` store + `Acquire` load of same var creates hb only if the load reads the value written by that store (or by a release sequence of RMWs).

### Rust's aliasing and mutability rules:

1. `&T`: shared, read-only (to T as seen through this reference). Multiple allowed per thread (and across threads if `T: Sync`).
2. `&mut T`: exclusive; no other references.
3. Interior mutability (`Cell`, `RefCell`, `Mutex`, atomics) gives mutation through `&T`.
4. `UnsafeCell<T>` is the ONLY way to obtain `&mut T` from `&UnsafeCell<T>` soundly.

---

## Appendix: Glossary

- **Atomic**: operation appears instantaneous to other threads; no torn reads/writes.
- **Memory ordering**: the "ordering" argument on atomic ops; governs cross-thread visibility.
- **Happens-before**: partial order defining which writes are visible where.
- **Synchronizes-with**: specific edge type from Release store to Acquire load.
- **Data race**: unsynchronized concurrent access with at least one write; UB in Rust.
- **False sharing**: two unrelated atomics sharing a cache line, causing contention.
- **True sharing**: multiple threads contending on the same atomic; intrinsic.
- **CAS** (compare-and-swap): `compare_exchange`.
- **LL/SC**: load-linked/store-conditional; hardware primitive on ARM, POWER.
- **Weak CAS**: `compare_exchange_weak`; may spuriously fail; prefer in loops.
- **Futex**: Linux primitive for atomic-check-then-sleep.
- **Spin lock**: mutex that busy-loops; fine for very short critical sections, bad under contention.
- **Poisoning**: std `Mutex`/`RwLock` tracks panics-while-locked; `lock()` returns `Err`.
- **ABA**: pointer value repeats; CAS succeeds on stale pointer.
- **Epoch-based reclamation**: safe memory reclamation for lock-free structures.
- **Cache line**: 64-byte unit of cache coherence.
- **MESI/MOESI**: cache coherence protocols.
- **TSO** (Total Store Order): x86's memory model; stores can be buffered, reads-after-writes reordered.
- **Weak memory model**: ARM, POWER; almost any reordering allowed.

---

## Code Archive — Reusable Snippets

### 1. Cache-padded counter
```rust
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[repr(align(64))]
pub struct ShardedCounter {
    shards: [AtomicU64; 64],
}

impl ShardedCounter {
    pub fn new() -> Self {
        Self { shards: core::array::from_fn(|_| AtomicU64::new(0)) }
    }
    pub fn inc(&self) {
        let t = thread_id_hash() as usize % self.shards.len();
        self.shards[t].fetch_add(1, Relaxed);
    }
    pub fn read(&self) -> u64 {
        self.shards.iter().map(|s| s.load(Relaxed)).sum()
    }
}

fn thread_id_hash() -> u64 {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    let mut h = DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    h.finish()
}
```

### 2. Double-Checked Locking for Lazy Init
```rust
use std::sync::atomic::{AtomicPtr, Ordering::*};
use std::sync::Mutex;
use std::ptr;

pub struct Lazy<T> {
    value: AtomicPtr<T>,
    init_lock: Mutex<()>,
}

impl<T> Lazy<T> {
    pub const fn new() -> Self {
        Self { value: AtomicPtr::new(ptr::null_mut()), init_lock: Mutex::new(()) }
    }

    pub fn get_or_init(&self, init: impl FnOnce() -> T) -> &T {
        let p = self.value.load(Acquire);
        if !p.is_null() {
            return unsafe { &*p };
        }
        let _g = self.init_lock.lock().unwrap();
        let p = self.value.load(Acquire);
        if !p.is_null() {
            return unsafe { &*p };
        }
        let boxed = Box::new(init());
        let raw = Box::into_raw(boxed);
        self.value.store(raw, Release);
        unsafe { &*raw }
    }
}
```
Prefer `std::sync::OnceLock` in new code — this is shown for illustration.

### 3. Simple Event / Latch
```rust
use std::sync::atomic::{AtomicU32, Ordering::*};
use atomic_wait::{wait, wake_all};

pub struct Latch { done: AtomicU32 }

impl Latch {
    pub const fn new() -> Self { Self { done: AtomicU32::new(0) } }
    pub fn set(&self) {
        self.done.store(1, Release);
        wake_all(&self.done);
    }
    pub fn wait(&self) {
        while self.done.load(Acquire) == 0 {
            wait(&self.done, 0);
        }
    }
}
```

### 4. SPSC ring buffer sketch
```rust
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering::*};

pub struct Spsc<T, const N: usize> {
    buf: [UnsafeCell<MaybeUninit<T>>; N],
    head: AtomicUsize, // written by consumer
    tail: AtomicUsize, // written by producer
}
unsafe impl<T: Send, const N: usize> Sync for Spsc<T, N> {}

impl<T, const N: usize> Spsc<T, N> {
    pub fn push(&self, v: T) -> Result<(), T> {
        let tail = self.tail.load(Relaxed);
        let next = (tail + 1) % N;
        if next == self.head.load(Acquire) { return Err(v); } // full
        unsafe { (*self.buf[tail].get()).write(v); }
        self.tail.store(next, Release);
        Ok(())
    }
    pub fn pop(&self) -> Option<T> {
        let head = self.head.load(Relaxed);
        if head == self.tail.load(Acquire) { return None; } // empty
        let v = unsafe { (*self.buf[head].get()).assume_init_read() };
        self.head.store((head + 1) % N, Release);
        Some(v)
    }
}
```

### 5. Stop-flag worker
```rust
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::thread;

pub struct Worker { stop: Arc<AtomicBool>, handle: Option<thread::JoinHandle<()>> }

impl Worker {
    pub fn start(mut task: impl FnMut() + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let handle = thread::spawn(move || {
            while !s.load(Relaxed) { task(); }
        });
        Self { stop, handle: Some(handle) }
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}

impl Drop for Worker { fn drop(&mut self) { self.stop(); } }
```

### 6. Guard pattern with RAII unlock
```rust
pub struct LockGuard<'a, T> {
    lock: &'a SpinLock<T>, // from Ch. 4
}
impl<'a, T> Drop for LockGuard<'a, T> {
    fn drop(&mut self) { self.lock.locked.store(false, Release); }
}
```

### 7. Arc drop with Release+AcquireFence
```rust
impl<T> Drop for MyArc<T> {
    fn drop(&mut self) {
        if self.inner().ref_count.fetch_sub(1, Release) == 1 {
            std::sync::atomic::fence(Acquire);
            unsafe { drop(Box::from_raw(self.ptr.as_ptr())); }
        }
    }
}
```

### 8. compare_exchange_weak retry loop
```rust
fn atomic_max(a: &AtomicU32, v: u32) {
    let mut cur = a.load(Relaxed);
    while cur < v {
        match a.compare_exchange_weak(cur, v, Relaxed, Relaxed) {
            Ok(_) => return,
            Err(observed) => cur = observed,
        }
    }
}
```

### 9. RwLock simple (reader-preferred)
```rust
use std::sync::atomic::{AtomicU32, Ordering::*};
use atomic_wait::{wait, wake_one, wake_all};

// state: 0 = idle; n (1..=u32::MAX-1) = n readers; u32::MAX = writer
pub struct RwLock { state: AtomicU32 }

impl RwLock {
    pub fn read(&self) {
        let mut s = self.state.load(Relaxed);
        loop {
            if s < u32::MAX - 1 {
                match self.state.compare_exchange_weak(s, s + 1, Acquire, Relaxed) {
                    Ok(_) => return,
                    Err(o) => s = o,
                }
            } else {
                wait(&self.state, s);
                s = self.state.load(Relaxed);
            }
        }
    }
    pub fn unread(&self) {
        if self.state.fetch_sub(1, Release) == 1 {
            wake_one(&self.state);
        }
    }
    pub fn write(&self) {
        while let Err(s) = self.state.compare_exchange(0, u32::MAX, Acquire, Relaxed) {
            wait(&self.state, s);
        }
    }
    pub fn unwrite(&self) {
        self.state.store(0, Release);
        wake_all(&self.state);
    }
}
```

### 10. `thread::scope` parallel map
```rust
fn parallel_map<T: Send + Sync, R: Send>(
    items: &[T],
    f: impl Fn(&T) -> R + Send + Sync,
) -> Vec<R> {
    let chunk = (items.len() + num_cpus::get() - 1) / num_cpus::get();
    std::thread::scope(|s| {
        let handles: Vec<_> = items.chunks(chunk).map(|c| {
            s.spawn(|| c.iter().map(&f).collect::<Vec<_>>())
        }).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}
```

---

## Appendix: Signposts for Common Bugs

- **Double decrement**: forgetting to guard drop so two Arc drops try to free the same allocation. Use `NonNull<ArcData<T>>` + atomic CAS on drop.
- **Missing `Acquire` fence on last drop**: last thread reads stale data. Always pair `Release` decrement with `Acquire` fence.
- **Relaxed on pointer publication**: data not visible. Use `Release`/`Acquire`.
- **Spurious CAS failures ignored**: use `_weak` in loops, `_strong` outside.
- **SeqCst over-use**: free perf wins by switching non-global atomics to `Release`/`Acquire`.
- **Forgetting spin_loop**: CPU throttles, fails to yield SMT partner.
- **MutexGuard lifetime**: `let _ = lock()` — dropped immediately. Use `let _g = lock()`.
- **Sharing same address non-atomically and atomically**: UB.
- **`unsafe impl Send`** without docs: invites future regressions.

---

## Appendix: Rule Summary (One-Liners)

- `Relaxed` is for atomicity only; use for independent counters/flags.
- `Release` publishes; `Acquire` observes; use them together.
- `SeqCst` only when total order is genuinely needed.
- `compare_exchange_weak` in loops; `compare_exchange` otherwise.
- Release decrement + Acquire fence on last drop for reference counts.
- Cache-pad hot atomics; shard contended ones.
- Guard lifetimes are hazard points — drop them explicitly.
- `Mutex<T>` doesn't need `T: Sync`; `RwLock<T>` does.
- `Arc<Mutex<T>>` is the canonical shared-mutable pattern.
- `Weak<T>` breaks cycles; upgrade may fail.
- `thread::scope` for borrowing across threads without `Arc`.
- OS futex / WaitOnAddress is the bedrock of real mutexes.
- `parking_lot` = no poisoning, adaptive spin, smaller footprint.
- Prefer `OnceLock`/`LazyLock` over manual DCL.
- Never hold `std::sync::MutexGuard` across `.await`.
- Under FFI, share ordering semantics explicitly (use SeqCst at the boundary unless documented otherwise).
- `unsafe impl Send/Sync` requires proof in comments AND review.
- Test with `loom`, `miri`, ThreadSanitizer.
- Hardware picks can obscure bugs: x86 is strong; ARM is weak. Bugs hidden on x86 surface on ARM.
- Build on std's primitives first; roll your own only when profiling demands it.

---

## Addendum — Tools, Clippy, and review checklist `[05-anti-patterns]` `[07-async-concurrency]`

### Cargo / rustc flags (rough guide)

- **`RUSTFLAGS="-Z sanitizer=thread"`** (nightly): ThreadSanitizer for **data races** in tests — run on CI for concurrency-heavy crates when feasible.
- **`cargo miri test`**: interpreter detects UB including some atomic misuse; use for `unsafe` and lock-free tests (slow; limited weak-memory coverage unless flags enabled).
- **`loom`**: model-check small snippets of concurrent logic under **enumerated interleavings** — indispensable for custom atomics; not a substitute for TSan on full binaries.

### Clippy lints to enable for concurrent Rust

- **`await_holding_lock`**: `std::sync::MutexGuard` (and similar) held across `.await` — high deadlock risk on async runtimes.
- Use **`cargo clippy`** with default groups; manually review **“could this be `Atomic*` instead of `Mutex<u64>`?”** — Clippy has heuristic lints in this family on some versions; always verify correctness beats the suggestion.

### One-page review checklist (before merging concurrency PRs)

1. Every **`unsafe` block** has a `// SAFETY:` comment tying to invariants.
2. Every **`unsafe impl Send/Sync`** names the invariant that replaces compiler proof.
3. **`Ordering` on each atomic op** has a one-line rationale (Relaxed vs Release/Acquire vs SeqCst).
4. **No `compare_exchange` in a busy loop** without `_weak` (except single-shot CAS).
5. **Pointer publication** uses Release/Acquire or AcqRel CAS — not Relaxed.
6. **Guards** not held across I/O, sleep, or `.await` unless async mutex + proven safe.
7. **Tests** or `loom`/`miri`/TSan evidence for non-trivial lock-free code.

### `std::sync::mpsc` vs ecosystem (orientation)

- **`std::sync::mpsc`**: multi-producer, single-consumer; bounded via `sync_channel(n)`.
- For **MPMC** or different fairness/perf, crates like **`crossbeam-channel`**, **`flume`** are common — the book’s hand-rolled channels are for **understanding**, not a production replacement for battle-tested queues.

### When to reach for `std::sync::Barrier` / `std::sync::Once`

- **`Barrier`**: fixed party size; all threads wait until N arrivals — good for phased parallel loops.
- **`Once` / `OnceLock` / `LazyLock`**: one-time init without writing your own CAS loop — **prefer these** over ad-hoc static init unless you need very custom behavior.

---

## End-of-Cluster Notes for Coding LLMs

When generating Rust concurrency code, prefer:

1. Pick the simplest working primitive from the hierarchy: `Cell` < `RefCell` < `Mutex` < RwLock < atomics < OS primitives < custom lock-free.
2. Default to `std::sync::Mutex<T>` with `Arc<T>` sharing unless there's a specific reason (profiling, fairness, async) to choose differently.
3. For shared counters, `AtomicUsize::fetch_add(_, Relaxed)` is almost always correct.
4. Any publication of heap-allocated state through a pointer requires Release/Acquire pairing.
5. Never write `unsafe impl Send/Sync` without a documented rationale.
6. Explicitly `join()` threads — dropped handles silently swallow panics with `thread::spawn`, but `thread::scope` re-raises panics from un-joined handles.
7. Inside `async` contexts, default to `tokio::sync::Mutex`; never hold `std::sync::MutexGuard` across `.await`.
8. Use `thread::scope` for short-lived worker parallelism; `rayon` for data parallelism.
9. Use `OnceLock`/`LazyLock` over hand-rolled double-checked locking.
10. Watch for false sharing in hot counters; pad with `crossbeam_utils::CachePadded` when contention matters.

---

*Source: Mara Bos, "Rust Atomics and Locks" (O'Reilly, 2023), https://marabos.nl/atomics/. These notes synthesize the book's content into reference material for coding-assistant LLMs; verify against the primary source when precise wording matters.*
