//! Probe 4 — SchemeGuard bound to a borrow'd context cannot escape the call.
//!
//! Expected: E0597 — borrowed value does not live long enough.
//!
//! FINDING (CP5 §15.7 refinement): the §15.7 signature
//!   `fn on_credential_refresh<'a>(&self, g: SchemeGuard<'a, C>) -> …`
//! does NOT structurally prevent retention if `'a` is unconstrained. The
//! compiler freely infers `'a = 'static` when no reference pins it down,
//! and the resource can then store the guard in a `'static` field.
//!
//! The retention barrier REQUIRES an engine-side contract that passes the
//! guard ALONGSIDE a short-lived borrow (e.g. `&'a CredentialContext`) with
//! the SAME `'a`. Then `'a` is pinned to the context's lifetime and cannot
//! escape upward. This probe verifies the "bound to borrow" form rejects
//! retention — the form the engine must use.

use credential_proto::{Credential, SchemeGuard};
use credential_proto_builtin::{BearerScheme, OAuth2Credential};

struct CallContext;

struct RogueResource<'long> {
    retained: Option<SchemeGuard<'long, OAuth2Credential>>,
}

impl<'long> RogueResource<'long> {
    /// Mimics the engine-side contract: guard is passed ALONGSIDE a
    /// short-lived `&'a ctx`. `'a` is pinned to `ctx`'s lifetime.
    fn on_refresh<'a>(
        &mut self,
        _ctx: &'a CallContext,
        guard: SchemeGuard<'a, OAuth2Credential>,
    ) where
        'a: 'long, // attempt: widen 'a to 'long. Succeeds iff 'a actually outlives 'long.
    {
        self.retained = Some(guard);
    }
}

fn main() {
    let mut r: RogueResource<'static> = RogueResource { retained: None };
    let ctx_stack = CallContext;
    let ctx_ref = &ctx_stack; // borrow constrained to stack lifetime
    let scheme = BearerScheme { token: "leaked".into() };
    let guard = SchemeGuard::<'_, OAuth2Credential>::new(scheme);
    // 'a must equal ctx_ref's lifetime (stack) AND must equal guard's lifetime
    // AND must outlive 'long = 'static. Impossible; compile error.
    r.on_refresh(ctx_ref, guard);
}
