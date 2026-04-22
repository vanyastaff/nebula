# 4. Design Patterns

[← Idioms](./03-idioms.md) | [README](./README.md) | [Next: Anti-Patterns →](./05-anti-patterns.md)

---

## [P-001] Command

Three shapes, pick by closure property:

```rust
// A) Trait object — commands are full types with state
trait Migration: Send + Sync {
    fn execute(&self) -> Result<()>;
    fn rollback(&self) -> Result<()>;
}
struct Schema { cmds: Vec<Box<dyn Migration>> }

// B) Function pointer / closure — stateless, cheap
type Exec = fn() -> Result<()>;
struct Cmd { exec: Exec, undo: Exec }

// C) Enum — closed, known at compile time
enum Cmd { CreateTable(Table), AddField(Field) }
impl Cmd {
    fn execute(&self) -> Result<()> { match self { /* … */ } }
}
```

Rules:

- Open set + extern plugins → trait object (A).
- Closed set + exhaustiveness → enum (C).
- Commands = single functions → fn pointer (B).

## [P-002] Interpreter

For small DSLs, prefer declarative macros (`macro_rules!`) — zero runtime cost. For non-trivial grammars (config, queries), use `nom`, `chumsky`, `pest`, `logos`, `winnow`. Build a recursive-descent parser by hand only for education or minimal-dependency constraints.

```rust
macro_rules! l2norm {
    ($($e:expr),+) => {{ let mut n = 0.0_f64; $(n += ($e as f64).powi(2);)+ n.sqrt() }};
}
```

## [P-003] Newtype

```rust
// Identifier safety
pub struct UserId(u64);
pub struct OrderId(u64);

// Units
pub struct Meters(pub f64);
pub struct Feet(pub f64);

// Validated value
pub struct SanitizedInput(String);
impl SanitizedInput {
    pub fn new(s: &str) -> Result<Self, InvalidInput> { /* validate */ Ok(Self(s.into())) }
    pub fn as_str(&self) -> &str { &self.0 }
}
```

Do NOT derive `Deref` on a newtype unless the newtype is *genuinely* a pointer. See `[A-003]`. For cutting boilerplate: `derive_more` (`Display`, `From`, arithmetic).

## [P-004] RAII guards

```rust
pub struct Tx<'db> {
    db: &'db Database,
    committed: Cell<bool>,
}
impl Drop for Tx<'_> {
    fn drop(&mut self) {
        if !self.committed.get() {
            self.db.rollback();
        }
    }
}
impl<'db> Tx<'db> {
    pub fn commit(self) { self.committed.set(true); /* drop runs, but rollback skipped */ }
}
```

Applies to: pools, buses, GPU contexts, transactions, unsafe init scopes.

## [P-005] Strategy

```rust
// Static dispatch
pub trait Formatter { fn format(&self, d: &Data, buf: &mut String); }
fn report<F: Formatter>(f: F, d: &Data) -> String { /* … */ }

// Via closures for single-function strategies
fn apply<F: Fn(u8, u8) -> u8>(x: u8, y: u8, f: F) -> u8 { f(x, y) }

// Dynamic dispatch when strategies are stored heterogeneously
struct Report { strategies: Vec<Box<dyn Formatter>> }
```

Do NOT create a `StrategyFactory` trait. Rust does not need it.

## [P-006] Visitor

Walk a heterogeneous AST with stateful traversal:

```rust
pub trait Visitor {
    fn visit_stmt(&mut self, s: &Stmt) { walk_stmt(self, s); }
    fn visit_expr(&mut self, e: &Expr) { walk_expr(self, e); }
    fn visit_name(&mut self, _n: &Name) {}
}

pub fn walk_expr<V: Visitor + ?Sized>(v: &mut V, e: &Expr) {
    match e {
        Expr::Add(l, r) | Expr::Sub(l, r) => { v.visit_expr(l); v.visit_expr(r); }
        Expr::Lit(_) => {}
    }
}
```

Default methods + free `walk_*` functions is the convention (as in `syn::visit`). Justified when state must flow between nodes; for pure traversal, `Iterator` is simpler.

## [P-007] Fold

Transform a structure into a new structure:

```rust
pub trait Folder {
    fn fold_expr(&mut self, e: Expr) -> Expr {
        match e {
            Expr::Add(l, r) => Expr::Add(Box::new(self.fold_expr(*l)),
                                         Box::new(self.fold_expr(*r))),
            leaf => leaf,
        }
    }
}
```

Ownership choice:

- `Box<Node>`: cheap reuse of unchanged subtrees, consumes original.
- `&Node` + `Clone`: preserves original, clones everywhere (expensive).
- `Rc<Node>`/`Arc<Node>`: structural sharing, immutable.

Used in macro expansion, AST → HIR → MIR lowering, optimization passes.

## [P-008] Builder

Three variants — pick by construction style:

```rust
// A) Owning chain — simple, no conditional fields
Foo::builder().name("x").timeout(d).build();

// B) Mutable-borrow two-step — conditional construction
let mut b = Foo::builder();
b.name("x"); if cfg { b.timeout(d); }
let f = b.build();

// C) Typestate — enforce required fields at compile time
pub struct Missing; pub struct Present<T>(T);
pub struct ReqBuilder<U, M> { url: U, method: M }
impl ReqBuilder<Missing, Missing> { pub fn new() -> Self { /* … */ } }
impl<M> ReqBuilder<Missing, M> {
    pub fn url(self, u: Url) -> ReqBuilder<Present<Url>, M> { /* … */ }
}
impl ReqBuilder<Present<Url>, Present<Method>> {
    pub fn build(self) -> Request { /* … */ }
}
```

Automation: `bon` crate (handles `impl Trait`, async, generics). Avoid `derive_builder` in new code; it is legacy.

## [P-009] Struct decomposition for independent borrowing

The borrow checker sees fields independently but not across methods. Split a struct when multiple methods contend on disjoint fields:

```rust
// Bad — &mut self on any method locks everything
struct Db { cs: String, pool: Pool, logger: Logger }
impl Db {
    fn acquire(&mut self) { self.pool.lease(); self.log(); }
    fn log(&self) { println!("{}", self.cs); }   // borrow conflict in acquire
}

// Good — independent borrows
struct Db { cfg: Cfg, pool: Pool, logger: Logger }
```

If decomposition doesn't map to domain concepts, the root cause is design debt — refactor the architecture, not just the struct.

## [P-010] Small crates

Split a project into focused crates when:

- Public API boundaries are clear.
- Compile parallelism matters (workspace > 10 crates).
- Features need to be selectable.

Costs:

- Version skew (duplicate incompatible `url`).
- No cross-crate LTO by default — set `lto = "thin"` in release profile.
- Longer clean builds.

For workspaces with 20+ crates, use `cargo-hakari` to unify features.

## [P-011] Contain unsafety in small modules

```rust
mod ring {
    use std::cell::UnsafeCell;
    pub struct Ring<T> { /* private fields */ }
    impl<T> Ring<T> {
        pub fn push(&self, v: T) -> Result<(), T> { /* unsafe inside */ }
        pub fn pop(&self) -> Option<T> { /* unsafe inside */ }
    }
}
```

Rules:

- Every `unsafe` block: SAFETY comment.
- Edition 2024: `unsafe_op_in_unsafe_fn = deny` (default).
- Module size: 1–3 types, 10–20 methods.
- Tests: unit tests + Miri (always) + loom (for lock-free).

## [P-012] Custom trait to collapse bounds

When bounds become a chain `F: FnMut() -> Result<T, E>, T: Display, …`, introduce a trait with a blanket impl:

```rust
pub trait Getter {
    type Output: Display;
    fn get(&mut self) -> Result<Self::Output, Error>;
}

impl<F, T> Getter for F
where F: FnMut() -> Result<T, Error>, T: Display,
{
    type Output = T;
    fn get(&mut self) -> Result<T, Error> { self() }
}

// Now callers can:
struct Probe<G: Getter, S: Fn(&G::Output) -> Status> { /* … */ }
```

Saves one type parameter, improves readability, enables alternative implementations (mocks, caches).

## [P-013] FFI Object-based API

Opaque owned type + transparent transactional type:

```rust
pub struct Db { /* opaque */ }
#[repr(C)] pub struct Datum { pub ptr: *const u8, pub len: usize }

#[no_mangle] pub extern "C" fn db_open(path: *const c_char) -> *mut Db { /* … */ }
#[no_mangle] pub extern "C" fn db_close(db: *mut Db) { /* … */ }
#[no_mangle] pub extern "C" fn db_get(db: *mut Db, key: Datum, out: *mut Datum) -> c_int { /* … */ }
```

- Encapsulated types: owned by Rust, managed by user via opaque pointer.
- Transactional types: transparent `#[repr(C)]`, owned by caller.
- Behavior: functions over encapsulated types (not trait objects).
- Avoid exporting iterators as separate types — embed cursor state in parent (POSIX DBM pattern).

## [P-014] FFI Type consolidation into wrappers

For multi-type Rust APIs exposed via FFI, fold related types into one wrapper. Store iterator *state* (index), reconstruct iterator per-call — never store a live iterator with `'self` transmuted to `'static`.

```rust
// Good
pub struct DbWrap {
    db: Db,
    iter_pos: usize,
}
impl DbWrap {
    pub fn next_key(&mut self) -> Option<&[u8]> {
        let k = self.db.keys().nth(self.iter_pos)?;
        self.iter_pos += 1;
        Some(k)
    }
}

// Bad — stores borrowed iterator; requires unsound transmute
pub struct DbWrap {
    db: Db,
    iter: Option<std::collections::hash_map::Keys<'static, Key, Val>>, // UB source
}
```

---

[← Idioms](./03-idioms.md) | [README](./README.md) | [Next: Anti-Patterns →](./05-anti-patterns.md)