// Q1 tactical compile test — rust-senior predicted E0191 on dyn service trait
// projection через Credential с 4 associated types.
//
// Claim: `CredentialRef<dyn BitbucketCredential>` (Pattern 2 default в drafts)
// does NOT compile usefully because dyn object требует naming всех assoc types,
// not just the projected Scheme.
//
// Если error fires — archiving drafts with confidence (Pattern 2 unsupported).
// Если компилится — reassess drafts; rust-senior prediction wrong.
//
// Date: 2026-04-24. Single-file test, rustc 1.95.0.

// ── Minimal reproduction of nebula-credential contract shape ──

trait AuthScheme {}
trait CredentialState {}
trait PendingState {}

// Credential trait — 4 assoc types mirroring actual crate
trait Credential {
    type Input;
    type State: CredentialState;
    type Pending: PendingState;
    type Scheme: AuthScheme;
}

// Concrete shared scheme type (projection target)
struct BearerScheme;
impl AuthScheme for BearerScheme {}

// Service trait that constrains only one of 4 assoc types (rust-senior's exact form)
trait BitbucketCredential: Credential<Scheme = BearerScheme> {}

// ── THE TEST ──

// Attempt 1: raw dyn — should fail per rust-senior E0191 prediction
fn accepts_dyn_raw(_: &dyn BitbucketCredential) {}

// Attempt 2: name one more assoc type — still fails (Input, State not named)
// fn accepts_dyn_partial<I, S: CredentialState, P: PendingState>(
//     _: &dyn BitbucketCredential<Input = I, State = S, Pending = P>
// ) {}

// Attempt 3: fully-named dyn — the shape Pattern 2 would have to use
fn accepts_dyn_fully_named<I, S: CredentialState, P: PendingState>(
    _: &dyn BitbucketCredential<Input = I, State = S, Pending = P>
) {}

fn main() {
    // If `accepts_dyn_raw` compiles, Pattern 2 works как в drafts.
    // If `accepts_dyn_fully_named` is required, Pattern 2 does not deliver
    // "multiple impls with different State" — defeats its purpose.
    println!("If this line prints, the test compiled.");
}
