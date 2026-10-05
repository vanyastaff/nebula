# SDK release quality (NS20)

`scripts/sdk-release-quality.py run` executes the CP3 SDK release contract at
the exact checked-out source revision. It writes command vectors, raw exit
statuses and hashed execution logs into `sdk-release-quality-candidate`.
This is an untrusted candidate, not independently qualified release evidence.

The five required cases are:

| Case | Executed evidence |
| --- | --- |
| `api-snapshots` | Existing `public_api_snapshot` tests for default, minimal, all, and each declared SDK feature |
| `semver-classification` | Pinned cargo-semver-checks for `nebula-sdk` and `nebula-api-contract` against an immutable compatibility baseline |
| `msrv` | Both supported packages checked with pinned stable Rust 1.97.1, all features and targets |
| `rustdoc` | Both supported packages documented with all features and `RUSTDOCFLAGS=-D warnings` |
| `supported-feature-matrices` | SDK all-target checks per actual Cargo feature plus SDK-only perimeter and persona consumer tests |

Feature names come from locked Cargo metadata. Default, minimal, and all-feature
builds do not replace isolated-feature checks. The verifier independently reads
the source revision's SDK manifest, so a candidate cannot omit a declared feature.
An absent `embedded` feature is a CP3 prerequisite failure. An absent transport
package or compatibility baseline is a failure, never a skipped semver pass.

The supplemental SemVer workflow requests `cargo xtask ci-plan semver
--supported-surface`. Cargo metadata still owns diff selection and reverse
closure. This independent compatibility policy then keeps the two supported
boundaries before baseline admission; new or renamed unsupported implementation
packages cannot create false branded SemVer failures. Missing supported baselines
still fail. Generic `ci-plan semver` behavior is unchanged.

## First transport baseline: staged adoption

The pre-extraction main revision contains no `nebula-api-contract`. That absence
is a baseline-unavailable failure, not a compatible SemVer result. The existing
OpenAPI assertions and representative new wire round trips do not establish a
complete before/after protocol equivalence report.

Bootstrap therefore introduces policy scripts, source-profile helpers and tests
without adding NS20 jobs or needs to the existing required `Tests`/`CI` checks.
The extracted contract must first obtain reviewed source, executed extraction
checks and an actual merged main revision. SDK compatibility against its real
pre-extraction baseline remains independently executable; it is never replaced
by comparing the SDK against the newly extracted transport or current source.

Only after that reviewed extraction is on main may the activation change enable
the SDK release producer and protected verifier. Its original event baseline
must actually contain both supported packages; the protected policy bootstrap
must also exist at the OIDC-authenticated immutable policy revision. No fabricated
baseline SHA, current-source self-comparison or empty transport check is accepted.
Missing prerequisites keep the matrix unqualified. There is no authorized merge
or publication implied by preparation of either patch.

Use the existing snapshots extended by issue 1000; do not create another
baseline. Review intentional API changes before updating them:

```text
cargo test -p nebula-sdk --test public_api_snapshot
INSTA_UPDATE=always cargo test -p nebula-sdk --test public_api_snapshot
git diff -- crates/sdk/tests/snapshots
```

The update environment syntax above is for Bash. CI forces `INSTA_UPDATE=no`.
No nightly or direct `RUSTC_BOOTSTRAP` invocation is needed by this gate.
Cargo-semver-checks may manage its own rustdoc extraction on stable.

Protected admission must authenticate the original run, attempt, repository,
source SHA, successful producer job, and GitHub artifact archive digest before
invoking `verify`. The existing CI4 protected reusable verifier is the sole
elevated-permission job. It reads policy from its OIDC-bound immutable protected
workflow revision; candidate manifests and same-run hashes cannot select policy.
The compatibility baseline is supplied independently from the source event or
authenticated source commit parent. Successful qualification retains a signed
archive and effective `partial` NS20 result, never a registry `passed` state.

Local admission tests use synthetic fixtures to probe failed/skipped commands,
feature omission, baseline substitution, changed logs, duplicate JSON keys, and
stale identities. Passing those tests does not prove the SDK compiles or that
the protected workflow has executed. Final release evidence requires all real
feature builds and the protected signed report at the final source SHA.
