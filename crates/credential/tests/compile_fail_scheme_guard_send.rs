//! SEC-06 (security hardening 2026-04-27 Stage 2) — `SchemeGuard<'a, C>: !Send`.
//!
//! After SEC-06 added `_thread_marker: PhantomData<*const ()>` to the
//! struct, `SchemeGuard` is `!Send + !Sync` regardless of the wrapped
//! scheme's auto-traits. This probe verifies attempting to `tokio::spawn`
//! a future capturing a `SchemeGuard` is rejected with `E0277`
//! («`*const ()` cannot be sent between threads safely»).
//!
//! See `docs/superpowers/specs/2026-04-27-credential-security-hardening-design.md` §4.

#[test]
fn compile_fail_scheme_guard_send() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_guard_send.rs");
}
