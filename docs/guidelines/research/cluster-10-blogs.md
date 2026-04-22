# Cluster 10 — High-Signal Rust Blogs (Expert Mental Models)

Источники: fasterthanli.me (Amos Wenger), baby steps (Niko Matsakis), without.boats (Boats).  
**Зачем этот файл:** не список ссылок, а **сжатое знание** — что именно из этих текстов следует про устройство Rust, async, типы и дизайн языка. Удобно для базы знаний к LLM: смысл и ментальные модели, а не указатели на страницы.

**Purpose (EN):** Same — distill **what the authors argue and why**, not a bibliography.

---

## Суть материала — что в итоге следует из прочитанного

**Boats** объясняет async в Rust как историю компромиссов без сборщика мусора: «зелёные потоки» убрали, потому что дешёвые растущие стеки требуют либо сегментов с непредсказуемой ценой вызова, либо копирования стека с переписыванием указателей — для Rust без GC это нежизнеспособно; остался путь «одна машина состояний на future» и внешний `poll`. Отсюда же — почему ранний CPS-стиль futures ломался на `join` (нужно разделить continuation между детьми → аллокации). `Pin` появился не как эстетика, а как способ выразить «объект больше нельзя сдвигать в памяти», не ломая существующие контракты `&mut` и `mem::swap`: отказ от гипотетического `?Move` как от маркера «не двигать тип» — осознанный выбор совместимости. Боль пользователей с `Pin` он связывает в основном с тем, что это **библиотечный** тип без той же синтаксической поддержки, что у обычных ссылок (reborrow, проекции полей). Отдельно — спор про `AsyncIterator`: один длинный `poll_next` vs `async fn next` как два конкурирующих представления машины состояний; от этого зависят pinning, отмена, аллокации под `dyn` и очереди в рантаймах. Про эффекты: корутины и effect handlers — это про разную «область видимости» передачи управления; Rust сознательно держит `await` и `?` видимыми в тексте. Про процесс: затяжные споры вроде postfix `await` без новых аргументов выжигают сообщество — важно закрывать решения.

**Niko** разбирает, почему `T: Send + Trait` не гарантирует `Send` у future из `async fn` в трейте: future — отдельный оpaque тип, и «не-Send» может спрятаться в теле метода; `tokio::spawn` требует отправляемости **на каждом await** при work-stealing. Отсюда потребность в **notation** вроде RTN — чтобы **потребитель** мог потребовать «этот async-метод возвращает Send-future», не навязывая всем impl одну политику (в отличие от макроса `async-trait`, который всё боксирует). Про GATs и «many modes»: это не абстракция ради абстракции, а реальные паттерны (парсеры, embedded async), которые уже эмулируются на ночнике. Про coherence: сиротское правило защищает предсказуемость, но иногда мешает композиции крейтов — обсуждается ослабление с осторожностью.

**Amos** с педагогической стороны: переход на Rust — это не новый синтаксис, а новые **категории проблем** (владение, доказательства в типах); злость нормальна, поиск в Google часто бесполезен без словаря ошибок компилятора. «Сильная типизация» у него иллюстрируется контрастом: где другие языки молчаливо переполняют целое или уезжают в float, Rust заставляет явно выбрать модель числа. Про Go — не «Go плохой», а «простота» может переносить сложность в семантику ОС и кроссплатформенные полустыки». Про async: размер future — реальный layout (локали живут в состоянии машины); `async-trait` разворачивается в тяжёлые сигнатуры с `Pin<Box<dyn Future…>>`, что хорошо видно и в тексте, и в отладчике — наглядный аргумент, чем нативный `async fn in traits` для статической диспетчеризации лучше, и почему `dyn` всё ещё дорог.

---

## English synthesis (same substance, compact)

Rust’s async stack is **poll + explicit state machines** because **green threads** failed the no-GC, FFI, and predictability constraints; **CPS futures** failed because combinators like `**join`** need shared ownership of continuations → allocations. `**Pin`** is the **compatibility-preserving** way to express “this value’s address is now semantically fixed,” not the first-principles design anyone would invent on a clean slate; its UX pain is largely **missing language sugar** vs `&mut`. `**AsyncIterator`** design is really about **one vs two state machines** per stream iteration, with consequences for **pinning, cancellation, intrusive queues, and `dyn` costs**. **Send on async traits** is subtle: bounds on `Self` don’t automatically constrain **opaque returned futures**; **callee-side RTN**-style solutions exist precisely so downstream can demand `**Send`** without forking crates. **Amos** reframes learning pain and numeric/cross-platform honesty as **early, local** friction vs **late, distributed** bugs; his async writing ties **future size** and **macro-expanded trait objects** to concrete debugging and layout intuition.

---

# Part A — without.boats (Boats)

## [TAG: 07-async-concurrency] Why `Future` is poll-based, not continuation-based (Boats — “Why async Rust?”)

- **Mental model:** A `Future` is a state machine that an external executor drives by calling `poll`. It is the same *architectural move* Rust made for external iterators (`Iterator::next`) versus internal/callback iterators — invert control so the combinator graph compiles to one monolithic object without storing heap continuations everywhere.
- **Key insight:** Early Rust futures experiments used continuation-passing style (`schedule(self, continuation)`). That pattern forces allocation (e.g. `join` needs the continuation shared by two children → reference counting). Poll-based design avoids storing the “what happens next” callback inside every future.
- **Common misunderstanding:** “Async Rust is slow because of polling.” Polling is how you avoid the allocation tax of CPS-style futures in Rust; performance comes from monomorphized state machines, not from the poll API being inherently heavy.
- **Historical context:** Green threads existed pre-1.0; they were removed (RFC 230) because the abstraction was not zero-cost, forced identical APIs where semantics differed, and still allowed FFI/native IO escape hatches that broke the model.

## [TAG: 07-async-concurrency] Why green threads could not be Rust’s answer (Boats — “Why async Rust?”)

- **Mental model:** User-space threads need small, growable stacks. Options: segmented stacks (variable cost per call — bad in hot loops) or stack copying (move stack → update pointers). Go can scan its own stacks for pointers; Rust cannot without a GC, and pointers to stack data may live outside that stack.
- **Key insight:** Once green threads used large fixed stacks like OS threads, they lost the memory advantage — a core motivation for removing them. FFI to C also penalizes green-thread stacks; Rust targets embedded and library embedding where a runtime thread scheduler is unacceptable.
- **Common misunderstanding:** “Rust should have used Go-style goroutines.” That path assumes runtime relocation of stack memory or GC-like pointer tracking — both clash with Rust’s borrow-checked, no-GC contract.

## [TAG: 07-async-concurrency] Pin, the `Move` trait that never was, and backward compatibility (Boats — “Pin”)

- **Mental model:** Self-referential async state needs “pinned typestate” — after a certain point, the storage address must stay stable. Ralf Jung formalized this as a third state beyond owned/shared.
- **Key insight:** A `?Move`-style marker trait fails not only because of ergonomics but because adding a new default trait bound on generics breaks existing associated-type reasoning: code may legally assume `IntoFuture::IntoFuture` is swappable via `mem::swap` today; making it `?Move` is a breaking change across the ecosystem.
- **Why `Pin` won:** Pin wraps pointers and restricts moving through those pointers without breaking APIs that require moving out of `&mut T`. `Unpin` opts out for types where pinning is meaningless.
- **Common misunderstanding:** “Pin is for self-referential structs in user code.” Boats stresses Pin’s primary role is compiler-generated futures and unsafe runtime code — safe user-defined self-referential structs are a different (still open) problem.

## [TAG: 05-anti-patterns] The real reason `Pin` feels awful (Boats — “Pin”)

- **Mental model:** Ordinary references enjoy language sugar: reborrowing, method resolution autoref, field projections. `Pin<&mut T>` is a library wrapper — so users hit “moved value” errors where `&mut T` would reborrow, must call `Pin::as_mut`, and learn `pin-project` macros for field access.
- **Key insight:** The difficulty is less “conditional Unpin” and more missing syntactic parity with normal references — especially reborrowing and projections.
- **Design scar:** `Drop::drop` takes `&mut self`, not pinned self, which interacts badly with pinned fields — pin-project crates work around a stable-order issue.

## [TAG: 12-modern-rust] Pinned places — a path to fix Pin ergonomics (Boats — “Pinned places”)

- **Mental model:** Treat pinning as a property of places (like mutability), not as an awkward pointer wrapper only. Strachey / Rust “place vs value” distinction: mutability applies to places, not “mutable values.”
- **Key insight:** Proposed surface: `let pinned mut stream = ...`, `&pinned mut T` as sugar for pinned references, method resolution inserts pinned refs like it inserts `&mut`, and `pinned` fields on structs enable safe pinned projections with destructor rules (`drop(&pinned mut self)` for types with pinned fields).
- **Common misunderstanding:** “Immovable types (`!Move`) replace Pin.” Boats argues pinning is a place-state; a type-level `Move` trait reintroduces two-type emplacement stories and massive backward incompatibility versus `Pin` + `Unpin`.
- Position vs Yosh’s proposal: Boats prefers integrating pinned places into the language over adopting a new `Move` auto-trait for immovable types; cites backward compatibility and theoretical fit.

## [TAG: 01-meta-principles] Coroutines vs effect handlers — static vs dynamic scope (Boats — “Coroutines and effects”)

- **Mental model:** Coroutines yield to their caller; effect handlers yield to the nearest enclosing handler for that effect. That is the static vs dynamic scoping distinction for control flow.
- **Key insight:** Rust’s `async`/`await` + `Result`/`?` model effects as statically typed and lexically scoped — you see `await` and `?` at call sites. Effect systems (Koka-style) forward effects implicitly through callees; understanding “where IO happens” requires reading every callee signature.
- **Common misunderstanding:** “Rust should use effect polymorphism instead of async.” Different axis: Rust chose explicit effect forwarding at call sites (like explicit `?`) deliberately — local reasoning about errors and IO points.
- **Table mental model:** unchecked exceptions / blocking IO ≈ dynamic+dynamic; checked exceptions / IO effects ≈ static types + dynamic scope; `Result` + async/await ≈ static + lexical.

## [TAG: 07-async-concurrency] `poll_next` vs `async fn next` for `AsyncIterator` (Boats — “poll_next”)

- **Mental model:** `poll_next` — one pinned state machine for the whole async iteration. `async fn next` — two state machines: the iterator + a fresh future per item.
- **Key insight:** Second design introduces indirection and per-iteration allocation risk (especially under `dyn` where inlining dies). Not zero-cost in the sense Rust promises for futures.
- **Pinning:** Long-lived intrusive structures (tokio/smol async primitives) need the long-lived state machine pinned in intrusive lists; if only the short-lived `next` future is pinned, dropping/cancelling it loses queue position — starvation and ordering bugs.
- **Cancellation:** Dropping a `next` future cancels that iteration’s state; `poll_next` cancellation semantics differ — Boats ties this to broader “cancellation safety” debates (not always a bug to cancel “unsafe” futures — depends on reactor: epoll vs io-uring).

## [TAG: 12-modern-rust] Four-year stagnation after async MVP (Boats — “A four year plan for async Rust”)

- **Mental model:** Async/await shipped as MVP; lack of follow-on language/library features is a controllable driver of async Rust’s bad reputation (alongside essential complexity of concurrency).
- **Key insight:** Near-term priorities: stabilize `AsyncIterator` (with `poll_next`), RTN (return-type notation) for `Send` bounds on opaque async returns, async generators / `for await`, coroutine closures modeled as `Fn -> impl Future` with sugar rather than multiplying `AsyncFn`* trait families.
- **Medium-term:** Object-safe async traits need story for heap-allocated opaque futures — tension between “explicit allocation” culture and ergonomics; `dyn`* not strictly prerequisite if allocation strategies differ by trait object type.
- **Long-term / existential:** Immovable/unforgettable/linear types — Pin is a compatibility hack; `Iterator`/`Drop` predate Pin and block clean generators unless rules change across editions — 2027 edition floated as earliest possible for fundamental movement rules.

## [TAG: 01-meta-principles] Governance and process (Boats — “A four year plan” closing)

- **Mental model:** Await syntax debate burnout: holding out when consensus already exists wastes community energy; not every controversy has a win-win.
- **Key insight:** Criticism of Rust project: indefinite open controversies, inward documentation (Zulip/HackMD), fragmented public narrative — hard for outsiders to see priorities.
- **Takeaway for LLMs:** When discussing RFC status, prefer primary sources (RFC repo, nightly release notes) — blog posts capture intent and tradeoffs at a point in time.

---

# Part B — smallcultfollowing.com/babysteps (Niko Matsakis)

## [TAG: 12-modern-rust] Send bounds on futures from async trait methods (Niko — “Async trait send bounds, part 1”)

- **Mental model:** `trait T { async fn f() }` desugars to returning an opaque `impl Future`. Whether that future is `Send` is not part of the trait unless syntax says so — yet `tokio::spawn` requires a `Send` future.
- **Key insight:** `H: Trait + Send + 'static` does not imply `H::method()`’s future is `Send` — the future may capture non-`Send` state from the method body. The error surfaces at the await of that call.
- **Why not `async-trait` crate’s approach for std: Macro expands to boxed sendable futures — forces heap allocation, hurts `no_std`, and prevents single trait from serving single-threaded, multi-threaded, and embedded executors interchangeably.
- **Desired direction:** Caller-side bounds — the function that needs `Send` futures should say so (Return Type Notation / related syntax), not force every impl to return boxed `Send` futures globally.
- **Appendix mental model:** Work-stealing executors may move the task across threads at any await point — anything held across awaits must be `Send` (not just the value you passed into `spawn`).

## [TAG: 12-modern-rust] GATs — “many modes” pattern (Niko — “Many modes: a GATs pattern” — from index lede)

- **Mental model:** GATs let associated types depend on input lifetimes/modes — enabling patterns like parser combinator libraries switching representation per mode without duplicating the entire API surface.
- **Key insight:** Stabilization debate: GATs add surface complexity; counterpoint — they are already how advanced async traits are emulated (Embassy, `real-async-trait`), and hiding them behind nicer syntax (async fn, RPITIT) still depends on GAT-like semantics underneath.
- **Common misunderstanding:** “We should skip GATs and only ship sugar.” Sugar still needs semantic foundations; rejecting GATs risks delaying expressive traits that async/embedded ecosystems already rely on in nightly macros.

## [TAG: 02-language-rules] Coherence and orphan rule (Niko — “Coherence and crate-level where-clauses” — from index lede)

- **Mental model:** Orphan rule ensures at most one impl applies for a `(Trait, Types...)` query so crates compose — foreign trait + foreign type impls are forbidden.
- **Key insight:** Rule is safe but overly strict — “chilling effect” on crate composition when legitimate impls are disallowed. Direction of travel: explore weakening orphan restrictions via carefully controlled mechanisms (crate-level where-clauses / proofs of non-overlap), not abandoning coherence globally.
- **Common misunderstanding:** “Just allow impls anywhere.” Without coherence, Rust needs either disambiguation at use sites or global uniqueness breaks — orphan rule exists because of ecosystem predictability, not spite.

## [TAG: 12-modern-rust] Dyn async traits, “soul of Rust”, transparency (Niko — series titles on index)

- **Mental model:** Async fn in traits forces allocation questions for `dyn` — tension between high-level ergonomics and low-level visibility of costs (the “soul” essays).
- **Key insight:** Rust’s identity is not “never allocate” — it is transparent about costs when they exist. Hard calls happen when productivity vs transparency conflicts; “soul” language encodes values for tradeoff decisions, not a single rule.

## [TAG: 07-async-concurrency] Async cancellation case study (Niko — “Async cancellation: mini-redis” — index)

- **Mental model:** Real async code can violate expectations without compile errors — subtle timing / cancellation / subscription semantics.
- **Key insight:** Mini-redis exemplifies good async style while relying on conventions that are easy to get almost right — failures appear under race conditions, not in unit tests.

## [TAG: 12-modern-rust] Async vision doc and “living UX document” (Niko — index posts on Async Vision / project goals)

- **Mental model:** Status quo stories (narrative pain) before shiny future — shared vocabulary for why async work matters.
- **Key insight:** Project goals as incremental roadmap with owners — response to roadmap drift and morale issues in a maturing Rust.

---

# Part C — fasterthanli.me (Amos Wenger)

## [TAG: 01-meta-principles] Learning Rust is learning new *topics*, not new syntax (Amos — “Frustrated? It’s not you, it’s Rust”)

- **Mental model:** Switching from Java/Python/JS to Rust is not like French→Spanish; you lack vocabulary for problems Rust makes explicit (aliasing, lifetimes, provenance). Prior expertise can hurt — habits encode assumptions Rust does not guarantee.
- **Key insight:** The compiler works from code, not intent — frustration spikes when mental model omits details Rust encodes in types. Search engines are weak for this; compiler errors + targeted reading beat random SO.
- **Common misunderstanding:** “I’m experienced; Rust shouldn’t take long.” Experience in GC languages doesn’t transfer as fluency — only discipline and debugging skill transfer.

## [TAG: 01-meta-principles] “You’re smarter than Rust” — explicit types as contracts (Amos — “Frustrated?”)

- **Mental model:** Dynamic languages let you keep implicit invariants; Rust forces machine-checkable contracts. You carry more proofs; the upside is library users cannot accidentally violate those contracts across crate boundaries.
- **Key insight:** Generics require trait bounds — `T: Add` isn’t bureaucracy; it is proof `+` is legal. The compiler refuses to guess.

## [TAG: 02-language-rules] “A half-hour to learn Rust” as a symbol lookup table (Amos)

- **Mental model:** The post is a dense tour of surface syntax: `let`, shadowing, tuples, pattern matching, `struct`, `enum`, `match`, `fn`, closures, `impl`, traits, generics, `where`, iterators — intended as read fluency, not mastery.
- **Key insight:** Rust readability scales with recognizing keywords/operators quickly; mastery still requires ownership/borrowing chapters not reducible to syntax glossaries.

## [TAG: 01-meta-principles] Correctness is economic, not binary (Amos — “Aiming for correctness with types”)

- **Mental model:** “Correct enough” depends on product domain; incorrectness has cost (credits, support, churn) even when “ship fast” culture dismisses it.
- **Key insight:** Rust advocacy misfires when it sounds moralizing; three truths coexist — Rust does require different thinking, it is harder to get *first draft* code than in Go/JS, and it can reduce certain bug classes if you invest in modeling.
- Implicit contracts: Protocols (HTTP, SSH) rely on assumptions not fully specified; attackers and reality violate “implicit contracts.” Types are explicit contracts — analogous to pinning down what you assumed.

## [TAG: 05-anti-patterns] Strong typing vs “works on my machine” numerics (Amos — “The curse of strong typing”)

- **Mental model:** Integer vs float is not pedantry — it mirrors distinct machine representations and error modes (`{integer} * {float}` is a real ambiguity).
- **Key insight:** Go’s `int` loop multiplying silently wraps at overflow; JS promotes to IEEE double — both “work” until they don’t. Rust refuses implicit mixing; pain is early not late.
- **Common misunderstanding:** “Strong typing is just annoying.” The post frames annoyance as paying inconsistency costs upfront instead of debugging financial/traffic incidents later.

## [TAG: 01-meta-principles] “Simple is a lie” — complexity has to live somewhere (Amos — “I want off Mr. Golang’s wild ride”)

- **Mental model:** Go markets simple syntax; platform reality (Windows vs Unix file metadata, networking edge cases) doesn’t disappear — it leaks as fabricated behavior (`FileMode` on Windows), surprising semantics, or hidden runtime costs.
- **Key insight:** Porting Unix-shaped APIs to Windows without rich types produces made-up modes/permissions — “simple” API moved complexity to correctness and cross-platform reasoning instead of removing it.
- **Common misunderstanding:** “Rust is more complex than Go.” Often Rust is more explicit about the same OS mess; Go hid it until it bit you.

## [TAG: 07-async-concurrency] Async trait objects and future sizing (Amos — “Catching up with async Rust”)

- **Mental model:** Each `async fn` has a distinct anonymous future type whose size depends on live stack locals across await points — bigger bodies → bigger futures.
- **Key insight:** `dyn` methods returning unsized futures require erasure — boxing (`Pin<Box<dyn Future + Send>>`) is how `async-trait` macro made `dyn` work; native `async fn in traits` eventually avoids always boxing for static dispatch but `dyn` still implies vtable + storage questions.
- Practical picture: `Pin<Box<dyn Future<...>>>` is fat pointer + vtable; Amos walks through LLDB showing `pointer` + `vtable` fields — dynamic dispatch is not invisible to data layout.
- **Common misunderstanding:** “Async is zero-cost everywhere.” Zero-cost static dispatch monomorphization ≠ zero-cost `dyn` async calls.

## [TAG: 12-modern-rust] 2024–2025 ecosystem notes (Amos — home feed topics)

- **Mental model:** Amos’s recent work ties async to real systems (sans-io, spectrogram/audio tooling, HTTP/io_uring, supply-chain / crates.io phishing awareness).
- **Key insight:** “Catching up with async Rust” positions native `async fn in traits` as closing a major gap versus macros; emphasizes Sized futures, boxing for `dyn`, and stdlib trajectory.

---

# Part D — Cross-author themes (for LLM routing)

## [TAG: 01-meta-principles] Rust’s “novelty is the compromise”

- Boats: systems constraints (no GC, FFI) + async state machines + Pin.  
- Niko: productivity vs transparency in async trait design.  
- Amos: honesty about learning curve vs implicit failure modes in “easy” languages.

## [TAG: 07-async-concurrency] The async trinity in expert discourse

- Execution: `poll` + executor + waker.  
- Safety: `Pin` + `Unpin` + pinned projections (language integration still evolving).  
- Concurrency: `Send`/`Sync` on generated state machine fields — bounds appear where tasks move across threads, not only where values are constructed.

## [TAG: 05-anti-patterns] Recurring expert warnings

- Treating `.clone()` as the first fix instead of ownership restructuring.  
- `std::sync::Mutex` across async await points (can block executor) — ecosystem prefers async mutexes when holding across await.  
- Assuming cancellation is harmless — Boats/Niko both complicate this with io-uring vs epoll and “cancellation safety” nuance.  
- `async` for “speed” without concurrency — async doesn’t make CPU work faster; it helps overlap IO and structure state machines.

## [TAG: 12-modern-rust] Feature coupling map (as of blog-era discussions)

- `async fn in traits` ↔ RPITIT ↔ GAT-like semantics ↔ Send bounds (RTN) ↔ `dyn` object safety (allocation, vtables).  
- Generators ↔ self-referential structs ↔ `Pin` / possible `Move` / pinned places proposals.  
- Async drop / scoped tasks ↔ linear/unforgettable types ↔ tension with drop as synchronous today.

---

# Appendix — Tag index (quick scan)

- 01-meta-principles: Learning curve honesty; complexity relocation; correctness economics; Rust as compromise; process/governance (Boats closing).
- 02-language-rules: Places vs values; orphan rule / coherence direction; GAT “modes” pattern; numeric types as machine truth.
- 05-anti-patterns: Silent overflow in other languages; implicit cross-platform lies; mutex choice across await; cancellation assumptions.
- 07-async-concurrency: Poll vs CPS; green thread removal; Pin rationale; pinned places proposal; `poll_next` vs `async next`; Send futures and work-stealing.
- 12-modern-rust: RTN / send bounds; async trait object costs; AsyncIterator stabilization; four-year async roadmap; dyn async soul-of-Rust framing.

---

# Part E — Deep dives (additional fetched material)

## [TAG: 07-async-concurrency] `select!` in a loop hides cancellation (Boats — “poll_next” continuation)

- **Mental model:** Each `select!` iteration drops futures that did not win the race. That is cancellation. Branches are not “polled in parallel forever” — losers are torn down and recreated next iteration unless you hoist and fuse a long-lived future and poll `&mut future` instead.
- **Key insight:** Users often mis-model `select!` as repeated polling of stable operations; in reality they may construct and cancel every iteration — if cancellation is meaningful (mutex wait queues, io-uring reads), behavior differs from intuition.
- **Mitigation pattern:** `let mut fut = pin!(async_function().fuse());` then `select! { _ = &mut fut => ... }` keeps one logical async operation alive across iterations.
- **Common misunderstanding:** “Cancellation safety is always about bad code.” Sometimes cancellation is desired; the bug is not knowing cancellation happened (especially with reactor-specific semantics).

## [TAG: 07-async-concurrency] Merge vs select — stream-shaped concurrency (Boats — “poll_next”)

- **Mental model:** For repeated heterogeneous events, merge-shaped APIs (over `AsyncIterator`) match “long-lived multiplex” better than select over one-shot futures.
- **Key insight:** Ecosystem lacks polished `merge!`-style macros partly because `AsyncIterator` / Stream stabilization lagged — core language delay blocked higher-level ergonomic patterns that depend on stable async iteration.
- Table: `await` / `for await` / `select!` / `join!` / `merge!` / `zip!` align with single-item vs sum (one ready) vs product (all ready) — useful taxonomy for choosing primitives.

## [TAG: 05-anti-patterns] Hand-written state machines vs compiler coroutines (Boats — “poll_next”)

- **Mental model:** `async fn` and (future) `async gen` store all cross-await state in one generated struct — you cannot accidentally leave a variable on the C stack across a poll point. Hand-written `poll` can mis-place state (see curl CVE story — value reset every poll).
- **Key insight:** `async fn next` for `AsyncIterator` splits responsibility: compiler state machine for one iteration, hand-written iterator for cross-item state — easier to mis-place persistent state between iterations (“mixed register” API).
- **Pragmatic stance:** High-level users should prefer async generators or combinators; `poll_next` remains for low-level control of layout/behavior — and `poll_next` + generators still allows wrapping `async fn next`-style objects into `AsyncIterator` via a small `async gen` adapter loop.

## [TAG: 02-language-rules] `AsyncIterator` is both “async Iterator” and “iterative Future” (Boats — “poll_next”)

- **Mental model:** The trait sits at the product of iterator-ish and future-ish coroutine patterns — not a mere duplicate of `Iterator` with async seasoning.
- **Key insight:** Framing debates (“splitting Iterator in two”) miss that Rust already chose separate traits for async steps (`Future`) and sync steps (`Iterator`); combining yields requires a third pattern, not a trivial alias.
- **Pedagogical takeaway:** Teach `AsyncIterator` as its own coroutine species (`Poll<Item>` yields, `Context` resumes).

## [TAG: 07-async-concurrency] Tasks vs futures — vocabulary (Boats — “Let futures be futures”)

- **Mental model:** All tasks are futures; not all futures are tasks. A future becomes a task when scheduled on an executor (`spawn`); most futures inside an async block are composed state, not separate tasks.
- **Key insight:** The task’s state machine is the “perfectly sized stack” for that workload — nested `await` merges child future state into parent state without per-await heap tasks by default.
- **Consequence:** Recursive `async fn` needs explicit boxing of the recursive call — analogous to recursive structs.

## [TAG: 07-async-concurrency] Multi-task vs intra-task concurrency (Boats — “Let futures be futures”)

- **Mental model:** Multi-task: `spawn` + async channels/locks — resembles threads. Intra-task: `select!`/`join!` embed concurrent futures inside one state machine — unique to async futures model.
- **Key insight:** Readiness-based futures + monomorphization made join/select practical without CPS allocation tax — this was the engineering point of moving away from continuation-stored callbacks (`join` was Turon’s motivating painful case).
- **Limitation:** Intra-task concurrency is fixed arity at compile time — dynamic fan-out needs heap collections (`FuturesUnordered`, etc.), mirroring “no DST stack arrays”.

## [TAG: 01-meta-principles] “Function coloring” in static languages (Boats — “Let futures be futures”)

- **Mental model:** Nystrom’s “red/blue functions” critique targets JS-era pain (wrong color → absurd runtime). Rust encodes color in types → compile error, not vitreous-humor-stealing clowns.
- **Key insight:** Distinct async vs sync function types match Eriksen-style benefits: you see IO vs pure work in signatures, compose concurrent structured graphs, lift business logic from scheduling boilerplate.
- **Common misunderstanding:** “Async Rust’s coloring is uniquely bad.” Experts often argue the unique win is intra-task structured concurrency + zero-allocation fused state machines — once ecosystem gaps (AsyncIterator, RTN) close.

## [TAG: 12-modern-rust] 2018 async/await RFC FAQs — design fossils (Boats — “Async & Await in Rust: a full proposal”)

- **Mental model:** `await!()` macro-first syntax was Stroustrup’s rule — ship noisy syntax first, layer polish later (precedence with `?` was genuinely tricky).
- **Key insight:** Async fns return immediately with an unevaluated future; eager-to-first-await was rejected to keep polling starts everything mental model simple.
- **Std surface:** Only minimal `Future` + task/executor support was slated for std — not full `futures` crate; explains historical ecosystem vs std split (`Stream` lived in crates for years).
- Pin in early RFC text: Shows `Future::poll(self: Pin<...>)` even when surrounding naming (`Async` vs `Poll`) evolved — Pin was always part of the self-referential story.
- **Error channel removal:** Early `Future::Error` idea dropped — async fns shouldn’t be forced to `Result` outputs; errors become `Output = Result<...>` when needed (keeps async useful for non-IO concurrency).

## [TAG: 02-language-rules] Inner vs outer return type of `async fn` (Boats — “Async & Await in Rust: a full proposal”)

- **Mental model:** Async functions have inner return (what you `return`/`?`) and outer anonymous future type. Early RFC chose inner notation for lifetime elision ergonomics vs `-> impl Future` outer style.
- **Key insight:** This choice cascades into how RPITIT and async-in-traits later had to reason about opaque associated return types — the language committed to surface syntax that hides the future type while borrowing rules still apply.

---

# Part F — Granular Amos takeaways (same articles, finer slices)

## [TAG: 01-meta-principles] Compiler errors vs search engines (Amos — “Frustrated?”)

- **Mental model:** Rust problems are often ungoogleable because the issue is type-system vocabulary, not a missing import. Compiler messages + The Book beat random SEO.
- **Key insight:** Frustration peaks when you cannot name what you want — invest in learning ownership vocabulary before large refactors.

## [TAG: 05-anti-patterns] Expertise can slow Rust learning (Amos — “Frustrated?”)

- **Mental model:** Senior engineers expect fast feedback; Rust front-loads slow compiles + strict proofs. Emotional mismatch ≠ intellectual inability.
- **Key insight:** “Feeling dumb” is a known stage — communities should normalize it without hand-waving real ergonomics issues.

## [TAG: 02-language-rules] Fencepost errors meet futures layout (Amos — “Catching up with async Rust”)

- **Mental model:** Measuring stack distances between locals demonstrates future sizes include neighbor locals — intuitive pictures beat abstract “state machine” talk for learners.
- **Key insight:** `foo` vs `bar` async fns differ in sizeof future — not all async calls are equally “light.”

## [TAG: 07-async-concurrency] Why `async-trait` boxed (Amos — “Catching up with async Rust”)

- **Mental model:** Trait objects need known vtable entry sizes; different async fns → different future sizes → erase via `Pin<Box<dyn Future + Send + '_>>` in macro expansion.
- **Key insight:** Reading macro-expanded trait shows exact bound soup — educational for understanding what native async-in-traits avoids for static dispatch.

## [TAG: 01-meta-principles] Cross-platform API simplicity debt (Amos — “Golang wild ride” intro)

- **Mental model:** Portable API design on non-Unix systems often simulates Unix semantics — “simple” `FileMode` masks OS-specific truth.
- **Key insight:** Rust’s explicit split types (paths, OS strings) look noisy but surface cross-platform lies early.

## [TAG: 05-anti-patterns] Overflow narratives (Amos — “Curse of strong typing” opening)

- **Mental model:** Show Go overflow and JS float growth side-by-side to argue “ergonomic” languages defer numeric truth until production.
- **Key insight:** Rust’s refusal to multiply `{integer}` by `{float}` without explicit conversion is consistent with that thesis — pain is local and explainable.

---

# Part G — Granular Niko takeaways (series context from index + part 1 fetch)

## [TAG: 12-modern-rust] RTN as “can you fix downstream?” (Niko — send bounds series + Boats alignment)

- **Mental model:** Without RTN-like syntax, if a crate author didn’t add `Send` to an async trait method’s future, you may be unable to bound it at the call site — forking becomes the “fix.”
- **Key insight:** This is library-ergonomics and composability — not micro-syntax bikeshedding.

## [TAG: 07-async-concurrency] `Send` vs thread-affinity (Niko — send bounds appendix)

- **Mental model:** Some types are thread-affine (TLS assumptions) — not just “`Rc` is weird.” Future: maybe distinguish `Rc`-like vs true thread-local non-Send.
- **Key insight:** Work-stealing requires `Send` at await, not only at spawn entry — non-Send can lurk inside opaque futures.

## [TAG: 12-modern-rust] Preview crates & “plumbing vs porcelain” (Niko — index post)

- **Mental model:** Ship compiler plumbing to stable preview crates while porcelain syntax iterates — reduces deadlock between “need the capability” and “don’t know final syntax.”
- **Key insight:** Governance pattern for LLMs to track: feature may exist twice — nightly/internal vs stable user-facing.

## [TAG: 01-meta-principles] Rust design axioms / Rustacean principles (Niko — index titles)

- **Mental model:** Explicit axioms help resolve soul of Rust disagreements — not every decision has a third way immediately.
- **Key insight:** Useful when LLMs answer “what would Rust likely do” — anchor on transparency, versatility, reliability tensions described in dyn-async series.

---

# Part H — Study prompts (for LLM fine-tuning / retrieval drills)

1. Explain why CPS futures forced `join` to allocate but poll futures avoid it — cite shared continuation ownership argument.
2. Contrast effect handlers vs stackless coroutines using lexical vs dynamic effect scope.
3. Why did green threads fail in pre-1.0 Rust — list segmented stacks, stack copying, FFI, two-worlds abstraction leak.
4. Describe pinned typestate without using the word “self-referential” first — then connect to async lowering.
5. Why is `select!` loop dangerous for io-uring-style completion APIs?
6. What does Boats mean by intra-task concurrency and why is `FuturesUnordered` a different tool?
7. Summarize `async-trait` macro tradeoff vs native async fn in traits for `no_std` and single-threaded executors.
8. How does Niko decompose the `Send` future problem for `tokio::spawn` — what’s not captured by `T: Send`?
9. What is the `many modes` GAT pattern defending against — “GATs too complex vs sugar-only” debate?
10. Explain orphan rule chilling effect in ecosystem composition terms.

---

# Part J — Supplementary tagged entries (same authors; alternative phrasing)

The following entries restate the same themes in different words — useful if an LLM or human queries the same idea with different vocabulary.

## [TAG: 01-meta-principles] Stroustrup’s rule and `await!()` noise (Boats — 2018 async proposal)

- **Mental model:** Ship a clearly temporary syntax when precedence/design is unresolved; iterate toward final `await` spelling once experience lands.
- **Key insight:** `await` vs `?` interaction is a real parsing/precedence problem — not bikeshedding theater; early macro form documents the uncertainty honestly.
- **Common misunderstanding:** “Rust async syntax was rushed.” The rush was feature delivery; syntax intentionally stayed noisily replaceable.

## [TAG: 07-async-concurrency] “Async tasks aren’t ersatz threads” (Boats — “Let futures be futures”)

- **Mental model:** Threads compose with blocking calls; futures compose with pending polls — different algebra of control flow even when both model concurrency.
- **Key insight:** Performance story is secondary to composition story — futures as values you can pass, store, cancel, and combine.
- **Common misunderstanding:** “Futures are just green threads done badly.” Experts in this lineage reject that — task is not OS thread analog in cost model.

## [TAG: 07-async-concurrency] Recursive async functions box for the same reason recursive structs box (Boats)

- **Mental model:** Infinite expansion of monomorphized state size is forbidden — recursion needs indirection (`Box`) to keep the generated future finite-sized.
- **Key insight:** This is a fundamental interaction between stackless codegen and Rust sized types, not a tokio quirk.
- **Common misunderstanding:** “I'll just recurse async like normal functions.” Only if you box the recursive async call path.

## [TAG: 05-anti-patterns] Dynamic arity concurrency needs heap structures (Boats — intra-task limits)

- **Mental model:** `join!(a,b)` can embed statically; joining N unknown at compile time requires `Vec` of boxed futures or `FuturesUnordered` — different Big-O of bookkeeping.
- **Key insight:** Confusing intra-task `select!` with dynamic fan-out leads to wrong data structure choice.
- **Common misunderstanding:** “`FuturesUnordered` is always better.” It solves dynamic sets; static `join!` remains tighter when arity is fixed.

## [TAG: 07-async-concurrency] Async mutex vs blocking mutex — not “async worship” (Boats — async-std analogy)

- **Mental model:** Async locks park tasks in a queue compatible with the executor; blocking locks park OS threads — mixing them starves the executor if held across await incorrectly.
- **Key insight:** Similar surface API, different runtime contract — expert docs emphasize which side of `await` the lock guard crosses.
- **Common misunderstanding:** “The book says Mutex is fine.” `std::sync::Mutex` + `await` while holding can block other tasks on the same thread — pattern-level bug.

## [TAG: 12-modern-rust] Effects / keyword generics as dependency of “async Iterator” framing (Boats — poll_next conclusion)

- **Mental model:** Some contributors want one Iterator trait with maybe-async methods — that implies effect polymorphism / keyword generics — a large language feature cluster.
- **Key insight:** Boats argues poll_next + async gen reaches ergonomics without waiting for that cluster — schedule risk analysis is part of engineering literacy here.
- **Common misunderstanding:** “Lang team is lazy for not unifying sync/async traits.” Unification has massive semantic and object-safety costs.

## [TAG: 02-language-rules] `IteratorFuture` reducio — reductio ad absurdum for design arguments (Boats)

- **Mental model:** You can imagine `poll` yielding an iterator of `Poll<T>` — symmetric absurdity to `async fn next` — shows many dual formulations; choose based on Rust history, not purity.
- **Key insight:** Teaches why bikeshedding without representation model (one vs two state machines) is empty.
- **Common misunderstanding:** “Only async-next feels symmetric.” Symmetry exists in multiple directions; cost model breaks ties.

## [TAG: 05-anti-patterns] `merge!` missing ecosystem — language/stdlib as bottleneck (Boats)

- **Mental model:** Macro ergonomics depend on stable async iteration trait — stabilization delays cascade into every higher-level pattern built atop.
- **Key insight:** Users blame tokio when the root is core trait uncertainty — distinguish library quality from foundational spec gaps.

## [TAG: 07-async-concurrency] Canceled `next` future vs canceled `poll_next` call (Boats)

- **Mental model:** Dropping the per-item future in `async next` aborts that item’s partial state; dropping between `poll_next` calls doesn’t split the same state machine the same way.
- **Key insight:** Cancellation semantics differ — experts tune APIs (broadcast vs smol anycast) around whether queue position must survive partial waits.

## [TAG: 01-meta-principles] Await postfix burnout and governance (Boats — four year plan closing)

- **Mental model:** Holding lone veto on a syntax decision with no new arguments wastes community attention — process failure mode distinct from technical wrong/right.
- **Key insight:** Important for LLMs advising OSS: close decisions when input is duplicate; keep rationale indexed.

## [TAG: 12-modern-rust] Network services domain working group origin story (Boats — “Why async Rust?”)

- **Mental model:** 2018 edition marketing + domain working groups channeled energy into network services async — shaped tokio-first ecosystem gravity.
- **Key insight:** Historical funding/survival pressure explains ecosystem skew — not proof other domains are “less Rusty.”
- **Common misunderstanding:** “Async exists for web devs.” Primary stated driver was C/C++ network service displacement + Mozilla narrative needs — JS similarity is not the design center.

## [TAG: 01-meta-principles] Boats clarifies misread of “Rust survival depended on async” (Boats — “Why async Rust?”)

- **Mental model:** Economic/adoption arguments are context for priority, not a replacement for technical green-thread/stackless reasoning.
- **Key insight:** LLMs should not collapse the essay into “marketing picked async” — the article’s core remains memory model + executor constraints.
- **Common misunderstanding:** Quoting one sentence out of economic paragraph to dismiss technical sections — bad faith reading explicitly called out by author.

## [TAG: 07-async-concurrency] Blocking IO / thread pools as moral alternative (Boats — “Why async Rust?” closing)

- **Mental model:** Threads remain fine for many programs; async ecosystem dominance is social+cargo graph concentration, not compiler mandate.
- **Key insight:** “pollster in std” style bridges could reduce forced runtime adoption for small tools — still an ecosystem/politics issue.
- **Common misunderstanding:** “Rust forces async.” Std library is largely sync; crates.io network stack concentration is the pressure point.

## [TAG: 02-language-rules] Monads vs coroutines ordering (Boats — coroutines/effects)

- **Mental model:** Monads layer effects (`IO (Result T)` ordering matters); coroutines can yield either pending or error in one composable stack — unlayered model.
- **Key insight:** Rust still orders errors via `Result` separately from async — deliberate hybrid; theoretical purity sacrificed for ergonomics.

## [TAG: 12-modern-rust] Koka/Effekt citation purpose (Boats — coroutines/effects)

- **Mental model:** Academic effect systems typed effects with handler scoping — contrasts with Rust’s lexical `await`/`?` — useful for PL readers, not “Rust should become Koka.”
- **Key insight:** Clarifies vocabulary when users call everything “effects.”

## [TAG: 05-anti-patterns] Python/JS async as fourth quadrant (Boats — effects table)

- **Mental model:** Dynamically typed + lexically scoped async (`await` required but not typechecked) — distinct failure mode from Rust’s quadrant.
- **Key insight:** Helps explain why “forget await” is soundness issue in some languages but type error in Rust (in `async` contexts).

## [TAG: 02-language-rules] Wikipedia coroutine definition drift (Boats)

- **Mental model:** Older literature “coroutine” includes generators with multiple yield targets; Rust uses narrower stackless generator model.
- **Key insight:** Translation guide for readers of 1970s papers vs Rust docs reduces talking past each other in RFC threads.

## [TAG: 07-async-concurrency] Intrusive lists need pinned long-lived nodes (Boats — poll_next)

- **Mental model:** Wait queues store pointers into futures; if the container future moves, pointers dangle — pin is not abstract pedantry for executors.
- **Key insight:** Connects Pin to real DS in runtimes — not only academic self-reference examples.

## [TAG: 12-modern-rust] `UnpinCell` forward reference (Boats — pinned places)

- **Mental model:** Later proposal (`UnpinCell`) tries to narrow safe projection surface area — read Boats sequence in order: `Pin` → `pinned places` → `UnpinCell`.
- **Key insight:** LLMs answering projection questions should cite current year crate ecosystem vs proposed syntax — label status.

## [TAG: 02-language-rules] Drop signature evolution thought experiment (Boats — pinned places)

- **Mental model:** For types with `pinned` fields, destructor could take `&pinned mut self` — interacts with dropck and manual drop glue — compatibility story is subtle.
- **Key insight:** Explains why `pin-project` persists — language didn’t retrofit Drop when Pin landed.

## [TAG: 01-meta-principles] Immovable types proposals vs Pin ecosystem (Boats — pinned places comparison)

- **Mental model:** Competing proposals (e.g. Yosh-adjacent self-ref types discussions) trade backward compat vs conceptual cleanliness.
- **Key insight:** Evaluate along migration cost + FFI + dyn async — not syntax alone.

## [TAG: 05-anti-patterns] “Pin is conditional” rebuttal (Boats — Pin usability)

- **Mental model:** Many features are conditional (`cfg`, const generics branches) without being hated — Pin’s trouble is API parity, not boolean nature.
- **Key insight:** Pushback against Yosh lobste.rs conditional framing — useful meta debugging of learner complaints.

## [TAG: 07-async-concurrency] `Pin::set` vs `*mut` assignment (Boats — Pin usability)

- **Mental model:** Ordinary `&mut` assignment uses `=`; pinned mutation uses `set` — small API friction × many calls = perceived hostility.
- **Key insight:** Supports language sugar proposals — not merely docs issue.

## [TAG: 02-language-rules] `swap` vs `Pin` invariant story (Boats — Move trait history)

- **Mental model:** `mem::swap` requires moving out of `&mut` — contradicts immovable values — explains why `Move` as ?Trait hits every generic API.
- **Key insight:** Makes `Pin` look comparatively local — only interactions through pinned handles change.

## [TAG: 12-modern-rust] `IntoFuture` swap counterexample (Boats — associated type bounds)

- **Mental model:** Associated types in traits implicitly carry Move assumptions today — relaxing requires two-crate world coordination.
- **Key insight:** Use when explaining why async trait work ties to solver and RPIT — not an isolated syntactic patch.

## [TAG: 01-meta-principles] Correctness ladder (Amos — “Aiming for correctness”)

- **Mental model:** Correctness is economic ladder — each rung costs opportunity; types shift some costs left in the timeline.
- **Key insight:** Push back on moralizing RIIR — replace with cost/benefit language for engineering audiences.

## [TAG: 05-anti-patterns] Implicit contracts in protocols (Amos — “Aiming for correctness”)

- **Mental model:** Real-world protocols rely on assumptions not in RFCs; attackers and hardware violate them — types can’t fix humanity but can shrink trusted computing base in your code.
- **Key insight:** SSH tarpit story illustrates adversarial implicit contracts — memorable teaching device for why parsing must be defensive.

## [TAG: 01-meta-principles] Rust advocacy perceived as elitist (Amos — “Aiming for correctness”)

- **Mental model:** “Think differently” reads as condescension until learners see payoff — communication hazard independent of technical merit.
- **Key insight:** LLM assistants should validate frustration before lecturing benefits — matches Amos’s emotional literacy.

## [TAG: 02-language-rules] Type-level newtypes vs runtime checks (Amos — correctness article)

- **Mental model:** Newtypes turn convention into construction — misuse becomes compile error not test gap.
- **Key insight:** Pair with serde / config stories carefully — boundary validation still required at IO edges.

## [TAG: 05-anti-patterns] Integer/float separation as machine honesty (Amos — “Curse of strong typing”)

- **Mental model:** Numeric tower types reflect distinct ALU paths — mixing is not pedantry; it’s modeling CPU + rounding semantics.
- **Key insight:** Use when learners ask why `2 * 3.14` fails — connect to hardware, not “compiler meanness.”

## [TAG: 01-meta-principles] Go simple / complexity relocation (Amos — “Golang wild ride”)

- **Mental model:** Simple language can export complexity to runtime behavior (`FileMode` fabrication) — observability worse than explicit types sometimes.
- **Key insight:** Cross-platform lies are documentation problem + testing problem — types help surface them.

## [TAG: 07-async-concurrency] `dyn` async trait object size — two-pointer mental picture (Amos — “Catching up with async Rust”)

- **Mental model:** `Box<dyn Trait>` for async methods is data pointer + vtable pointer; stepping through LLDB cements not zero-cost claim precisely.
- **Key insight:** Pair with static dispatch default in generic code — explain when `dyn` enters (plugin systems, heterogeneous collections).

## [TAG: 12-modern-rust] Native `async fn in traits` shrinks default dependency on macros (Amos — “Catching up with async Rust”)

- **Mental model:** Macro-expanded traits were de facto std for async traits — language feature removes a whole ecosystem shim for static dispatch.
- **Key insight:** Still leaves `dyn` path — don’t oversell “async traits solved” without object-safety caveat.

## [TAG: 02-language-rules] Stack reservation and `sizeof` future (Amos — disassembly section)

- **Mental model:** Compiler reserves max frame including future temporaries — understanding layout helps debug stack overflows in tiny embedded stacks.
- **Key insight:** Useful for embedded async audiences — large `async` fn bodies matter.

## [TAG: 05-anti-patterns] Teaching Rust with syntax-first half-hour (Amos — “Half-hour”)

- **Mental model:** Rapid symbol exposure reduces fear of reading — separate from writing safe code.
- **Key insight:** Pedagogy split: syntax tour early, ownership second — aligns with Amos’s own ordering.

## [TAG: 01-meta-principles] “You’re smarter than Rust” hook (Amos — “Frustrated?”)

- **Mental model:** Rust doesn’t infer your intent — it checks evidence you supply via types/trait bounds.
- **Key insight:** Reframe compiler as dumb verifier you instruct — empowers learners vs anthropomorphizing “compiler hates me.”

## [TAG: 12-modern-rust] Send bound series cliffhanger → RTN (Niko — part 1)

- **Mental model:** Problem statement complete in part 1; solutions intentionally sequenced across series — readers should follow order.
- **Key insight:** For LLMs: cite full series, not intro only, when discussing RTN vs trait transformers vs higher-ranked projections.

## [TAG: 12-modern-rust] `async_trait` crate as pragmatic baseline (Niko — contrast paragraph)

- **Mental model:** Macros are essential stopgaps — language design must respect what macros proved people need.
- **Key insight:** Anti-pattern: sneering at `async_trait` in production — it’s engineering under constraints.

## [TAG: 02-language-rules] Orphan rule & “foreign trait for local type” intuition (Niko — coherence lede)

- **Mental model:** Prevents two crates both adding conflicting `impl Display for MyType` if `MyType` is foreign — global uniqueness of meaning.
- **Key insight:** Weakening proposals try to restore opt-in impls with proof — not free-for-all.

## [TAG: 12-modern-rust] GAT MVP stabilization worries (Niko — many modes lede)

- **Mental model:** Ship minimal GATs to unblock real patterns while accepting rough edges — standard Rust stabilization pattern.
- **Key insight:** Opponents fear complexity spiral — proponents cite already macro-depends ecosystems — tension still live in community memory.

## [TAG: 07-async-concurrency] mini-redis timing bugs (Niko — cancellation case title)

- **Mental model:** Correct-looking async code may lose races only in production — types won’t save you from semantic async conventions.
- **Key insight:** Pair with Boats cancellation essays — full async safety includes temporal reasoning.

## [TAG: 01-meta-principles] Symposium / agentic Rust tooling (Niko — 2025–2026 index titles)

- **Mental model:** Rust ecosystem innovation now includes MCP/skills packaging — metadata as deliverable alongside crates.
- **Key insight:** For LLM knowledge bases: crate-local operational knowledge may ship as skills — watch Symposium pattern.

## [TAG: 12-modern-rust] Rust/Python/TypeScript trifecta hypothesis (Niko — index)

- **Mental model:** AI coding shifts language choice toward fundamentals — predicts Rust for perf/systems, Python for science, TS for web.
- **Key insight:** Speculative — treat as sociology, not compiler fact.

## [TAG: 01-meta-principles] EuroRust / Rust Nation travelogues as signal (Niko — Ubuntu Rust, reflections)

- **Mental model:** Conference posts capture industry adoption anecdotes — useful for context on Linux distro Rust commitments.
- **Key insight:** Distinguish marketing keynote from technical post — both appear in same blog feed.

---

# Part K — Historical anchors (for timeline reasoning)

- 2013 — External iterators shift (Daniel Micay mailing list); Boats cites as prehistory of state-machine style in Rust.
- 2014 — Green threads removal (RFC 230 precursors).
- 2016 — Turon/Crichton futures: readiness vs CPS; `join` allocation story.
- 2018 — Pin RFC + async/await RFC pair; `await!()` macro syntax.
- 2019 — Async/await on stable; await syntax debate burnout.
- 2023 — Async fn in traits nightly; send-bounds series; Boats “Why async Rust” reception shift.
- 2024 — Boats “Pinned places”; Amos “Catching up with async Rust”; Niko “Rust in 2025” series titles appear.
- 2026 — Fetched pages show Symposium, Dada posts, continuing dyn async sequence — verify dates when citing “current.”

---

# Part L — Anti-pattern / pattern cheat sheet (expert blog consensus)

This section compresses recurring warnings into actionable patterns. It is not exhaustive safety guidance — always pair with official docs.

## Async / await

1. Don’t hold `std::sync::Mutex` guards across `.await` unless you know exactly which thread pool and latency you can tolerate — prefer `tokio::sync::Mutex` (or restructure) for async-critical paths.
2. `select!` loops re-create branches — if a branch future is long-lived (socket handshake, mutex acquisition with queue position), hoist + fuse + pin it.
3. Completion-based IO (io-uring) breaks naive “read is restartable” assumptions — cancellation can drop bytes — cancellation safety is reactor-dependent.
4. `spawn` + `Send`: bounding `H: Send` is insufficient — the opaque future returned by async methods must be `Send` when work-stealing can move tasks at await points.
5. Recursive `async fn`: requires explicit `Box::pin` (or equivalent) — treat like recursive `struct` sizing.

## Pin / self-reference

1. `Pin` is not “for user self-referential structs” primarily — it’s the compat story for compiler-generated self-referential futures and unsafe pin APIs.
2. Projections: field access through `Pin<&mut T>` is not automatically safe — `Unpin`, `pin-project`, or (future) `pinned` fields — pick consciously.
3. `Drop` + `Pin` mismatch is historical — expect `pin-project`-style patterns until language catches up.

## Traits / generics

1. `async_trait` macro: great interop; forces boxing in the expanded trait — understand cost vs native async fn in traits for static dispatch.
2. GATs: powerful for lending iterators, mode-parameterized APIs, Embassy-style patterns — expect verbosity and occasional compiler MVP rough edges.
3. Orphan rule: if you can’t write an impl, consider newtype wrapper or upstream trait — weakening orphan is lang discussion, not a quick hack.

## Types / numerics

1. Numeric literals have distinct types — mixing `integer`×`float` without explicit conversion is refusing silent widening — not pettiness.
2. Overflow: Rust debug vs release profiles differ — don’t confuse language rules with `-C overflow-checks` settings.

## Cross-platform / FFI

1. Simulated POSIX semantics on Windows leak surprising values — explicit `OsString`/`Path` types encode uncertainty — embrace them at boundaries.

---

# Part M — Где искать «канон» помимо блогов

Нормативные решения (что именно в языке) — в RFC и в dev guide rustc; блоги дают мотивацию и картинку в голове. Для async: история отказа от зелёных потоков, RFC про `Pin` и про async/await, документация async WG / Async Vision — там же обсуждения, которые потом превращаются в стабилизации. Для спорных тем вроде RTN и object-safe async trait обсуждение часто опережает блоги в рабочих группах.

---

# Part N — Additional study prompts (batch 2)

1. Explain why `Future` uses `Poll` not callbacks — tie to `join` allocation story.
2. What does “lexically scoped effects” mean for `?` and `await` in Rust vs effect handlers?
3. How does Boats argue `AsyncIterator` is not “just async `Iterator`” using the product metaphor?
4. Why might `async fn next` cause per-iteration allocation under `dyn`?
5. What is `intra-task concurrency` and how does it differ from `spawn`?
6. How does Amos use LLDB to teach `dyn` async costs?
7. Summarize Niko’s argument against adopting `async-trait` macro semantics natively for std.
8. Why does Niko think `many modes` GAT pattern matters to stabilization debates?
9. What governance lesson does Boats draw from await postfix debate burnout?
10. Connect curl CVE story to hand-written poll state machines.

---

# Part O — Glossary (blog coinages & shorthand)

- Pinned typestate — memory for this object must not be moved away until destructor runs (Boats / Jung).  
- Perfectly sized stack — monolithic task future containing all nested await state (Boats).  
- Intra-task concurrency — `select!`/`join!` embedding multiple futures inside one state machine (Boats).  
- Multi-task concurrency — `spawn` + channels; closer to threads (Boats).  
- Function coloring — async vs sync function distinction; debated term from Nystrom; Rust encodes in types (Boats).  
- Keyword generics / effects — hypothetical abstract over async/sync in traits; controversial; interacts with `async fn next` debates (Boats).  
- Poll next vs async next — two designs for `AsyncIterator`; two state machines vs one (Boats).  
- Cancellation safety — informal property; Boats warns it’s not universal correctness criterion — depends on IO model.  
- Soul of Rust — transparency vs ergonomics tension in dyn async allocation debates (Niko).  
- Many modes pattern — GAT-parameterized parser/DSL style; chumsky-family (Niko).  
- Implicit contracts — protocol assumptions not fully specified; Amos narrative frame for correctness.  
- RIIR — “Rewrite It In Rust”; advocacy dynamics in Amos meta sections.

---

# Part P — Coverage matrix (authors × taxonomy)


| Taxonomy tag         | fasterthanli (Amos)                                                        | baby steps (Niko)                                       | without.boats (Boats)                                                     |
| -------------------- | -------------------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------- |
| 01-meta-principles   | Advocacy psychology; learning curve; implicit contracts; “simple” critique | Design axioms; project goals; community ownership       | Async history + economics context; governance burnout; Eriksen vs Nystrom |
| 02-language-rules    | Numeric types; future stack layout; string/type proliferation themes       | GAT patterns; coherence/ orphan; trait solver adjacency | Pin/`Move` assoc type argument; `async fn` inner vs outer returns         |
| 05-anti-patterns     | Overflow stories; cross-platform lies; “expertise hurts”                   | mini-redis timing pitfalls                              | `select!` loops; hand-written SM bugs; mutex across await                 |
| 07-async-concurrency | `async-trait` expansion; `dyn` costs; sizeof futures                       | `Send` future bounds; thread affinity appendix          | Entire async corpus: poll, tasks, intra-task, io-uring cancellation       |
| 12-modern-rust       | Dec 2024 async trait post                                                  | send-bounds series; dyn async series; preview crates    | four-year plan; pinned places; RPITIT/RTN; keyword generics skepticism    |


---

# Part Q — One-line “expert takes” (high density)

1. Rust async is poll-based because CPS made `join` allocate — readiness inversion preserves single state-machine object (Boats).
2. Green threads died: stack growth incompatible with Rust’s pointer rules without GC (Boats).
3. `Pin` exists because `?Move` can’t be added compatibly — pinning scopes immovability to pinned handles (Boats).
4. `Pin` ergonomics pain is mostly missing reborrow sugar — not `Unpin` confusion alone (Boats).
5. Effect handlers hide effect flow behind dynamic scoping — Rust prefers visible `await`/`?` sites (Boats).
6. `AsyncIterator`: `poll_next` keeps one pinned machine for whole loop — `async next` splits machines — perf/pinning/cancellation consequences (Boats).
7. `select!` loops cancel losing branches — hoist long-lived futures — merge may fit stream-shaped workloads (Boats).
8. `async fn in traits` needs caller-side `Send` bounds — `T: Send` insufficient (Niko).
9. `async_trait` macro trades flexibility for boxed futures — fine for ecosystem, wrong as only std story (Niko).
10. GAT “many modes” shows real ecosystem patterns — not pure complexity for its own sake (Niko).
11. Orphan rule protects crate pairing — too strict sometimes — weakening needs careful design (Niko).
12. Learning Rust is acquiring new ontologies — not learning syntax (Amos).
13. Strong typing blocks numeric accidents JS/Go hide until late (Amos).
14. Go simplicity can relocate complexity into surprising OS behavior (Amos).
15. `dyn` async shows vtable + data pointer — dynamic dispatch has measurable size/branch costs (Amos).
16. Async future size scales with locals alive across await — large stacks risk in embedded (Amos).
17. Network services focus shaped early async investment — explains ecosystem skew (Boats).
18. Await postfix debate burnout: close decisions when arguments repeat — process lesson (Boats).
19. Four-year async stagnation after MVP hurt perception — fixable with shipping features (Boats).
20. Pinned places proposal aims to restore reference parity for pin without breaking Pin ecosystem (Boats).

---

# Part R — FAQ-style mismatches (for LLM error correction)

Q: Is `Pin` only for async?  
A: Primary motivation is async lowering + unsafe pin APIs; user self-referential structs remain partial / unsafe story (Boats).

Q: Does Rust async “add threads”?  
A: Default is tasks on executor — OS threads optional; many tasks may map to few threads (Boats).

Q: Why not Haskell-style IO monad instead?  
A: Monads order effect layers; coroutines yield multiple effect classes without layering — Rust still uses `Result` for errors separately (Boats).

Q: Are Go’s goroutines “coroutines”?  
A: Boats uses narrow coroutine definition; preemptive schedulers often called green threads — terminology confusion is normal in HN threads (Boats).

Q: Is `Send` only for multithreading?  
A: In work-stealing, tasks move at await — `Send` gates internal future state, not only outer values (Niko).

Q: Can I always `async_trait` my way out?  
A: You can, but you pay boxed futures and dyn ergonomics — native async traits aim to preserve unboxed static dispatch where possible (Niko/Amos).

Q: Does strong typing mean “more typing”?  
A: Often yes in characters; fewer debugging hours — tradeoff depends on domain (Amos).

Q: Is Rust “over” after async shipped?  
A: Blogs from 2023–2026 emphasize ongoing integration: AsyncIterator, RTN, generators, dyn async — treat async as multi-year arc (Boats/Niko).

---

# Part S — Coverage gaps (кратко)

- По полным текстам не все посты Нико про GATs/coherence удавалось открыть отдельными страницами — смысл по ним частично из заголовков и лидов в индексе блога + из поста про Send bounds.  
- У Boats на главной есть ещё материалы (например про ownership и ссылки), в этот проход они не входили — при необходимости их можно добавить отдельно.

---

*Конец cluster-10-blogs.md. Сроки стабилизации фич сверяйте с актуальными RFC и release notes.*