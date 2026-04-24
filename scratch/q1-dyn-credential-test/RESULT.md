# Q1 compile test result

**Date:** 2026-04-24
**rustc:** 1.95.0 (59807616e 2026-04-14)
**Command:** `rustc --edition=2021 test.rs`

## Claim being tested

Rust-senior Q1 prediction (`docs/superpowers/drafts/2026-04-24-credential-redesign/05-known-gaps.md` finding #32 + extended analysis): `CredentialRef<dyn BitbucketCredential>` as proposed Pattern 2 (default) в drafts — **does NOT compile usefully** because `Credential` имеет 4 associated types (`Input`, `State`, `Pending`, `Scheme`), and trait objects require all assoc types to be named, not just the `Scheme` projection bound.

## Result: CLAIM CONFIRMED

```
error[E0191]: the value of the associated types `Input`, `Pending` and `State`
              in `Credential` must be specified
  --> test.rs:37:28
   |
21 |     type Input;
   |     ---------- `Input` defined here
22 |     type State: CredentialState;
   |     --------------------------- `State` defined here
23 |     type Pending: PendingState;
   |     -------------------------- `Pending` defined here
...
37 | fn accepts_dyn_raw(_: &dyn BitbucketCredential) {}
   |                            ^^^^^^^^^^^^^^^^^^^
   |
help: specify the associated types
   |
37 | fn accepts_dyn_raw(_: &dyn BitbucketCredential<Input = /* Type */,
                                                    State = /* Type */,
                                                    Pending = /* Type */>) {}
   |
error: aborting due to 1 previous error
```

## Implications

1. **Pattern 2 (service trait default) does not compile as drafted.** Rust-senior prediction validated by rustc itself.

2. **Fully-named version `dyn BitbucketCredential<Input=_, State=_, Pending=_>`** compiles but defeats Pattern 2's purpose: the whole point was "accept any Bitbucket credential regardless of Input/State shape". Naming 3 assoc types means action carries 3 phantom generics through every call — catastrophic ergonomics (dx-tester blocker #1).

3. **Alternative paths rust-senior proposed:**
   - Split `Credential` (typed, 4 assoc types, for implementors) + `DynCredential` (erased, no assoc types, exposes `as_injector()`). Pattern matches `std::error::Error` → `Box<dyn Error>`. Service traits become subtraits of `DynCredential`.
   - Loses some compile-time Scheme checking but unblocks Pattern 2.

4. **Decision impact:** tech-lead's Path C (defer redesign, promote only #17) is supported by this compile evidence. The proposed redesign in drafts **requires structural rework** before it can compile usefully — which is 6-10 weeks of work for architecturally-disputed value per tech-lead.

## Interpretation

This result **does not** refute the idea of eventually redesigning credential traits. It **does** refute the specific shape proposed in `01-type-system-draft.md` and confirms that writing spec based on those drafts would force into the `DynCredential` split (or similar structural change), multiplying scope.

The test was 20 lines. Result took ~5 seconds to produce. Shaped a decision that avoids 6-10 weeks of redesign churn.

## Companion files

- `test.rs` — the reproduction
- Linked from `docs/superpowers/archive/2026-04-24-credential-redesign-exploratory/STATUS.md`
