//! H3 — typed accessor method.
//!
//! Per Strategy §3.4 H3: the `#[action]` macro emits a direct typed
//! accessor method per credential field:
//!   fn bb(&self, reg: &CredentialRegistry) -> Option<&BitbucketOAuth2>
//! The resolve call itself is still a registry hit, but the `C` is known
//! at the call site — the compiler can inline the entire chain. No shared
//! dispatch table, no fn-pointer indirection.
//!
//! Expectation: H3 ≈ H1 if the compiler inlines `resolve_concrete` to the
//! same HashMap::get + downcast_ref sequence as the baseline. In practice
//! H3 should be indistinguishable from baseline under release builds —
//! the whole point of H3 is "zero overhead vs a hand-written hashmap
//! resolve."

use std::hint::black_box;

use credential_proto::{CredentialKey, CredentialRegistry};
use credential_proto_builtin::BitbucketOAuth2;
use criterion::{Criterion, criterion_group, criterion_main};

// Hand-expanded action with typed accessor method — what `#[action]` macro
// would emit.
struct BitbucketAction {
    bb_key: CredentialKey,
}

impl BitbucketAction {
    #[inline]
    fn bb<'r>(&self, reg: &'r CredentialRegistry) -> Option<&'r BitbucketOAuth2> {
        reg.resolve_concrete::<BitbucketOAuth2>(self.bb_key.as_str())
    }
}

fn make_registry() -> CredentialRegistry {
    let mut reg = CredentialRegistry::new();
    for i in 0..64 {
        let k = CredentialKey::new(format!("slot_{i}").as_str());
        reg.insert(k, BitbucketOAuth2);
    }
    reg
}

fn bench_h3(c: &mut Criterion) {
    let reg = make_registry();
    let action = BitbucketAction {
        bb_key: CredentialKey::new("slot_42"),
    };

    c.bench_function("h3/typed_accessor", |b| {
        b.iter(|| {
            let v = black_box(&action).bb(black_box(&reg));
            black_box(v)
        });
    });
}

criterion_group!(benches, bench_h3);
criterion_main!(benches);
