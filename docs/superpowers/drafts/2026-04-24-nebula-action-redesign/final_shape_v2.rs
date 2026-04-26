//! Final shape v2 — distilled from spike commit `c8aef6a0` for Tech Spec §7.
//!
//! This file is **NOT compile-checked in isolation**; it is a curated
//! extract from the spike crate at
//! `scratch/spike-action-credential/src/`. It compiles end-to-end as
//! part of that crate. The commentary is what Tech Spec §7 should
//! cite + the shape it should freeze.
//!
//! See `07-spike-NOTES.md` for full spike context.

// ============================================================
// 1. CredentialRef<C> — Credential Tech Spec §3.5 typed handle
// ============================================================

use std::marker::PhantomData;

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct CredentialRef<C: ?Sized> {
    pub key: CredentialKey,
    _t: PhantomData<fn() -> C>,
}

impl<C: ?Sized> CredentialRef<C> {
    pub fn from_key(key: CredentialKey) -> Self {
        Self { key, _t: PhantomData }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CredentialKey(pub std::sync::Arc<str>);

// ============================================================
// 2. SlotBinding — Credential Tech Spec §3.4 line 869 verbatim
// ============================================================

use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn core::future::Future<Output = T> + Send + 'a>>;

/// HRTB fn-pointer — load-bearing shape per §3.4 line 869.
/// Single 'ctx lifetime per 02c §6 modernization.
/// Cannot be `async fn` pointer (no such syntax on 1.95).
pub type ResolveFn = for<'ctx> fn(
    ctx: &'ctx CredentialContext<'ctx>,
    key: &'ctx SlotKey,
) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>;

/// Macro-emitted as `&'static [SlotBinding]` per action.
/// Must be `Copy + 'static` for static-slice storage.
#[derive(Clone, Copy, Debug)]
pub struct SlotBinding {
    pub field_name: &'static str,
    pub slot_type: SlotType,
    pub resolve_fn: ResolveFn,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SlotKey {
    pub credential_key: String,
    pub field_name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotType {
    DirectType,
    ServiceCapability { capability: Capability },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Bearer,
    Basic,
    OAuth2,
}

// Placeholders — defined in surrounding crate.
pub struct CredentialContext<'a>(PhantomData<&'a ()>);
pub enum ResolvedSlot {
    Bearer { /* SecretString */ },
    Basic { /* ... */ },
    OAuth2 { /* ... */ },
}
#[derive(Debug)]
pub enum ResolveError {
    NotFound { key: String },
    WrongType { key: String, expected: &'static str },
    StateLoad { key: String, reason: String },
}

// ============================================================
// 3. SchemeGuard<'a, C> — §15.7 line 3394-3429 + iter-3 refinement
// ============================================================

use std::ops::Deref;
use zeroize::Zeroize;

pub trait Credential: Send + Sync + 'static {
    type State: Send + Sync + 'static;
    type Scheme: Send + Sync + 'static;
    const KEY: &'static str;
    const DYNAMIC: bool = false;
    const LEASE_TTL: Option<std::time::Duration> = None;
    fn project(state: &Self::State) -> Self::Scheme;
}

/// !Clone, ZeroizeOnDrop, Deref<Target = Scheme>.
/// Lifetime 'a is pinned by `engine_construct(scheme, &'a ctx)` —
/// see iter-3 refinement (§15.7 line 3503-3516). PhantomData<&'a ()>
/// alone does NOT prevent retention; the construction signature does.
pub struct SchemeGuard<'a, C: Credential>
where
    C::Scheme: Zeroize,
{
    scheme: C::Scheme,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a, C: Credential> SchemeGuard<'a, C>
where
    C::Scheme: Zeroize,
{
    /// Engine-only constructor. The `&'a CredentialContext<'a>` borrow
    /// pins `'a` to a real, non-static lifetime. Without this borrow
    /// at construction, callers could store `SchemeGuard<'static, C>`
    /// in a struct field, defeating the lifetime-parameter intent.
    pub(crate) fn engine_construct(
        scheme: C::Scheme,
        _pin: &'a CredentialContext<'a>,
    ) -> Self {
        Self { scheme, _lifetime: PhantomData }
    }
}

impl<'a, C: Credential> Deref for SchemeGuard<'a, C>
where
    C::Scheme: Zeroize,
{
    type Target = C::Scheme;
    fn deref(&self) -> &Self::Target {
        &self.scheme
    }
}

impl<'a, C: Credential> Drop for SchemeGuard<'a, C>
where
    C::Scheme: Zeroize,
{
    fn drop(&mut self) {
        self.scheme.zeroize();
    }
}

// IMPORTANT: NO Clone, NO Copy. Verified by the **qualified-form**
// compile-fail probe (`<SchemeGuard as Clone>::clone(&guard)`). The
// unqualified `guard.clone()` would silently call `Scheme::clone`
// via auto-deref — see spike NOTES.md finding #1.

// ============================================================
// 4. SchemeFactory<C> — §15.7 line 3438-3447
// ============================================================

use std::sync::Arc;

#[derive(Debug)]
pub enum AcquireError {
    ResolveFailed(String),
    RefreshExhausted,
}

pub struct SchemeFactory<C: Credential>
where
    C::Scheme: Zeroize,
{
    inner: Arc<dyn Fn() -> BoxFuture<'static, Result<C::Scheme, AcquireError>> + Send + Sync>,
}

impl<C: Credential> Clone for SchemeFactory<C>
where
    C::Scheme: Zeroize,
{
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<C: Credential> SchemeFactory<C>
where
    C::Scheme: Zeroize,
{
    pub async fn acquire<'a>(
        &'a self,
        ctx: &'a CredentialContext<'a>,
    ) -> Result<SchemeGuard<'a, C>, AcquireError> {
        let scheme = (self.inner)().await?;
        Ok(SchemeGuard::engine_construct(scheme, ctx))
    }
}

// ============================================================
// 5. Action trait family — RPITIT typed surface
// ============================================================

use std::future::Future;

pub struct ActionContext<'a> {
    pub creds: &'a CredentialContext<'a>,
}

pub trait StatelessAction: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}

pub trait StatefulAction: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + 'static;
    type State: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        state: &'a mut Self::State,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}

pub trait Resource: Send + Sync + 'static {
    type Credential: Credential;
}

pub trait ResourceAction: Send + Sync + 'static {
    type Resource: Resource;  // <- required; Probe 1 verifies
    type Input: Send + 'static;
    type Output: Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        resource: &'a Self::Resource,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}

pub trait TriggerSource: Send + Sync + 'static {
    type Event: Send + 'static;
}

pub trait TriggerAction: Send + Sync + 'static {
    type Source: TriggerSource;  // <- required; Probe 2 verifies
    type Error: std::error::Error + Send + Sync + 'static;
    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: <Self::Source as TriggerSource>::Event,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}

// ============================================================
// 6. ActionSlots — macro-emitted-only trait
// ============================================================

/// Action authors do NOT implement `ActionSlots` by hand. The
/// `#[action]` macro emits this from the `credentials(slot: Type)`
/// zone. A struct with a bare `CredentialRef<_>` field outside the
/// zone has no `ActionSlots` impl, cannot satisfy the `Action`
/// blanket marker, cannot be registered in the engine — Probe 3.
///
/// Production proc-macro should ALSO emit a `compile_error!` when
/// it sees a `CredentialRef<_>` field outside the `credentials(...)`
/// zone — DX layer, complementing the type-system layer.
pub trait ActionSlots {
    fn credential_slots(&self) -> &'static [SlotBinding];
}

pub trait Action: ActionSlots {}
impl<T: StatelessAction + ActionSlots> Action for T {}
// Add similar blanket impls for StatefulAction / ResourceAction /
// TriggerAction in production code.
