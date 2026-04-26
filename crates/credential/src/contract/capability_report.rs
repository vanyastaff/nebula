//! Capability detection at registration.
//!
//! Per Tech Spec §15.8 (closes security-lead N6 — silent capability
//! self-attestation), the credential registry computes its
//! [`Capabilities`] set from per-credential trait constants emitted by
//! `#[derive(Credential)]` rather than reading a builder-attested
//! metadata field. A plugin therefore cannot lie about what it implements
//! — the macro emits one `IsX::VALUE` per sub-trait, and operators see
//! exactly the capability surface the type satisfies.
//!
//! # Bitflag set
//!
//! The five capability flags mirror the sub-trait surface introduced
//! in Stage 3 (Tech Spec §15.4):
//!
//! | Flag | Sub-trait |
//! |------|-----------|
//! | `INTERACTIVE` | `Interactive` |
//! | `REFRESHABLE` | `Refreshable` |
//! | `REVOCABLE`   | `Revocable` |
//! | `TESTABLE`    | `Testable` |
//! | `DYNAMIC`     | `Dynamic` |
//!
//! # Detection mechanics
//!
//! `compute_capabilities::<C>()` is invoked once per type at registration
//! (`CredentialRegistry::register`). It requires `C` to implement all
//! five [`plugin_capability_report`] traits — each carries a `const
//! VALUE: bool` whose value is `true` when the corresponding sub-trait is
//! impl'd by the credential and `false` otherwise. The macro emits ALL
//! five `IsX` impls for every `#[derive(Credential)]` type so the bound
//! is satisfied by construction.
//!
//! Plugin authors that hand-write a `Credential` impl (escape hatch — not
//! the canonical path) must hand-write the five `IsX` impls too; the
//! registry refuses to register a credential without them at the type
//! system level.
//!
//! # Why type-level constants instead of `TypeId` lookup
//!
//! The const-bool route compiles to a five-instruction `OR` chain at
//! `compute_capabilities` — no allocation, no dynamic dispatch, no
//! coherence work. A `TypeId`-keyed runtime registry would require a
//! second registration step (`with_capability::<C, Refreshable>()`),
//! re-introducing the self-attestation surface §15.8 closes. The macro
//! emission keeps the trait-list and the const-bool list in lockstep
//! within a single attribute declaration.
//!
//! # Macro / hand-roll mismatch
//!
//! - `IsRefreshable::VALUE = true` without `impl Refreshable for X` → the engine `dispatch_*`
//!   binding fails to compile (probe 4 territory). Macro authors catch this at expansion; hand-roll
//!   authors at the dispatch site.
//! - `IsRefreshable::VALUE = false` with `impl Refreshable for X` → the registry under-reports the
//!   capability, [`CredentialRegistry::iter_compatible`] excludes the credential. Operator-visible
//!   discovery bug, not a security failure (worst case: refresh is structurally unreachable for the
//!   credential in question).

use bitflags::bitflags;

bitflags! {
    /// Capability set computed at registration time.
    ///
    /// Authoritative source of capability discovery per Tech Spec §15.8 —
    /// supersedes the metadata-field reads that previously controlled
    /// `iter_compatible` filtering and refresh-dispatcher entry points.
    /// Operator UI / discovery code reads this via
    /// [`CredentialRegistry::capabilities_of`](crate::CredentialRegistry::capabilities_of)
    /// or [`CredentialRegistry::iter_compatible`](crate::CredentialRegistry::iter_compatible).
    #[derive(Clone, Copy, Default, Debug, Eq, PartialEq, Hash)]
    pub struct Capabilities: u8 {
        /// Credential implements [`Interactive`](crate::Interactive).
        const INTERACTIVE = 1 << 0;
        /// Credential implements [`Refreshable`](crate::Refreshable).
        const REFRESHABLE = 1 << 1;
        /// Credential implements [`Revocable`](crate::Revocable).
        const REVOCABLE = 1 << 2;
        /// Credential implements [`Testable`](crate::Testable).
        const TESTABLE = 1 << 3;
        /// Credential implements [`Dynamic`](crate::Dynamic).
        const DYNAMIC = 1 << 4;
    }
}

/// Per-credential capability report — five blanket-bool traits emitted by
/// `#[derive(Credential)]` so [`compute_capabilities`] can fold them into
/// a [`Capabilities`] set without runtime reflection.
///
/// Hidden from rustdoc and the public crate prelude on purpose — these
/// traits are an implementation channel between the macro and the
/// registry, not a stable extension surface for plugin authors. Hand-roll
/// `Credential` impls must implement all five (the bound on
/// [`compute_capabilities`] enforces this at compile time), but the
/// trait names are subject to change as detection evolves.
///
/// # Stability
///
/// Treat as semver-private: a future Stage may collapse these into a
/// single trait with five associated consts, or extend the set as new
/// capability sub-traits land. The bitflag surface in [`Capabilities`]
/// is the stable contract.
#[doc(hidden)]
pub mod plugin_capability_report {
    /// Reports whether the credential implements
    /// [`Interactive`](crate::Interactive).
    pub trait IsInteractive {
        /// `true` when the credential type implements `Interactive`.
        const VALUE: bool;
    }

    /// Reports whether the credential implements
    /// [`Refreshable`](crate::Refreshable).
    pub trait IsRefreshable {
        /// `true` when the credential type implements `Refreshable`.
        const VALUE: bool;
    }

    /// Reports whether the credential implements
    /// [`Revocable`](crate::Revocable).
    pub trait IsRevocable {
        /// `true` when the credential type implements `Revocable`.
        const VALUE: bool;
    }

    /// Reports whether the credential implements
    /// [`Testable`](crate::Testable).
    pub trait IsTestable {
        /// `true` when the credential type implements `Testable`.
        const VALUE: bool;
    }

    /// Reports whether the credential implements
    /// [`Dynamic`](crate::Dynamic).
    pub trait IsDynamic {
        /// `true` when the credential type implements `Dynamic`.
        const VALUE: bool;
    }
}

/// Compute the capability set for credential type `C` from its
/// [`plugin_capability_report`] trait constants.
///
/// Called once per registered credential by
/// [`CredentialRegistry::register`](crate::CredentialRegistry::register).
/// Returns the `OR` of every flag whose corresponding `IsX::VALUE` is
/// `true`.
///
/// # Bound rationale
///
/// `C` must implement all five `IsX` traits — `#[derive(Credential)]`
/// emits all five, hand-roll authors must add them. The bound is the
/// type-system gate that prevents a plugin from registering without
/// declaring its capability surface explicitly.
#[must_use]
pub fn compute_capabilities<C>() -> Capabilities
where
    C: plugin_capability_report::IsInteractive
        + plugin_capability_report::IsRefreshable
        + plugin_capability_report::IsRevocable
        + plugin_capability_report::IsTestable
        + plugin_capability_report::IsDynamic,
{
    let mut caps = Capabilities::empty();
    if <C as plugin_capability_report::IsInteractive>::VALUE {
        caps.insert(Capabilities::INTERACTIVE);
    }
    if <C as plugin_capability_report::IsRefreshable>::VALUE {
        caps.insert(Capabilities::REFRESHABLE);
    }
    if <C as plugin_capability_report::IsRevocable>::VALUE {
        caps.insert(Capabilities::REVOCABLE);
    }
    if <C as plugin_capability_report::IsTestable>::VALUE {
        caps.insert(Capabilities::TESTABLE);
    }
    if <C as plugin_capability_report::IsDynamic>::VALUE {
        caps.insert(Capabilities::DYNAMIC);
    }
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in test credential with explicit capability constants —
    /// exercises the `Refreshable`-only fold without depending on the
    /// real built-in credentials (which carry their own coverage).
    struct TestCred;
    impl plugin_capability_report::IsInteractive for TestCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsRefreshable for TestCred {
        const VALUE: bool = true;
    }
    impl plugin_capability_report::IsRevocable for TestCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsTestable for TestCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsDynamic for TestCred {
        const VALUE: bool = false;
    }

    /// Stand-in for a fully-static credential — every IsX is false; the
    /// computed bitflag set is empty.
    struct StaticCred;
    impl plugin_capability_report::IsInteractive for StaticCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsRefreshable for StaticCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsRevocable for StaticCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsTestable for StaticCred {
        const VALUE: bool = false;
    }
    impl plugin_capability_report::IsDynamic for StaticCred {
        const VALUE: bool = false;
    }

    /// Stand-in for a richly-capable OAuth2-shaped credential — the
    /// canonical "Interactive + Refreshable + Revocable + Testable, not
    /// Dynamic" combination per Stage 3 OAuth2 fix wave.
    struct RichCred;
    impl plugin_capability_report::IsInteractive for RichCred {
        const VALUE: bool = true;
    }
    impl plugin_capability_report::IsRefreshable for RichCred {
        const VALUE: bool = true;
    }
    impl plugin_capability_report::IsRevocable for RichCred {
        const VALUE: bool = true;
    }
    impl plugin_capability_report::IsTestable for RichCred {
        const VALUE: bool = true;
    }
    impl plugin_capability_report::IsDynamic for RichCred {
        const VALUE: bool = false;
    }

    #[test]
    fn refreshable_only_yields_single_flag() {
        let caps = compute_capabilities::<TestCred>();
        assert_eq!(caps, Capabilities::REFRESHABLE);
    }

    #[test]
    fn fully_static_yields_empty() {
        let caps = compute_capabilities::<StaticCred>();
        assert_eq!(caps, Capabilities::empty());
    }

    #[test]
    fn rich_credential_yields_four_of_five_flags() {
        let caps = compute_capabilities::<RichCred>();
        assert_eq!(
            caps,
            Capabilities::INTERACTIVE
                | Capabilities::REFRESHABLE
                | Capabilities::REVOCABLE
                | Capabilities::TESTABLE,
        );
        assert!(!caps.contains(Capabilities::DYNAMIC));
    }
}
