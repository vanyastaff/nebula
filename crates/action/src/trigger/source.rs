//! [`TriggerSource`] — typed envelope marker for trigger event families.
//!
//! Per Tech Spec §2.2.3 line 230 — `TriggerAction` has an associated type
//! `Source: TriggerSource`, and the typed event a trigger handler receives
//! is `<Self::Source as TriggerSource>::Event`. This replaces the
//! transport-erased `TriggerEvent` envelope at the user-facing trait
//! level (the dyn-layer envelope still exists for engine routing —
//! see [`crate::trigger::TriggerEvent`]).
//!
//! ## Why the shape is `Source: TriggerSource` and not `type Event` directly
//!
//! Per spike Iter-2 §2.2
//! (`docs/superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md`) the indirection
//! lets a trigger family carry transport-specific invariants on the `Source` type (e.g.,
//! `WebhookSource` documents `WebhookRequest` body-size caps, `PollSource` documents cursor
//! invariants) without leaking into the base trait.

/// Marker trait identifying a trigger event family.
///
/// Implementors are zero-sized types per spec §2.2.3 — they exist only
/// to tie the family's typed event to [`TriggerAction`](crate::TriggerAction)
/// via the associated type.
pub trait TriggerSource: Send + Sync + 'static {
    /// Concrete event type this source delivers to the trigger handler.
    ///
    /// `Send + Sync + 'static` matches the dyn boundary's
    /// [`TriggerEvent`](crate::trigger::TriggerEvent) payload requirement
    /// (`Box<dyn Any + Send + Sync>` + `downcast::<T>` requiring
    /// `T: Any + Send + Sync + 'static`). Tightening the bound here means
    /// downstream registries / adapters never need to repeat it.
    ///
    /// Examples: `WebhookSource::Event = WebhookRequest`,
    /// `PollSource::Event = ()` (poll triggers self-drive their cycle and
    /// never receive pushed events — see [`crate::poll::PollSource`]).
    type Event: Send + Sync + 'static;
}
