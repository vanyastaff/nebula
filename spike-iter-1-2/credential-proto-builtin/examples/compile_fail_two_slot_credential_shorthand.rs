//! Q2 — 2-slot action's `ctx.credential()` shorthand must FAIL TO COMPILE.
//!
//! Slot count is 2 (a, b) — the macro REFUSES to emit
//! `SingleCredentialAction for TwoSlotAction`, so the `.credential()`
//! shorthand is not callable. User must call `action.registry_lookup::<C>
//! (reg, "a")` or `("b")` to disambiguate.
//!
//! Expected: error[E0599] no method named `credential` found ... or trait-
//! bound-not-satisfied for SingleCredentialAction.

use credential_proto::{Credential, CredentialKey, CredentialRef, CredentialRegistry};
use credential_proto_builtin::BitbucketBearerPhantom;

struct ActionContext<'r, A> {
    registry: &'r CredentialRegistry,
    action: &'r A,
}

trait SingleCredentialAction {
    type Cred: Credential;
    fn slot_key(&self) -> &str;
}

impl<'r, A: SingleCredentialAction> ActionContext<'r, A> {
    fn credential(&self) -> Option<&A::Cred> {
        self.registry.resolve_concrete::<A::Cred>(self.action.slot_key())
    }
}

struct TwoSlotAction {
    #[allow(dead_code)]
    a: CredentialRef<dyn BitbucketBearerPhantom>,
    #[allow(dead_code)]
    b: CredentialRef<dyn BitbucketBearerPhantom>,
}
// No SingleCredentialAction impl — macro refuses to emit for slot count 2.

fn main() {
    let reg = CredentialRegistry::new();
    let action = TwoSlotAction {
        a: CredentialRef::new(CredentialKey::new("a")),
        b: CredentialRef::new(CredentialKey::new("b")),
    };
    let ctx = ActionContext { registry: &reg, action: &action };

    // MUST FAIL: ambiguous slot, no shorthand.
    let _ = ctx.credential();
}
