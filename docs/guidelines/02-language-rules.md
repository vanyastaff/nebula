# 2. Language Rules (from the Rust Reference)

[← Meta-Principles](./01-meta-principles.md) | [README](./README.md) | [Next: Idioms →](./03-idioms.md)

---

These are correctness rules derived from the Reference. Violating them yields UB, miscompilation, or broken invariants the compiler relies on for optimization. `L-` rules are **hard constraints** — they take precedence over every other category.

**Jump to:**
- [2.1 Undefined Behavior](#21-undefined-behavior-catalog)
- [2.2 Drop Order and Temporaries](#22-drop-order-and-temporaries)
- [2.3 Subtyping and Variance](#23-subtyping-and-variance)
- [2.4 Type Layout and `#[repr]`](#24-type-layout-and-repr)
- [2.5 Trait Objects and Dyn Compatibility](#25-trait-objects-and-dyn-compatibility)
- [2.6 Coherence and Orphan Rules](#26-coherence-and-orphan-rules)
- [2.7 Pattern Matching Rules](#27-pattern-matching-rules)
- [2.8 Unsafe Contracts](#28-unsafe-contracts)
- [2.9 Special Types and Traits](#29-special-types-and-traits)
- [2.10 Expressions and Operators](#210-expressions-and-operators)
- [2.11 Items and Attributes](#211-items-and-attributes)
- [2.12 Macros](#212-macros)

---

## 2.1 Undefined Behavior Catalog

UB is forbidden in *all* code, including inside `unsafe` blocks. `unsafe` does not license UB; it transfers the burden of proving absence from the compiler to the programmer.

### [L-UB-001] The fixed UB list

Any of the following, anywhere in any Rust program, is UB:

1. **Data races** — concurrent unsynchronized accesses where at least one is a write.
2. **Dereferencing a dangling or misaligned pointer** (load or store through it).
3. **Out-of-bounds place projection** (field access, index, tuple index through an invalid pointer).
4. **Breaking aliasing rules**: `&T` requires the pointee not to be mutated (except through `UnsafeCell`); `&mut T` requires exclusive access. `Box<T>` ≈ `&'static mut T` for aliasing purposes.
5. **Mutating immutable bytes** — bytes owned by an immutable binding, reachable through a shared reference (non-`UnsafeCell`), or const-promoted are immutable.
6. **Invoking UB via compiler intrinsics**.
7. **Executing code with unsupported `target_feature`** on the current CPU.
8. **Calling a function with the wrong ABI**, or unwinding across a frame that forbids it (e.g., `extern "C"`).
9. **Producing an invalid value** (see `[L-UB-003]`).
10. **Incorrect inline assembly**.
11. **Violating runtime assumptions** (e.g., `longjmp`-ing past Rust frames, skipping destructors for stack-allocated types).

### [L-UB-002] `&T` and `&mut T` semantics

- **`&T` is read-only aliased:** the memory it points to must not be mutated during its live range, except through `UnsafeCell`.
- **`&mut T` is unique:** no other reference (shared or mutable) to the same memory may exist during its live range.
- **Each reborrow resets liveness.** When a reference is passed to a function, it is live at least for the duration of the call.
- **Liveness is upper-bounded by the borrow checker's syntactic lifetime.**

```rust
// Bad — creating &mut from & without UnsafeCell: UB
fn wrong(r: &i32) {
    let m = unsafe { &mut *(r as *const i32 as *mut i32) };
    *m = 5; // UB: mutates through shared reference
}

// Good — use UnsafeCell when shared mutability is needed
use std::cell::UnsafeCell;
struct SharedMut(UnsafeCell<i32>);
// SAFETY on any reader: synchronize manually (atomics/lock) before reading.
```

### [L-UB-003] Invalid values (per-type validity)

Producing any of the following is UB, even in private fields:

- `bool` not in `{0, 1}`.
- `fn` pointer = null.
- `char` a surrogate (`0xD800..=0xDFFF`) or > `char::MAX`.
- Any value of type `!`.
- Uninitialized `i*`/`u*`/`f*`/raw pointer/`bool`/`char` read (uninit only allowed inside `union` fields and padding).
- `str` containing uninitialized bytes.
- `enum` with an invalid discriminant.
- `NonNull<T>` holding 0; `NonZero<_>` holding 0.
- `&T`/`&mut T`/`Box<T>` that is null, misaligned, dangling, or points to an invalid value.
- Wide pointer metadata mismatch (e.g., `dyn Trait` vtable not matching the actual type).

```rust
// Bad — MaybeUninit misused
let x: i32 = unsafe { std::mem::MaybeUninit::<i32>::uninit().assume_init() };
// UB: produces invalid i32 (uninit)

// Good — write first, then assume_init
let mut u = std::mem::MaybeUninit::<i32>::uninit();
u.write(42);
let x: i32 = unsafe { u.assume_init() };
```

### [L-UB-004] Pointer alignment

A place is "based on a misaligned pointer" if the last `*` dereference in its computation was through a pointer not aligned for its pointee type. Reading or writing such a place is UB.

- `&raw const/mut` on a misaligned place is **OK** (it produces a raw pointer; just do not dereference it without `read_unaligned`/`write_unaligned`).
- `&`/`&mut` on a misaligned place is **UB producing an invalid reference**.
- `#[repr(packed)]` fields cannot be safely referenced by `&`; use `&raw const/mut` + `read_unaligned`/`write_unaligned`.

```rust
#[repr(packed)]
struct P { a: u8, b: u32 }

let p = P { a: 0, b: 0x1234_5678 };

// Bad
let r: &u32 = &p.b;               // ⚠ might be UB (unaligned ref)

// Good
let v: u32 = p.b;                 // copy out
let ptr = &raw const p.b;         // raw pointer, legal even unaligned
let v2 = unsafe { ptr.read_unaligned() };
```

### [L-UB-005] Provenance in const

Inside `const`/`static` evaluation, any byte-value holding integer data must have no provenance. Transmuting a pointer with provenance to `usize` in const is UB even if it would be valid at runtime.

### [L-UB-006] Dangling pointers

A pointer/reference is dangling if not all of its `size_of_val` bytes lie within a single live allocation. Zero-sized pointees are never dangling (including null). Slice/str metadata that makes total size exceed `isize::MAX` is invalid.

---

## 2.2 Drop Order and Temporaries

Drop rules affect correctness (guards releasing too early/late) and are frequently misunderstood. Generated code MUST assume these rules rather than running experiments.

### [L-DROP-001] Variable drop order

Variables in a scope are dropped in **reverse declaration order**. Function parameters are dropped *after* locals (they come first in declaration). Closure captures drop in unspecified order.

```rust
// Drops in order: c, b, a
{
    let a = Log("a");
    let b = Log("b");
    let c = Log("c");
}
```

### [L-DROP-002] Field drop order within a compound

Fields of a struct/enum variant/tuple drop in **declaration order** (not reverse). Array elements drop from first to last. Owned slice elements: first to last.

```rust
struct S { a: Log, b: Log, c: Log }
// On drop of S, runs a.drop(), then b.drop(), then c.drop().
```

This matters when later fields reference earlier fields via `Arc`/`Rc` cycles or when a `Drop` impl expects a specific teardown sequence. Order fields accordingly.

### [L-DROP-003] Temporary scope (default)

Absent lifetime extension, a temporary (rvalue) is dropped at the end of the **smallest enclosing** scope that is one of: whole function; a statement; the body of `if`/`while`/`loop`; the `else` block of `if`; the condition of `if`/`while` or a match guard; the body of a match arm; operand of `&&`/`||`.

For a `match <scrutinee> { … }`, do **not** rely on subtle rules about how long temporaries inside `<scrutinee>` live relative to the arms. If `<scrutinee>` creates an RAII guard (mutex, file, transaction), **bind it in a `let` before the `match`** so the guard’s lifetime is obvious in reviews and stable across compiler versions.

```rust
// Bad — `MutexGuard` temporary from `lock()` is not held across the match arms
match data.lock().unwrap().value { /* … */ }

// Good — bind the guard, then match on the protected data
let g = data.lock().unwrap();
match g.value { /* … */ }
drop(g);
```

### [L-DROP-004] Temporary lifetime extension (let bindings)

A temporary used in the initializer of `let PAT = EXPR` where `PAT` is an *extending pattern* (including `let ref`, `let ref mut`, a struct/tuple/or-pattern whose subpattern is extending) gets its lifetime extended to the enclosing block.

```rust
let x: &Vec<i32> = &vec![1, 2, 3];  // Vec lives until end of block (extended)
```

### [L-DROP-005] On-stack dynamic dispatch (Rust 1.79+)

Binding a temporary via `&` or `&mut` to a variable at function scope extends it — so no `Box` is needed for conditional dynamic dispatch:

```rust
// Good — no heap allocation
let reader: &mut dyn Read = if cond { &mut io::stdin() } else { &mut fs::File::open(p)? };
```

### [L-DROP-006] Partial moves drop remaining fields only

After a partial move out of a struct/tuple, only the fields that remain initialized are dropped.

```rust
let t = (Log("a"), Log("b"));
std::mem::forget(t.1);   // t.1 now uninitialized (as far as drop goes)
// at scope end: only t.0 is dropped.
```

### [L-DROP-007] Drop is not guaranteed

`Drop` does NOT run on: `mem::forget`, reference cycles in `Rc`/`Arc`, `process::exit`/`abort`, double-panic (the second panic skips remaining drops), or when a test harness kills the process. DO NOT rely on `Drop` for critical finalization (flushing WAL, releasing external locks); provide an explicit `close()` method and document Drop as best-effort.

### [L-DROP-008] Drop cannot `.await`

Async resources cannot be cleanly released in `Drop` today. For async teardown:

```rust
// Good — explicit async close, Drop emits warning if skipped
impl Handle {
    pub async fn close(mut self) -> Result<()> {
        self.flush().await?;
        self.closed = true;
        Ok(())
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.closed { tracing::warn!("Handle dropped without close()"); }
    }
}
```

---

## 2.3 Subtyping and Variance

Rust subtyping is entirely about lifetimes. `'long <: 'short` means `'long` outlives `'short` (so `&'long T` can be used where `&'short T` is expected).

### [L-VAR-001] Variance table (memorize)

| Type | Variance in `T` | Variance in `'a` |
|------|-----------------|-------------------|
| `&'a T` | covariant | covariant |
| `&'a mut T` | **invariant** | covariant |
| `*const T` | covariant | — |
| `*mut T` | **invariant** | — |
| `Box<T>`, `Vec<T>`, `Rc<T>`, `Arc<T>` | covariant | — |
| `Cell<T>`, `RefCell<T>`, `UnsafeCell<T>`, `Mutex<T>` | **invariant** | — |
| `fn(T) -> U` | **contravariant** in `T`, covariant in `U` | — |
| `NonNull<T>` | covariant | — |
| `PhantomData<T>` | covariant | — |

If a type parameter `T` appears in multiple positions with differing variance, the result is **invariant**.

### [L-VAR-002] Structs inherit variance from fields

```rust
// Covariant in 'a and T, invariant in 'b and U
struct S<'a, 'b, T, U> {
    a: &'a T,             // covariant 'a and T
    b: UnsafeCell<&'b ()>, // invariant 'b
    u: *mut U,            // invariant U
}
```

### [L-VAR-003] Unused parameters require `PhantomData`

If a generic parameter does not appear in any field, the type is disallowed. Use `PhantomData` to record intended variance:

```rust
use std::marker::PhantomData;

// Good — signals "owns T" for drop check + covariance
struct MyBox<T> {
    ptr: std::ptr::NonNull<T>,
    _owns: PhantomData<T>,
}

// Good — phantom lifetime, covariant
struct Ref<'a, T> {
    ptr: std::ptr::NonNull<T>,
    _life: PhantomData<&'a T>,
}

// Good — forces invariance
struct Invariant<T> {
    _m: PhantomData<fn(T) -> T>,
}

// Good — !Send and !Sync
struct NotSendSync<T> {
    _m: PhantomData<*const T>,
}
```

### [L-VAR-004] When writing a custom unsafe collection/smart pointer

Always add `PhantomData<T>` for the owned-T case, even when using `NonNull<T>`. `NonNull` is *covariant* but has no drop-check knowledge; without `PhantomData<T>` the drop checker may let dangling references survive into your `Drop`.

---

## 2.4 Type Layout and `#[repr]`

### [L-REPR-001] Default `repr(Rust)` layout is unspecified

The compiler may reorder fields, add padding, niche-pack enum discriminants, etc. You MUST NOT assume field order, transmutability, or cross-version layout stability.

### [L-REPR-002] `#[repr(C)]` for FFI and layout control

`#[repr(C)]` guarantees:
- Field order = declaration order.
- Alignment = max of field alignments.
- Size = current offset rounded up to alignment after all fields.
- Use for FFI structs, `transmute` to/from C types, memory-mapped IO.

```rust
#[repr(C)]
struct Header {
    magic: u32,
    version: u16,
    flags: u16,
    length: u64,
}
```

### [L-REPR-003] `#[repr(transparent)]` for newtypes that must match ABI

`#[repr(transparent)]` on a struct/single-variant enum with exactly **one non-zero-sized field** (plus any number of zero-sized fields with alignment 1) guarantees identical layout + ABI to that field. Required when:
- Passing a newtype across FFI where the inner type is expected.
- Transmuting between newtype and inner type.
- Using a newtype at the Rust/C boundary without redundant conversion.

```rust
#[repr(transparent)]
pub struct Nanoseconds(u64);   // same ABI as u64
```

Caveat: do not use `repr(transparent)` with generic ZSTs you don't control — they may become non-zero-sized in a later version.

### [L-REPR-004] `#[repr(packed)]` requires `&raw` for field access

Packed fields may be unaligned. `&packed.field` is undefined / a compile error for packed-inner alignment. Use `&raw const field` or copy by value.

### [L-REPR-005] `#[repr(u8)]` etc. on enums

For fieldless enums with `#[repr(u8/u16/u32/.../isize)]`, the discriminant occupies that integer type. This is **not** equivalent to a C enum — Rust enums can only hold declared discriminants; holding any other value is UB. To interop with C enums that may have arbitrary integer values, use `#[repr(transparent)] struct Foo(u32);` with `const` values, not a Rust `enum`.

```rust
// Bad — would be UB if C sends value 99
#[repr(u32)]
enum CStatus { Ok = 0, Busy = 1 }

// Good — accept any u32
#[repr(transparent)]
pub struct CStatus(pub u32);
impl CStatus {
    pub const OK: Self = Self(0);
    pub const BUSY: Self = Self(1);
}
```

### [L-REPR-006] Zero-sized types rules

- `size_of::<()>()` == 0, `align_of::<()>()` == 1.
- A `*const T` / `*mut T` / `&T` / `&mut T` to a ZST is never dangling regardless of the address.
- Multiple ZST fields in a struct may share address 0.
- A ZST + sized type layout is identical to the sized type alone under `repr(Rust)` but not guaranteed under `repr(C)`.

### [L-REPR-007] `Option<T>` niche optimization

`Option<T>` has the same layout and ABI as `T` when `T` has a "niche" (unused bit-pattern):
- `&T`, `&mut T`, `Box<T>` (use null).
- `NonNull<T>`, `NonZero<_>`.
- `fn` pointers (use null).

This is guaranteed for FFI. You MAY rely on `Option<&T>` == `*const T` at the boundary.

---

## 2.5 Trait Objects and Dyn Compatibility

### [L-DYN-001] Dyn compatibility rules

A trait `Trait` can be used as `dyn Trait` iff all of:
- All supertraits are dyn-compatible.
- `Self: Sized` is NOT a supertrait bound.
- No associated constants.
- No associated types with generics.
- Every method is either dispatchable or has `where Self: Sized`.

A method is dispatchable iff:
- No generic type parameters (lifetime params OK).
- Self appears only as the receiver (`&self`, `&mut self`, `Box<Self>`, `Arc<Self>`, `Rc<Self>`, `Pin<&mut Self>`).
- Not an `async fn` (returns a hidden `impl Future`).
- Not an RPIT method (`-> impl Trait`).
- No `where Self: Sized` (that would mark it non-dispatchable, not incompatible).

### [L-DYN-002] Making a mostly-dyn-compatible trait

If a trait has a few generic methods, mark them `where Self: Sized` so the trait remains dyn-compatible:

```rust
pub trait Repository {
    fn find(&self, id: Uuid) -> Option<Entity>;
    fn save(&self, e: Entity);
    // Non-dispatchable but does not break dyn-compat:
    fn query<Q: Into<String>>(&self, q: Q) -> Vec<Entity> where Self: Sized;
}

// Good — dyn Repository works:
fn use_repo(r: &dyn Repository) { r.find(some_id()); }
```

### [L-DYN-003] `async fn` in traits + `dyn`

Stable `async fn in trait` (1.75+) is **not** dyn-compatible by default. Options:
- Use static dispatch (`impl Repository`) — usually correct.
- Use the `trait-variant` or `dynosaur` crate to derive a dyn-compatible shim.
- Manually define a parallel trait returning `Pin<Box<dyn Future<Output=_> + Send + 'a>>`.

```rust
// Static dispatch — no dyn needed
pub trait Repo { async fn find(&self, id: Uuid) -> Option<Entity>; }
fn use_repo<R: Repo>(r: &R) { /* … */ }

// Dyn-compatible shim
pub trait RepoDyn {
    fn find<'a>(&'a self, id: Uuid)
        -> Pin<Box<dyn Future<Output = Option<Entity>> + Send + 'a>>;
}
```

### [L-DYN-004] Trait object lifetime defaults

A `dyn Trait` has an implicit lifetime bound. Defaults:
- `&'a dyn Trait` → `&'a (dyn Trait + 'a)`
- `Box<dyn Trait>` → `Box<dyn Trait + 'static>`
- `Arc<dyn Trait>` → `Arc<dyn Trait + 'static>`
- `impl Trait` alias / struct field: `'static` default.

Specify explicitly when the default is wrong: `Box<dyn Trait + 'a>`.

### [L-DYN-005] Auto traits on `dyn`

`Send`, `Sync`, `Unpin`, `UnwindSafe`, `RefUnwindSafe` must be listed explicitly: `Box<dyn Error + Send + Sync>`. They do NOT inherit from the base trait.

---

## 2.6 Coherence and Orphan Rules

### [L-COH-001] Orphan rule

An `impl Trait for Type` is allowed iff at least one of:
- `Trait` is defined in the current crate, OR
- `Type` (or one of its generic parameters satisfying the "covered" condition) is defined in the current crate.

For generic impls `impl<T> ForeignTrait<T1,…> for T0`: at least one `Ti` must be local, and no uncovered type parameter may appear *before* the first local type.

```rust
// Bad — both foreign
impl std::fmt::Display for Vec<i32> { /* … */ }

// Good — newtype wrap
struct MyVec(Vec<i32>);
impl std::fmt::Display for MyVec { /* … */ }
```

### [L-COH-002] Fundamental types

`&T`, `&mut T`, `Box<T>`, `Pin<P>` are `#[fundamental]`. `Box<LocalType>` counts as "local" for orphan purposes. Adding a blanket impl to a fundamental type is a **major** breaking change.

### [L-COH-003] Overlap

Two impls must not overlap. `impl<T: Debug> Foo for T` overlaps with `impl<T: Display> Foo for T`. Specialization is unstable; do not rely on it.

### [L-COH-004] Constrained parameters

Every generic parameter in an `impl` must be either:
- A parameter of the Self type, or
- Constrained via an associated type of a bound on another parameter.

```rust
// Bad — T not constrained
impl<T> Foo for Bar {}

// Good
impl<T> Foo for Bar<T> {}
impl<T, U> Foo<U> for Bar<T> where U: Into<T> {}
```

---

## 2.7 Pattern Matching Rules

### [L-PAT-001] Exhaustiveness

`match` must cover every case reachable by the type. Use `_` only when a wildcard is semantically meaningful; prefer listing variants for maintainability on `#[non_exhaustive]` enums from other crates.

### [L-PAT-002] `if let` guards (Rust 1.95+)

```rust
match token {
    Token::Num(n) if let Some(v) = n.checked_mul(2) => use_doubled(v),
    Token::Num(n) => use_raw(n),
    _ => unreachable!(),
}
```

⚠ `if let` guards do NOT participate in exhaustiveness checking. Treat the guarded arm as fallible and always provide a non-guarded or wildcard arm.

### [L-PAT-003] Default binding modes

In a pattern, matching a reference-typed value automatically shifts to `ref`-binding mode:

```rust
let r: &Option<String> = &Some("x".into());
match r {
    Some(s) => { /* s: &String, not String */ }
    None => {}
}
```

Do not write `&Some(ref s) =>` explicitly; it's redundant in 2024 edition for most cases, but be aware of the rule when reading older code.

### [L-PAT-004] `@` bindings

Use `name @ pattern` to bind a value while matching:

```rust
match n {
    x @ 1..=10 => use_x(x),
    x @ _ => skip(x),
}
```

### [L-PAT-005] Or-patterns drop order

In an or-pattern like `A(x) | B(x)`, the **first** subpattern determines drop order for the bound variables. Keep or-pattern fields in a consistent layout.

---

## 2.8 Unsafe Contracts

### [L-UNSAFE-001] Every `unsafe` block MUST have a safety comment

Format:
```rust
// SAFETY: <explicit invariant being relied upon, why it holds here>
unsafe { … }
```

No exceptions. If you cannot articulate the invariant, the operation is not safe to perform.

### [L-UNSAFE-002] Edition 2024: `unsafe_op_in_unsafe_fn`

Inside an `unsafe fn`, each unsafe operation must still be wrapped in `unsafe { … }`. This is default-deny in Edition 2024. Do not disable this lint.

```rust
// Good
pub unsafe fn read_at(p: *const u8) -> u8 {
    // SAFETY: caller guarantees p is valid and points to initialized u8
    unsafe { *p }
}
```

### [L-UNSAFE-003] Public `unsafe fn` MUST document safety preconditions

```rust
/// Reads `n` bytes starting at `ptr` into a Vec.
///
/// # Safety
///
/// - `ptr` must be valid for reads of `n` bytes.
/// - `ptr` must be properly aligned for `u8` (always true).
/// - The memory referenced by `ptr` must not be mutated during the call.
pub unsafe fn read_bytes(ptr: *const u8, n: usize) -> Vec<u8> { /* … */ }
```

Every bullet is a precondition the caller MUST uphold.

### [L-UNSAFE-004] Minimize the unsafe block

Do not wrap large blocks in `unsafe`. Narrow to the exact operation:

```rust
// Bad
unsafe {
    let p = get_pointer();          // safe
    let v = *p;                     // unsafe
    println!("{}", v);              // safe
}

// Good
let p = get_pointer();
let v = unsafe {
    // SAFETY: p came from Box::leak, still live and exclusive here
    *p
};
println!("{}", v);
```

### [L-UNSAFE-005] `unsafe` boundaries go in small modules

Encapsulate raw operations in a module with narrow safe API. Outer code does not see `unsafe`.

```rust
mod raw {
    // Only this module has unsafe operations; everything it exports is safe.
    pub struct Ring<T> { /* fields not pub */ }
    impl<T> Ring<T> {
        pub fn push(&self, v: T) -> Result<(), T> { /* unsafe inside */ }
        pub fn pop(&self) -> Option<T> { /* unsafe inside */ }
    }
}
```

### [L-UNSAFE-006] `unsafe trait` and `unsafe impl`

Marker-like traits that impose safety requirements on implementors (e.g., `Send`, `Sync`, `Allocator`, `GlobalAlloc`) must be declared `unsafe trait`; impls must write `unsafe impl`. The caller of trait methods does NOT need an `unsafe` block.

### [L-UNSAFE-007] Use `&raw const/mut` — never build a reference to get a pointer

```rust
// Bad — may produce invalid &mut to unaligned/uninit memory
let p: *mut u32 = &mut packed.field as *mut _;

// Good
let p: *mut u32 = &raw mut packed.field;
```

### [L-UNSAFE-008] `MaybeUninit` is the only way to hold uninit data of a non-union type

Do NOT write `mem::uninitialized()` (removed) or `mem::zeroed()` for types whose all-zero bit pattern is not a valid value (e.g., `NonZero`, `&T`, `bool` in some codepaths).

```rust
// Good — array of uninit, initialize in place
let mut buf: [MaybeUninit<u8>; 1024] = [MaybeUninit::uninit(); 1024];
for i in 0..1024 { buf[i].write(0); }
// SAFETY: every element was written above
let buf: [u8; 1024] = unsafe { std::mem::transmute(buf) };
```

---

## 2.9 Special Types and Traits

### [L-SPECIAL-001] `Sized`, `?Sized`, DSTs

- `Sized` is an implicit bound on every generic parameter; opt out with `T: ?Sized`.
- DST types (`str`, `[T]`, `dyn Trait`) can only appear behind a pointer (`&`, `&mut`, `Box`, `Arc`, `Rc`, `Pin<P>`).
- You cannot put a DST directly in a local variable.
- The last field of a struct may be DST, making the struct itself DST (e.g., `OsStr`).

### [L-SPECIAL-002] `Send` and `Sync`

- `Send`: OK to move across threads. Most types are `Send`; `Rc<T>`, `*const T`, `*mut T`, `UnsafeCell<T>` (via `!Sync`) are not.
- `Sync`: `&T` is `Send` iff `T: Sync`. So `Sync` means "safe to share across threads".
- Both are auto traits: implemented automatically by structural rules.
- To opt out: contain a `!Send`/`!Sync` field (e.g., `PhantomData<*const ()>` for `!Send + !Sync`, `PhantomData<Cell<()>>` for `!Sync`).

```rust
// Good — force !Send + !Sync
pub struct ThreadLocal {
    _not_sync_send: PhantomData<*const ()>,
    // … your data …
}
```

### [L-SPECIAL-003] `Copy` and `Drop` are mutually exclusive

A type implementing `Drop` cannot be `Copy`. `Copy` implies no destructor.

### [L-SPECIAL-004] `Pin<P>` semantics

- `Pin<&mut T>` / `Pin<Box<T>>`: guarantees `T` will not be moved after pinning, until dropped.
- Required for self-referential futures and intrusive data structures.
- `T: Unpin` lifts the restriction (most types are `Unpin`).
- `!Unpin` types: contain `PhantomPinned` or wrap in `pin-project` macros.

Rule: if you write a `Future` by hand or intrusive list, opt out of `Unpin` explicitly:

```rust
use std::marker::PhantomPinned;
pub struct MyFut {
    state: State,
    _pin: PhantomPinned,
}
```

Use `pin-project-lite` to safely project fields of a pinned struct.

### [L-SPECIAL-005] `'static` does not mean "forever"

`T: 'static` means "contains no non-`'static` references" — the type itself may still be dropped. Do not confuse with `&'static T` (a reference with static lifetime).

### [L-SPECIAL-006] `?Sized` for flexibility in traits

```rust
// Bad — T: Sized required, can't use with dyn Trait or str
pub trait Process<T> { fn run(&self, t: &T); }

// Good — works with DSTs
pub trait Process<T: ?Sized> { fn run(&self, t: &T); }
```

### [L-SPECIAL-007] Integer overflow is NOT undefined

Overflow in debug mode panics; in release mode wraps (two's complement). Both are defined. For explicit semantics use `wrapping_add`, `checked_add`, `saturating_add`, `overflowing_add`, or `Wrapping<T>`/`Saturating<T>` newtypes.

---

## 2.10 Expressions and Operators

### [L-EXPR-001] Place expressions vs value expressions

A *place expression* refers to a memory location (can be borrowed, assigned). A *value expression* produces a value (temporaries). Mismatches generate temporaries.

- `x` (local): place.
- `*p`: place if `p` is a pointer/reference.
- `a.b`: place iff `a` is a place.
- `f()`: value.
- `arr[i]`: place iff `arr` is a place.

You cannot borrow a value expression except via lifetime extension (`[L-DROP-004]`).

### [L-EXPR-002] Assignment is a statement, not an expression

`(a = b)` has type `()`. `let x = (y = 5);` makes `x: ()`. Use compound statements:

```rust
// Bad
let _ = (state = State::Ready);

// Good
state = State::Ready;
```

### [L-EXPR-003] Block expressions and tail

A `{ stmts; tail }` block evaluates each statement, then yields the tail. No tail → yields `()`.

```rust
let x = { let y = 5; y * 2 };   // x: 12
```

### [L-EXPR-004] `&raw const` and `&raw mut` do NOT create references

They produce `*const T` / `*mut T` directly — use when a reference would be invalid (unaligned, uninit, aliased).

---

## 2.11 Items and Attributes

### [L-ITEM-001] `const` vs `static`

- `const`: inlined at each use site; no fixed address. Use for compile-time constants.
- `static`: one memory location, address-stable for the lifetime of the program. Use for globals that must be referenced.
- Neither may have a destructor that actually runs in const-eval; `const`s with destructors run the destructor at each use (rare, usually wrong).
- `static mut`: default-deny in Edition 2024; each access requires `unsafe`. Use `LazyLock`/`OnceLock`/`Mutex<T>` instead.

```rust
// Good — runtime-initialized, immutable-after-init
use std::sync::LazyLock;
static CONFIG: LazyLock<Config> = LazyLock::new(|| Config::load().unwrap());

// Good — manually initialized once
use std::sync::OnceLock;
static METRICS: OnceLock<Metrics> = OnceLock::new();
fn init_metrics(m: Metrics) { METRICS.set(m).ok(); }
```

### [L-ITEM-002] `#[non_exhaustive]` effects

- On a struct: cannot be instantiated with a record-literal, cannot be destructured exhaustively, outside the defining crate.
- On an enum: outside code must use `_` wildcard.
- On a variant: outside code cannot destructure it without `..`.
- No effect inside the defining crate.

Use on public enums/structs that may grow.

### [L-ITEM-003] `#[must_use]`

Apply on types and functions whose return value the caller should not silently discard.

```rust
#[must_use = "this `Result` may be an `Err`; handle it"]
pub enum Outcome { Ok, Err(Error) }

#[must_use]
pub fn pending_guard() -> Guard { /* … */ }
```

### [L-ITEM-004] `#[inline]` discipline

- `#[inline]`: hint; compiler may inline across crates only if attribute present.
- `#[inline(always)]`: force inline; use only for tiny hot functions.
- `#[inline(never)]`: prevent inlining; useful for error paths, `#[cold]`.
- Do NOT annotate everything — over-inlining increases compile time and binary size.

### [L-ITEM-005] `#[derive(...)]` order-independent, but convention:

`#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]` — common → specific.

---

## 2.12 Macros

### [L-MACRO-001] Prefer `macro_rules!` for pattern-directed rewriting; proc-macros for type-directed code gen

- `macro_rules!`: hygiene-respecting, fast, local. Use for DSLs, small patterns (`println!`, `matches!`).
- Proc-macros (`#[derive]`, attribute, function-like): heavier, require separate crate. Use for serde-style derives, schema generation.

### [L-MACRO-002] Macro hygiene

`macro_rules!` hygiene is per-identifier-kind: it prevents name capture for local bindings but not for items. When writing a macro that introduces a local variable, use a randomized name or document it.

### [L-MACRO-003] `$crate` for cross-crate macros

Inside a `macro_rules!` exported from a crate, refer to items in that crate via `$crate::path::item`. Never hardcode the crate name.

---

[← Meta-Principles](./01-meta-principles.md) | [README](./README.md) | [Next: Idioms →](./03-idioms.md)
