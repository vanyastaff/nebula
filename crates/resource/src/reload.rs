//! Reload outcome types for config hot-reload and credential rotation.

/// Result of per-topology reload dispatch.
///
/// Used by [`Manager`](crate::manager::Manager) for both config changes
/// and credential rotation. Each variant maps to one of the possible
/// outcomes when a reload is dispatched to a topology runtime.
///
/// The historical `Restarting` variant was reachable only from the
/// pre-collapse `Service` topology (a former engine-side daemon row);
/// post topology collapse (3 runtimes) it is structurally unreachable
/// and was removed. The two surviving outcomes — `NoChange` (fingerprint
/// identical) and `SwappedImmediately` (config/credential swapped in
/// place) — cover every live registry row. See the
/// [`manager`](crate::manager) module docs for the relabel rationale.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// The config `ArcSwap` and the topology's fingerprint are swapped
    /// **immediately** and synchronously — before this variant is returned.
    /// The fingerprint swap runs unconditionally for every topology (it is
    /// simply a no-op for one that tracks no config fingerprint of its own);
    /// what differs per topology is whether the *live runtime* picks up the
    /// new config on the very next acquire or lazily on a later one:
    ///
    /// - **Pooled** — the new fingerprint evicts stale-fingerprint idle
    ///   instances lazily: an idle instance is only checked (and, if its
    ///   fingerprint differs, destroyed) the next time it is popped for
    ///   checkout, not eagerly on reload. A fresh instance is then built
    ///   against the new config.
    /// - **Resident** — the shared master rebuilds on the next acquire:
    ///   `Resident::clone_or_create` compares the live config fingerprint
    ///   under `create_lock` and rebuilds the master handle when it has
    ///   changed, so every acquire after the reload observes the new config.
    /// - **Bounded-Exclusive** — the reused instance is fingerprint-aware:
    ///   `Bounded::accept` rejects (evicts) the currently-held instance when
    ///   `Bounded::set_fingerprint` has advanced past the fingerprint it was
    ///   built against, so the next acquire rebuilds it. `Capped`/`Unbounded`
    ///   build a fresh instance from the current config on every acquire
    ///   already, so a config swap is visible immediately for them.
    ///
    /// No topology drains or force-rebuilds the live runtime *eagerly* on
    /// reload; every topology instead applies the swap lazily, at the next
    /// point it would touch the instance anyway (checkout / acquire). This is
    /// the accurate, current-behavior meaning of "immediately": the *config*
    /// swap is immediate and synchronous, the *runtime* catch-up is lazy —
    /// see the [`manager`](crate::manager) module docs' reload-application
    /// ledger entry.
    SwappedImmediately,
    /// Fingerprint identical — no change needed.
    NoChange,
}
