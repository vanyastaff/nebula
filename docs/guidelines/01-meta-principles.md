# 1. Meta-Principles

[← README](./README.md) | [Next: Language Rules →](./02-language-rules.md)

---

## [M-001] Make invalid states unrepresentable

If an invariant can be encoded in a type, encode it. Runtime checks are acceptable only at trust boundaries (FFI input, deserialization, user input, network).

```rust
// Bad — runtime invariant spread across the codebase
struct Connection {
    state: State,
    stream: Option<TcpStream>,   // ⚠ every caller must check
}

// Good — state encoded in the type (typestate)
struct Connection<S> { inner: S }
struct Connected(TcpStream);
struct Disconnected;

impl Connection<Disconnected> {
    fn connect(addr: SocketAddr) -> io::Result<Connection<Connected>> { /* … */ }
}
impl Connection<Connected> {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> { /* … */ }
}
```

## [M-002] Apply YAGNI aggressively

Most GoF patterns collapse in Rust: Strategy → trait + generic (or closure); Observer → channel; Singleton → `OnceLock`/`LazyLock`; Abstract Factory → associated types; Template Method → default trait methods. Do not port Java skeletons.

## [M-003] Minimize API surface

Everything public is a compatibility contract. Default to `pub(crate)` or private; escalate to `pub` deliberately. Use `#[non_exhaustive]` on public enums that may grow.

## [M-004] Composition, not inheritance

Rust has no inheritance. Do not simulate it via `Deref` (see `[A-003]`). Use trait composition, delegation (manual or `delegate`/`ambassador` crate), or embedding.

## [M-005] Explicit > implicit, except at accepted seams

Rust has accepted implicit seams: `?`, `Deref` coercion, auto-ref in method calls, `into()`/`from()`, `Drop`. Use them. Do not invent new ones (e.g., `Deref` on owning types that are not smart pointers).

## [M-006] Prefer zero-cost abstractions

If two designs produce equivalent code under `-Copt-level=3`, prefer the one with a clearer type-level contract. Static dispatch (`impl Trait`, generic parameters) is zero-cost; dynamic dispatch (`dyn Trait`) is not but often worth it for compile times and binary size.

## [M-007] One fact, one place

Constants, configuration, and invariants live in exactly one module. Use re-exports (`pub use`) to expose them elsewhere. Avoid parallel definitions in tests vs production.

---

[← README](./README.md) | [Next: Language Rules →](./02-language-rules.md)
