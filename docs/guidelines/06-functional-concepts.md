# 6. Functional Concepts

[← Anti-Patterns](./05-anti-patterns.md) | [README](./README.md) | [Next: Modern Rust →](./07-modern-rust.md)

---

## [F-001] Generics as type classes (typestate)

`Vec<u8>` and `Vec<char>` are *different types*. Partial `impl` blocks specialize API per state:

```rust
use std::path::{Path, PathBuf};

mod proto {
    use std::path::PathBuf;

    #[derive(Clone)]
    pub struct NfsAuth;
    pub struct BootpAuth;

    pub trait ProtoKind {
        type Auth;
        fn auth(&self) -> Self::Auth;
    }
    pub struct Nfs {
        auth: NfsAuth,
        mount: PathBuf, // private — outside code cannot construct `Nfs` with arbitrary state
    }
    pub struct Bootp;
    impl Nfs {
        pub fn new(auth: NfsAuth, mount: PathBuf) -> Self {
            Self { auth, mount }
        }
    }
    impl ProtoKind for Nfs {
        type Auth = NfsAuth;
        fn auth(&self) -> NfsAuth {
            self.auth.clone()
        }
    }
    impl ProtoKind for Bootp {
        type Auth = BootpAuth;
        fn auth(&self) -> BootpAuth {
            BootpAuth
        }
    }
}

pub struct Request<P: proto::ProtoKind> {
    file: PathBuf,
    proto: P,
}
impl<P: proto::ProtoKind> Request<P> {
    pub fn new(file: PathBuf, proto: P) -> Self {
        Self { file, proto }
    }
    pub fn auth(&self) -> P::Auth {
        self.proto.auth()
    }
}
impl Request<proto::Nfs> {
    pub fn mount_point(&self) -> &Path {
        &self.proto.mount
    } // only on `Nfs`
}
```

`Request<Bootp>::mount_point()` is a compile error. State encoded in types.

Used throughout the ecosystem: typestate builders, embedded-hal GPIO pin modes, `hyper::Client<Connector>`, `sqlx::Executor`, session types.

Cost: monomorphization grows binary. Mitigate with `-Z share-generics` (nightly) or factor shared code into non-generic functions.

## [F-002] Iterator chains vs `for` loops

Rule of thumb:
- Single-value reduction: `iter().sum()`, `.product()`, `.fold(init, f)`.
- Transform: `.map(…).collect()`.
- Filter: `.filter(…)`, `.filter_map(…)`.
- Side effects: `for` (explicit, readable).
- Complex per-element state machine: `for` or `try_fold`.

```rust
// Good — map is the right tool
let sums: Vec<u64> = xs.iter().map(|x| x.total).collect();

// Acceptable — side effects belong in for
for msg in queue.drain(..) { handler.dispatch(msg)?; }

// Bad — overusing fold for iteration with side effects
queue.drain(..).try_fold((), |_, m| handler.dispatch(m))?;
```

## [F-003] Optics (conceptual)

Serde's design embodies optics: `Serialize`/`Deserialize` (Poly Iso on value ↔ data model) × `Serializer`/`Deserializer` (Poly Iso on format ↔ data model), linked through `Visitor`. Result: N types × M formats without N×M code.

You rarely write optics directly. Understanding the shape helps with: binary protocol crates, query translators, transducers, algebraic effect systems. For occasional nested-field access, prefer `Option::as_deref_mut`, pattern matching, or the `lens-rs` crate for serious use.

## [F-004] `impl Trait` as an existential type

- Argument position: "any T satisfying Trait" — generic, static dispatch.
- Return position (RPIT / RPITIT): "some specific T you can't name" — opaque.
- Let position: type inference.

```rust
// Good — opaque return, no boxing
fn parser() -> impl Fn(&str) -> Option<Token> { |s| /* … */ }

// Good — argument bound
fn process(items: impl Iterator<Item = Item>) { /* … */ }
```

Caveat: RPIT captures all input generic parameters and lifetimes by default. Use `use<'a, T>` bound (1.82+) to restrict:

```rust
fn filter<'a, T>(xs: &'a [T]) -> impl Iterator<Item = &'a T> + use<'a, T> {
    xs.iter().filter(|_| true)
}
```

---

[← Anti-Patterns](./05-anti-patterns.md) | [README](./README.md) | [Next: Modern Rust →](./07-modern-rust.md)
