//! Credential-scope identity for keying long-lived plugin processes.
//!
//! `ScopeHash` is the per-process isolation key from
//! plugin invocations with a different bound credential-slot set MUST run
//! in different processes, so a plugin can never name a slot outside its
//! scope. This module computes the hash from caller-supplied slot-name
//! strings **only** — it never resolves credentials and pulls no
//! Business-tier dependency. The engine extracts the bound slot names from
//! the workflow node and hands them in; the leaf stays scope-agnostic
//! transport plus this pure helper.

use sha2::{Digest, Sha256};

/// SHA-256 over the sorted, unambiguously-framed credential-slot-name set
/// bound to a plugin invocation.
///
/// Opaque, fixed-size, cheap to compare/hash — suitable as a process-pool
/// key and for the reattach identity tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeHash([u8; 32]);

impl ScopeHash {
    /// The raw 32-byte digest (e.g. for the persisted reattach tuple or a
    /// short logged prefix).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Domain-separation tag: pins the framing version and stops this digest
/// from colliding with any other SHA-256 use in the workspace.
const DOMAIN: &[u8] = b"nebula.scope.v1";

/// Compute the [`ScopeHash`] for a set of bound credential-slot names.
///
/// Order-independent (the input is sorted before hashing) and
/// collision-safe: each slot name is length-prefixed, so `["ab", "c"]`
/// and `["a", "bc"]` hash distinctly — without that, distinct bound sets
/// could collide and merge two credential scopes into one process,
/// breaking the. The slots are treated
/// as an ordered multiset (no de-duplication): the scope is the exact
/// bound set from workflow-config.
#[must_use]
pub fn scope_hash(slots: &[&str]) -> ScopeHash {
    let mut sorted: Vec<&str> = slots.to_vec();
    sorted.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update((sorted.len() as u64).to_le_bytes());
    for slot in sorted {
        let bytes = slot.as_bytes();
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }

    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    ScopeHash(out)
}

#[cfg(test)]
mod tests {
    use super::scope_hash;

    #[test]
    fn scope_hash_is_order_independent() {
        assert_eq!(
            scope_hash(&["stripe_key", "slack_token"]),
            scope_hash(&["slack_token", "stripe_key"]),
        );
    }

    #[test]
    fn scope_hash_frames_slots_unambiguously() {
        // A naive concat-then-hash would collide these; length-prefixing
        // must not. This is the
        // distinct bound set => a distinct process key.
        assert_ne!(scope_hash(&["ab", "c"]), scope_hash(&["a", "bc"]));
        assert_ne!(scope_hash(&["a", "b"]), scope_hash(&["ab"]));
    }

    #[test]
    fn scope_hash_empty_is_deterministic_and_distinct_from_one_empty_slot() {
        assert_eq!(scope_hash(&[]), scope_hash(&[]));
        assert_ne!(scope_hash(&[]), scope_hash(&[""]));
    }

    #[test]
    fn scope_hash_distinct_sets_differ() {
        assert_ne!(scope_hash(&["a"]), scope_hash(&["b"]));
        assert_ne!(scope_hash(&["a", "b"]), scope_hash(&["a", "b", "c"]));
    }

    #[test]
    fn scope_hash_is_multiset_not_deduped() {
        // A doubled binding is a different scope than a single one — no
        // silent de-dup that could merge two distinct workflow configs
        // onto the same process key.
        assert_ne!(scope_hash(&["a", "a"]), scope_hash(&["a"]));
    }
}
