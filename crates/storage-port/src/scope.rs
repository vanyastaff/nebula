//! Plain-data tenant scope.
//!
//! [`Scope`] is a value type only. Resolving a scope from a principal and
//! enforcing cross-tenant denial is policy and lives in `nebula-tenancy`.
//! Keeping the type here (Core tier) lets tenant-scoped port signatures
//! require it without an upward dependency on the policy crate.
use serde::{Deserialize, Serialize};

/// Workspace + org isolation key. Required by every tenant-scoped operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    /// Workspace identifier.
    pub workspace_id: String,
    /// Organization identifier.
    pub org_id: String,
}

impl Scope {
    /// Build a scope from workspace + org ids.
    pub fn new(workspace_id: impl Into<String>, org_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            org_id: org_id.into(),
        }
    }

    /// The canonical credential `owner_id` key for this scope.
    ///
    /// This is the **single** derivation every credential producer routes
    /// through before constructing the mandatory owner-bound persistence
    /// selector (ADR-0088 D7).
    /// Before this existed, the API edge keyed `"{org}:{workspace}"` while the
    /// runtime keyed `"{org}/{workspace}"`; once both planes share a backend
    /// that mismatch would silently partition a single tenant's credentials
    /// into two non-intersecting halves.
    ///
    /// # Encoding (collision-safe)
    ///
    /// `org_id` / `workspace_id` are unvalidated free strings, so **no single
    /// separator is safe**: a raw `':'` collapses `org="a", ws="b:c"` and
    /// `org="a:b", ws="c"` to the same key (and `'/'` has the identical flaw).
    /// The owner segment is therefore **length-prefixed** — the leading
    /// `org_id.len()` pins exactly where `org_id` ends, so the mapping from
    /// `(org_id, workspace_id)` to key is injective for arbitrary id bytes.
    /// The `\u{1e}` (ASCII Record Separator) delimiters keep it readable in
    /// logs; the key is an opaque internal match value, never wire-exposed.
    /// Only [`Self::from_credential_owner_id`] reads it back, so storage can
    /// file a credential under the workspace that owns it.
    #[must_use]
    pub fn credential_owner_id(&self) -> String {
        format!(
            "{}\u{1e}{}\u{1e}{}",
            self.org_id.len(),
            self.org_id,
            self.workspace_id
        )
    }

    /// The scope a [`Self::credential_owner_id`] key was derived from.
    ///
    /// `None` for any string that is not exactly the canonical key of some
    /// scope: the mapping is injective, so a key names at most one scope.
    #[must_use]
    pub fn from_credential_owner_id(owner_id: &str) -> Option<Self> {
        let (length, rest) = owner_id.split_once('\u{1e}')?;
        let length: usize = length.parse().ok()?;
        let org_id = rest.get(..length)?;
        let workspace_id = rest.get(length..)?.strip_prefix('\u{1e}')?;
        let scope = Self::new(workspace_id, org_id);
        (scope.credential_owner_id() == owner_id).then_some(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::Scope;

    #[test]
    fn owner_id_is_injective_across_embedded_separators() {
        // The classic raw-separator collision: with `:` or `/` these two
        // distinct tenants would derive the same key. Length-prefixing keeps
        // them distinct.
        let org_a_ws_c = Scope::new("c", "a").credential_owner_id(); // org="a", ws="c"
        let org_a_ws_bc = Scope::new("b:c", "a").credential_owner_id(); // org="a", ws="b:c"
        let org_ab_ws_c = Scope::new("c", "a:b").credential_owner_id(); // org="a:b", ws="c"
        assert_ne!(org_a_ws_c, org_a_ws_bc);
        assert_ne!(org_a_ws_c, org_ab_ws_c);
        assert_ne!(org_a_ws_bc, org_ab_ws_c);

        // Even when ids contain the RS delimiter itself, distinctness holds.
        let plain = Scope::new("y", "x").credential_owner_id();
        let org_with_rs = Scope::new("y", "x\u{1e}").credential_owner_id();
        assert_ne!(plain, org_with_rs);
    }

    #[test]
    fn owner_id_reads_back_to_its_scope() {
        for scope in [
            Scope::new("ws", "org"),
            Scope::new("b:c", "a"),
            Scope::new("y", "x\u{1e}"),
            Scope::new("\u{1e}", "\u{1e}\u{1e}"),
            Scope::new("", ""),
            Scope::new("ворк", "орг"),
        ] {
            assert_eq!(
                Scope::from_credential_owner_id(&scope.credential_owner_id()),
                Some(scope)
            );
        }
        for key in [
            "",
            "owner-a",
            "3\u{1e}org",
            "4\u{1e}org\u{1e}ws",
            "03\u{1e}org\u{1e}ws",
            "+3\u{1e}org\u{1e}ws",
            "1\u{1e}\u{444}\u{1e}ws",
        ] {
            assert_eq!(Scope::from_credential_owner_id(key), None, "{key:?}");
        }
    }

    #[test]
    fn owner_id_is_stable_for_equal_scopes() {
        assert_eq!(
            Scope::new("ws", "org").credential_owner_id(),
            Scope::new("ws", "org").credential_owner_id()
        );
    }
}
