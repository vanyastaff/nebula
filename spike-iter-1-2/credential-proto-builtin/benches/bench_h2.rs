//! H2 — hand-expanded proc-macro binding table.
//!
//! Per Strategy §3.4 H2: the `#[action]` macro emits a compile-time const
//! array of `BindingEntry { resolve_fn }`; the action reaches into the
//! table by field-slot index. Dispatch cost: array index + fn-pointer
//! indirect call.
//!
//! This bench models H2 faithfully: the resolve_fn still has to hit the
//! registry (that's where the actual credential lives). Compared to H1,
//! H2 adds the fn-pointer indirection and REMOVES the compile-time
//! `C: Credential` generic parameter resolution at the call site (the
//! monomorphization happens at macro expansion time).
//!
//! Expectation: H2 ≈ H1 + one fn-pointer indirect call. Should be within
//! noise of H1. Modern CPUs branch-predict fn-pointer targets well when
//! the same fn is called repeatedly (which is the hot-path reality — one
//! action invoked many times, same resolve_fn each call).

use std::hint::black_box;

use credential_proto::{AnyCredential, CredentialKey, CredentialRegistry};
use credential_proto_builtin::BitbucketOAuth2;
use criterion::{Criterion, criterion_group, criterion_main};

// Hand-expanded macro output: per-slot resolve fn.
// Real macro would emit one per (action-struct, field-name).
fn resolve_slot_bitbucket_oauth2<'r>(
    reg: &'r CredentialRegistry,
    key: &str,
) -> Option<&'r dyn AnyCredential> {
    // Resolve_any is O(1) indexed in iter-2. The generic `::<BitbucketOAuth2>`
    // is inlined at macro-expansion time, not at call site.
    reg.resolve_any(key).filter(|c| c.type_id_marker() == std::any::TypeId::of::<BitbucketOAuth2>())
}

#[allow(dead_code)]
struct BindingEntry {
    /// Field name on the action struct. Used by error messages + tracing
    /// in the real macro output; unused in this perf harness but kept for
    /// structural fidelity to the hand-expansion.
    field: &'static str,
    resolve_fn: for<'r> fn(&'r CredentialRegistry, &str) -> Option<&'r dyn AnyCredential>,
}

// Hand-expanded action binding table (what macro would emit for one
// `CredentialRef` field).
const BINDINGS: &[BindingEntry] = &[BindingEntry {
    field: "bb",
    resolve_fn: resolve_slot_bitbucket_oauth2,
}];

fn make_registry() -> CredentialRegistry {
    let mut reg = CredentialRegistry::new();
    for i in 0..64 {
        let k = CredentialKey::new(format!("slot_{i}").as_str());
        reg.insert(k, BitbucketOAuth2);
    }
    reg
}

fn bench_h2(c: &mut Criterion) {
    let reg = make_registry();
    let key = CredentialKey::new("slot_42");
    let key_s = key.as_str();

    c.bench_function("h2/binding_table_dispatch", |b| {
        b.iter(|| {
            // At the action body, macro-generated code would index BINDINGS
            // by the compile-time slot index. Here we use [0] directly.
            let entry = &BINDINGS[0];
            let v = (entry.resolve_fn)(&reg, black_box(key_s));
            black_box(v)
        });
    });
}

criterion_group!(benches, bench_h2);
criterion_main!(benches);
