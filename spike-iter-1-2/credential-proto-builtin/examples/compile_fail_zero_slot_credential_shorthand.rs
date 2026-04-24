//! Q2 — 0-slot action's `ctx.credential()` shorthand must FAIL TO COMPILE.
//!
//! The action declares no `CredentialRef` fields, so the macro does NOT emit
//! `SingleCredentialAction for ZeroSlotAction`. Calling `.credential()` on
//! the context fails with E0599 method-not-found (or trait-not-implemented).
//!
//! Expected: error[E0599] no method named `credential` found for struct
//! `ActionContext<…, ZeroSlotAction>` ... (mentions SingleCredentialAction
//! bound).

use credential_proto::{Credential, CredentialRegistry};

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

struct ZeroSlotAction;
// No SingleCredentialAction impl — macro refuses to emit it for 0-slot.

fn main() {
    let reg = CredentialRegistry::new();
    let action = ZeroSlotAction;
    let ctx = ActionContext { registry: &reg, action: &action };

    // MUST FAIL: no shorthand for 0-slot actions.
    let _ = ctx.credential();
}
