//! Q2 — `#[action]` macro ambiguity. Hand-expanded for 0/1/2+ credential
//! slot counts; demonstrates that `ctx.credential::<C>()` shorthand:
//!   - works for 1-slot actions
//!   - is a COMPILE error for 0-slot and 2+-slot actions
//!
//! The mechanism: macro emits `impl SingleCredentialAction for MyAction`
//! ONLY when slot count == 1. The shorthand calls a method on this trait.
//! No impl ⇒ no method ⇒ E0599 method-not-found at the call site.
//!
//! For 2+ slots, the user must use `credential_named::<C>("field_name")`
//! which is on a different trait (`MultiCredentialAction`) emitted for
//! actions with ≥1 slot. Both 1-slot and N-slot actions get the named
//! accessor; only 1-slot gets the `.credential()` shorthand.

use credential_proto::{Credential, CredentialKey, CredentialRegistry, CredentialRef};
use credential_proto_builtin::{
    BearerScheme, BitbucketBearerPhantom, BitbucketOAuth2, BitbucketPat, OAuth2State,
};

// ─── Hand-expanded ActionContext shape ────────────────────────────────────────

struct ActionContext<'r, A> {
    registry: &'r CredentialRegistry,
    action: &'r A,
}

// ─── Trait emitted ONLY for 1-slot actions ────────────────────────────────────

trait SingleCredentialAction {
    type Cred: Credential;
    fn slot_key(&self) -> &str;
}

impl<'r, A: SingleCredentialAction> ActionContext<'r, A> {
    /// Shorthand. Available iff `A: SingleCredentialAction`.
    /// For 0-slot actions: NO impl exists ⇒ method missing ⇒ compile error.
    /// For 2+-slot actions: NO impl exists (macro refuses to emit it for
    ///   ambiguous slot count) ⇒ method missing ⇒ compile error.
    fn credential(&self) -> Option<&A::Cred> {
        self.registry.resolve_concrete::<A::Cred>(self.action.slot_key())
    }
}

// ─── Trait for any-slot-count actions (the always-safe accessor) ──────────────

trait NamedCredentialAccess {
    fn registry_lookup<'r, C: Credential>(
        &self,
        reg: &'r CredentialRegistry,
        slot: &str,
    ) -> Option<&'r C>;
}

// Default impl works for any action — looks up the registry by user-named slot.
impl<A> NamedCredentialAccess for A {
    fn registry_lookup<'r, C: Credential>(
        &self,
        reg: &'r CredentialRegistry,
        slot: &str,
    ) -> Option<&'r C> {
        reg.resolve_concrete::<C>(slot)
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 0-slot action — no credentials at all.
// Macro emits NO impl for SingleCredentialAction; ctx.credential() unavailable.
// ═════════════════════════════════════════════════════════════════════════════

struct ZeroSlotAction;
// (No SingleCredentialAction impl emitted by macro.)

#[test]
fn q2_zero_slot_action_compiles_but_has_no_shorthand() {
    let reg = CredentialRegistry::new();
    let action = ZeroSlotAction;
    let _ctx = ActionContext { registry: &reg, action: &action };

    // Compile-fail: ctx.credential() not available because ZeroSlotAction
    // does not impl SingleCredentialAction.
    // (Demonstrated in compile_fail_zero_slot_credential_shorthand.rs)
}

// ═════════════════════════════════════════════════════════════════════════════
// 1-slot action — shorthand IS available.
// ═════════════════════════════════════════════════════════════════════════════

struct OneSlotAction {
    bb_key: CredentialKey,
    #[allow(dead_code)]
    bb: CredentialRef<dyn BitbucketBearerPhantom>,
}

// Macro emits this:
impl SingleCredentialAction for OneSlotAction {
    type Cred = BitbucketOAuth2;
    fn slot_key(&self) -> &str {
        self.bb_key.as_str()
    }
}

#[test]
fn q2_one_slot_action_shorthand_resolves() {
    let mut reg = CredentialRegistry::new();
    let key = CredentialKey::new("bb");
    reg.insert(key.clone(), BitbucketOAuth2);

    let action = OneSlotAction {
        bb_key: key.clone(),
        bb: CredentialRef::new(key),
    };
    let ctx = ActionContext { registry: &reg, action: &action };

    let cred = ctx.credential();
    assert!(cred.is_some(), "1-slot shorthand resolves");
}

// ═════════════════════════════════════════════════════════════════════════════
// 2+-slot action — NO shorthand impl emitted. Named accessor still works.
// ═════════════════════════════════════════════════════════════════════════════

struct TwoSlotAction {
    a_key: CredentialKey,
    b_key: CredentialKey,
    #[allow(dead_code)]
    a: CredentialRef<dyn BitbucketBearerPhantom>,
    #[allow(dead_code)]
    b: CredentialRef<dyn BitbucketBearerPhantom>,
}

// Macro emits NO SingleCredentialAction impl (slot count != 1) — ambiguity
// blocks the shorthand. Macro emits NamedCredentialAccess (default impl, no
// new code needed). Action body uses `action.registry_lookup::<C>(reg,
// "field_name")`.

#[test]
fn q2_two_slot_action_named_accessor_resolves_each() {
    let mut reg = CredentialRegistry::new();
    let key_a = CredentialKey::new("a");
    let key_b = CredentialKey::new("b");
    reg.insert(key_a.clone(), BitbucketOAuth2);
    reg.insert(key_b.clone(), BitbucketPat);

    let action = TwoSlotAction {
        a_key: key_a.clone(),
        b_key: key_b.clone(),
        a: CredentialRef::new(key_a),
        b: CredentialRef::new(key_b),
    };

    let cred_a: Option<&BitbucketOAuth2> =
        action.registry_lookup(&reg, action.a_key.as_str());
    let cred_b: Option<&BitbucketPat> =
        action.registry_lookup(&reg, action.b_key.as_str());

    assert!(cred_a.is_some());
    assert!(cred_b.is_some());

    // Verify the projection still works through the resolved credential.
    let state = OAuth2State {
        access_token: "tok".into(),
        refresh_token: "ref".into(),
    };
    let scheme: BearerScheme = BitbucketOAuth2::project(&state);
    assert_eq!(scheme.token, "tok");
}
