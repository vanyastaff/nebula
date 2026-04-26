//! Bonus probe (Stage 4) - Pattern 2 wrong-scheme rejection through
//! the phantom-shim chain.
//!
//! Per ADR-0035 1, declaring a credential whose `Scheme` does not
//! satisfy a capability's `scheme_bound` marker must reject at
//! compile time. The diagnostic chain travels:
//!
//! - `BasicScheme: AcceptsBearer` not satisfied (the original constraint), then
//! - `WrongCredential: MyServiceBearer` not satisfied (the real capability blanket fails), then
//! - `WrongCredential: MyServiceBearerPhantom` not satisfied (the phantom blanket re-requires the
//!   real trait).
//!
//! This probe exercises the resolution walk on a self-contained
//! fixture (local `mod sealed_caps`, local `AcceptsBearer` marker,
//! local service supertrait) so it does not depend on any future
//! Stage's marker traits.

#[test]
fn compile_fail_pattern2_service_reject() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/pattern2_service_reject.rs");
}
