//! H1 — PhantomData + TypeId registry (iter-2 indexed form).
//!
//! Exercises `CredentialRegistry::resolve_concrete::<C>(&str) -> Option<&C>`.
//! Delta vs baseline measures the cost of: (a) going through the
//! `Box<dyn AnyCredential>` wrapper (an extra vtable-backed trait level
//! over `Box<dyn Any + Send + Sync>` the baseline uses), (b) the `as_any`
//! method call, (c) downcast_ref on the widened trait object.

use std::hint::black_box;

use credential_proto::{CredentialKey, CredentialRegistry};
use credential_proto_builtin::BitbucketOAuth2;
use criterion::{Criterion, criterion_group, criterion_main};

fn make_registry() -> CredentialRegistry {
    let mut reg = CredentialRegistry::new();
    for i in 0..64 {
        let k = CredentialKey::new(format!("slot_{i}").as_str());
        reg.insert(k, BitbucketOAuth2);
    }
    reg
}

fn bench_h1(c: &mut Criterion) {
    let reg = make_registry();
    let key = CredentialKey::new("slot_42");
    let key_s = key.as_str();

    c.bench_function("h1/resolve_concrete", |b| {
        b.iter(|| {
            let v = reg.resolve_concrete::<BitbucketOAuth2>(black_box(key_s));
            black_box(v)
        });
    });
}

criterion_group!(benches, bench_h1);
criterion_main!(benches);
