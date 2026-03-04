# Decisions

## D-001: Plugin Trait Is Object-Safe

**Status:** Adopt

**Context:** Registry and loader must hold heterogeneous plugin types.

**Decision:** Plugin is object-safe (metadata, register); plugins stored as Arc<dyn Plugin>.

**Alternatives considered:** Only static plugin types — would block dynamic loading and generic registry.

**Trade-offs:** Dynamic dispatch cost; enables registry and loader to work with any Plugin impl.

**Consequences:** Adding non-object-safe methods to Plugin would be breaking (major).

**Migration impact:** None.

**Validation plan:** Registry and loader tests with multiple Plugin impls.

---

## D-002: Registry Does Not Own Thread-Safety

**Status:** Adopt

**Context:** Engine/API may share registry across threads.

**Decision:** PluginRegistry is not Sync/Send by default; caller wraps in RwLock or uses in single-threaded context.

**Alternatives considered:** Built-in Mutex/RwLock — rejected to avoid forcing sync dependency and to let caller choose locking.

**Trade-offs:** Caller must remember to wrap; doc and examples show RwLock pattern.

**Consequences:** Registry API is &mut self for register; get/list may be &self if we split or use interior mutability later.

**Migration impact:** None.

**Validation plan:** Doc and example with RwLock<PluginRegistry>.

---

## D-003: Dynamic Loading Behind Feature

**Status:** Adopt

**Context:** Unsafe FFI and libloading only needed for dynamic load.

**Decision:** PluginLoader and FFI behind `dynamic-loading` feature; default build has no unsafe.

**Alternatives considered:** Always-on dynamic loading — rejected for security and portability.

**Trade-offs:** Users who want dynamic load enable feature; default users get no unsafe.

**Consequences:** Document ABI and symbol stability for dynamic plugins; loader module allow(unsafe_code).

**Migration impact:** None.

**Validation plan:** CI with and without feature; no unsafe in default build (cargo build without default-features where applicable).
