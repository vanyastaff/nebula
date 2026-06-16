//! Plugin dependency resolution — topological load-order computation.
//!
//! [`resolve`] performs a white/grey/black DFS over the dependency graph
//! declared in each registered plugin's manifest, producing a topologically
//! sorted list (dependency before dependent). Three failure classes are
//! detected and reported as a typed [`PluginDependencyError`]:
//!
//! - **Missing** — a declared dependency key is not registered.
//! - **Version mismatch** — the registered version does not satisfy the semver
//!   requirement declared by the dependent.
//! - **Cycle** — the dependency graph contains a directed cycle; the error
//!   carries the closed path.

use std::collections::HashMap;

use nebula_core::PluginKey;
use semver::{Version, VersionReq};

use crate::PluginRegistry;

/// Detail payload for [`PluginDependencyError::VersionMismatch`].
///
/// Heap-allocated to keep `PluginDependencyError` below the 128-byte threshold
/// that `clippy::result_large_err` enforces on `Result`-error types.  The
/// struct is public so that callers can read its fields after matching, but it
/// is not re-exported at the crate root — access it via the matched binding.
#[derive(Debug, PartialEq, Eq)]
pub struct VersionMismatchDetail {
    /// The plugin declaring the dependency.
    pub dependent: PluginKey,
    /// The dependency whose version does not match.
    pub dependency: PluginKey,
    /// The requirement declared by the dependent.
    pub required: VersionReq,
    /// The actual version registered.
    pub found: Version,
}

impl std::fmt::Display for VersionMismatchDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "plugin '{}' requires '{}' {}, but found {}",
            self.dependent, self.dependency, self.required, self.found
        )
    }
}

/// Errors from the dependency resolver ([`PluginRegistry::resolve_load_order`]).
#[derive(Debug, thiserror::Error, nebula_error::Classify, PartialEq, Eq)]
pub enum PluginDependencyError {
    /// A declared dependency is not registered in the registry.
    #[classify(category = "not_found", code = "PLUGIN_DEP:MISSING")]
    #[error("plugin '{dependent}' requires '{dependency}' ({required}) which is not registered")]
    MissingDependency {
        /// The plugin declaring the dependency.
        dependent: PluginKey,
        /// The missing dependency key.
        dependency: PluginKey,
        /// The version requirement that cannot be satisfied.
        required: VersionReq,
    },

    /// A declared dependency is registered but its version does not satisfy the requirement.
    ///
    /// The detail is heap-allocated (`VersionMismatchDetail`) to keep the
    /// enum size below the 128-byte threshold `clippy::result_large_err`
    /// enforces on `Result` error types.
    #[classify(category = "validation", code = "PLUGIN_DEP:VERSION_MISMATCH")]
    #[error("{0}")]
    VersionMismatch(Box<VersionMismatchDetail>),

    /// The dependency graph contains a directed cycle.
    ///
    /// `path` is the closed cycle: the first and last elements are the same
    /// node (the cycle entry point), e.g. `[a, b, c, a]`.
    #[classify(category = "validation", code = "PLUGIN_DEP:CYCLE")]
    #[error("dependency cycle detected: {}", format_cycle(path))]
    Cycle {
        /// Closed cycle path; `path[0] == path[last]`.
        path: Vec<PluginKey>,
    },
}

fn format_cycle(path: &[PluginKey]) -> String {
    path.iter()
        .map(PluginKey::as_str)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// DFS colouring state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    /// Not yet visited.
    White,
    /// On the current DFS stack (recursion in progress).
    Grey,
    /// Fully processed.
    Black,
}

/// A pre-computed summary of one plugin's dependencies used during the DFS.
struct DepEntry {
    /// The key of the requiring plugin.
    dep_key: PluginKey,
    /// The semver requirement declared by the requiring plugin.
    req: VersionReq,
}

/// Compute a topological load order for all plugins in `reg`.
///
/// Returns `Ok(order)` where every dependency appears before its dependent,
/// or `Err` with the first detected error (missing dependency, version
/// mismatch, or cycle).
///
/// Roots are visited in ascending string order of their keys so that the
/// result is deterministic regardless of `HashMap` iteration order.
///
/// # Errors
///
/// - [`PluginDependencyError::MissingDependency`] — a declared dependency key
///   is absent from the registry.
/// - [`PluginDependencyError::VersionMismatch`] — the registered plugin's
///   version does not satisfy the declared [`VersionReq`].
/// - [`PluginDependencyError::Cycle`] — the dependency graph contains a
///   directed cycle; the error carries the closed path.
pub(crate) fn resolve(reg: &PluginRegistry) -> Result<Vec<PluginKey>, PluginDependencyError> {
    // Sorted root list for determinism (HashMap order is nondeterministic).
    let mut roots: Vec<PluginKey> = reg.iter().map(|(k, _)| k.clone()).collect();
    roots.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let n = roots.len();
    let mut color: HashMap<PluginKey, Color> =
        roots.iter().cloned().map(|k| (k, Color::White)).collect();

    let mut output: Vec<PluginKey> = Vec::with_capacity(n);
    // Tracks the current DFS path (grey nodes) for cycle path reconstruction.
    let mut grey_stack: Vec<PluginKey> = Vec::new();

    // Each stack frame: (node_being_processed, its_dep_list, next_dep_index).
    // Using a Vec<DepEntry> avoids re-fetching the manifest on every iteration.
    type Frame = (PluginKey, Vec<DepEntry>, usize);
    let mut call_stack: Vec<Frame> = Vec::new();

    for root in &roots {
        if color[root] != Color::White {
            continue;
        }
        let deps = collect_deps(reg, root);
        call_stack.push((root.clone(), deps, 0));
        *color.get_mut(root).expect("color map was built from roots") = Color::Grey;
        grey_stack.push(root.clone());

        // Drive the iterative DFS until the current connected component is done.
        while let Some(frame) = call_stack.last_mut() {
            let (node, children, idx) = frame;
            if *idx < children.len() {
                let entry = &children[*idx];
                let child_key = entry.dep_key.clone();
                let req = entry.req.clone();
                *idx += 1;

                match reg.get(&child_key) {
                    None => {
                        return Err(PluginDependencyError::MissingDependency {
                            dependent: node.clone(),
                            dependency: child_key,
                            required: req,
                        });
                    },
                    Some(dep_rp) => {
                        let found = dep_rp.version().clone();
                        if !req.matches(&found) {
                            return Err(PluginDependencyError::VersionMismatch(Box::new(
                                VersionMismatchDetail {
                                    dependent: node.clone(),
                                    dependency: child_key,
                                    required: req,
                                    found,
                                },
                            )));
                        }
                        match color[&child_key] {
                            Color::Grey => {
                                // Back-edge: cycle detected.
                                // Slice grey_stack from the first occurrence of the
                                // cycle entry, then append that node again to close
                                // the path (e.g. [a, b, c, a]).
                                let start = grey_stack
                                    .iter()
                                    .position(|k| k == &child_key)
                                    .expect("grey node must be in grey_stack");
                                let mut path: Vec<PluginKey> = grey_stack[start..].to_vec();
                                path.push(child_key);
                                return Err(PluginDependencyError::Cycle { path });
                            },
                            Color::Black => {
                                // Already fully processed — no further work needed.
                            },
                            Color::White => {
                                // Recurse: push the child frame.
                                let child_deps = collect_deps(reg, &child_key);
                                *color
                                    .get_mut(&child_key)
                                    .expect("color map covers all registered plugins") =
                                    Color::Grey;
                                grey_stack.push(child_key.clone());
                                call_stack.push((child_key, child_deps, 0));
                            },
                        }
                    },
                }
            } else {
                // All outgoing edges processed — finalize this node.
                let finished = node.clone();
                call_stack.pop();
                *color
                    .get_mut(&finished)
                    .expect("color map covers all registered plugins") = Color::Black;
                grey_stack.pop();
                output.push(finished);
            }
        }
    }

    // Topo-order invariant (debug builds only): for every edge (dep → node),
    // dep's index in `output` is strictly less than node's index.
    debug_assert!(
        {
            let idx: HashMap<&PluginKey, usize> =
                output.iter().enumerate().map(|(i, k)| (k, i)).collect();
            output.iter().all(|node| {
                reg.get(node).is_none_or(|rp| {
                    rp.manifest().dependencies().iter().all(|d| {
                        // Missing deps are caught above; skip if absent here.
                        idx.get(d.key()).is_none_or(|&di| di < idx[node])
                    })
                })
            })
        },
        "topological invariant violated: a dependency appears after its dependent in the output"
    );

    Ok(output)
}

/// Pre-collect the declared dependency entries for `node` from the registry.
///
/// Returns an empty `Vec` if `node` is not in the registry (should not happen
/// during normal resolution, but is safe to handle).
fn collect_deps(reg: &PluginRegistry, node: &PluginKey) -> Vec<DepEntry> {
    reg.get(node).map_or_else(Vec::new, |rp| {
        rp.manifest()
            .dependencies()
            .iter()
            .map(|d| DepEntry {
                dep_key: d.key().clone(),
                req: d.req().clone(),
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_metadata::{PluginDependency, PluginManifest};
    use semver::{Version, VersionReq};

    use super::*;
    use crate::{ResolvedPlugin, plugin::Plugin};

    // ── Helpers ─────────────────────────────────────────────────────────────────

    #[derive(Debug)]
    struct StubPlugin(PluginManifest);
    impl Plugin for StubPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.0
        }
    }

    fn resolved(manifest: PluginManifest) -> Arc<ResolvedPlugin> {
        Arc::new(ResolvedPlugin::from(StubPlugin(manifest)).unwrap())
    }

    fn simple(key: &str) -> Arc<ResolvedPlugin> {
        resolved(PluginManifest::builder(key, key).build().unwrap())
    }

    fn versioned(key: &str, version: Version) -> Arc<ResolvedPlugin> {
        resolved(
            PluginManifest::builder(key, key)
                .version(version)
                .build()
                .unwrap(),
        )
    }

    fn with_deps(key: &str, version: Version, deps: &[(&str, &str)]) -> Arc<ResolvedPlugin> {
        let mut builder = PluginManifest::builder(key, key).version(version);
        for &(dep_key, req_str) in deps {
            builder = builder.dependency(PluginDependency::new(
                dep_key.parse().unwrap(),
                req_str.parse().unwrap(),
            ));
        }
        resolved(builder.build().unwrap())
    }

    fn reg(plugins: Vec<Arc<ResolvedPlugin>>) -> PluginRegistry {
        let mut r = PluginRegistry::new();
        for p in plugins {
            r.register(p).unwrap();
        }
        r
    }

    fn key_strs(order: Vec<PluginKey>) -> Vec<String> {
        order.into_iter().map(|k| k.as_str().to_owned()).collect()
    }

    fn pos(order: &[PluginKey], name: &str) -> usize {
        order
            .iter()
            .position(|k| k.as_str() == name)
            .unwrap_or_else(|| panic!("{name} not found in order"))
    }

    // ── Basic cases ─────────────────────────────────────────────────────────────

    #[test]
    fn empty_registry() {
        let r = PluginRegistry::new();
        let order = resolve(&r).unwrap();
        assert!(order.is_empty());
    }

    #[test]
    fn single_no_dep() {
        let r = reg(vec![simple("alpha")]);
        let order = resolve(&r).unwrap();
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].as_str(), "alpha");
    }

    #[test]
    fn linear_chain_abc() {
        // c depends on b, b depends on a => order: a, b, c
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[]),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^1")]),
            with_deps("c", Version::new(1, 0, 0), &[("b", "^1")]),
        ]);
        let order = resolve(&r).unwrap();
        assert!(pos(&order, "a") < pos(&order, "b"), "a must precede b");
        assert!(pos(&order, "b") < pos(&order, "c"), "b must precede c");
    }

    #[test]
    fn diamond_dag() {
        // d depends on b and c; both depend on a => a first, d last
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[]),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^1")]),
            with_deps("c", Version::new(1, 0, 0), &[("a", "^1")]),
            with_deps("d", Version::new(1, 0, 0), &[("b", "^1"), ("c", "^1")]),
        ]);
        let order = resolve(&r).unwrap();
        assert!(pos(&order, "a") < pos(&order, "b"));
        assert!(pos(&order, "a") < pos(&order, "c"));
        assert!(pos(&order, "b") < pos(&order, "d"));
        assert!(pos(&order, "c") < pos(&order, "d"));
    }

    // ── Error: missing dependency ────────────────────────────────────────────────

    #[test]
    fn missing_dependency() {
        // b depends on a which is not registered
        let r = reg(vec![with_deps("b", Version::new(1, 0, 0), &[("a", "^1")])]);
        let err = resolve(&r).unwrap_err();
        assert!(
            matches!(
                &err,
                PluginDependencyError::MissingDependency { dependent, dependency, .. }
                if dependent.as_str() == "b" && dependency.as_str() == "a"
            ),
            "unexpected error: {err}"
        );
    }

    // ── Error: version mismatch ──────────────────────────────────────────────────

    #[test]
    fn version_mismatch_major() {
        // b requires a ^2.0.0, but a is 1.0.0
        let r = reg(vec![
            versioned("a", Version::new(1, 0, 0)),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^2")]),
        ]);
        let err = resolve(&r).unwrap_err();
        match &err {
            PluginDependencyError::VersionMismatch(detail) => {
                assert_eq!(detail.dependency.as_str(), "a");
                assert_eq!(detail.required, "^2".parse::<VersionReq>().unwrap());
                assert_eq!(detail.found, Version::new(1, 0, 0));
            },
            _ => panic!("expected VersionMismatch, got {err}"),
        }
    }

    #[test]
    fn version_satisfies_within_major() {
        // b requires a ^1.0.0 and a is 1.5.0 — must succeed
        let r = reg(vec![
            versioned("a", Version::new(1, 5, 0)),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^1.0.0")]),
        ]);
        resolve(&r).unwrap();
    }

    // ── Error: cycles ────────────────────────────────────────────────────────────

    #[test]
    fn self_cycle() {
        let r = reg(vec![with_deps("a", Version::new(1, 0, 0), &[("a", "^1")])]);
        let err = resolve(&r).unwrap_err();
        match &err {
            PluginDependencyError::Cycle { path } => {
                assert_eq!(
                    path.first().unwrap(),
                    path.last().unwrap(),
                    "cycle path must be closed"
                );
                assert_eq!(path[0].as_str(), "a");
            },
            _ => panic!("expected Cycle, got {err}"),
        }
    }

    #[test]
    fn two_cycle() {
        // a -> b -> a
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[("b", "^1")]),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^1")]),
        ]);
        let err = resolve(&r).unwrap_err();
        match &err {
            PluginDependencyError::Cycle { path } => {
                assert_eq!(
                    path.first().unwrap(),
                    path.last().unwrap(),
                    "cycle path must be closed"
                );
                // Closed 2-cycle: e.g. [a, b, a] — 3 elements
                assert!(
                    path.len() >= 3,
                    "closed 2-cycle must have at least 3 elements: {path:?}"
                );
            },
            _ => panic!("expected Cycle, got {err}"),
        }
    }

    #[test]
    fn three_cycle_closed_path() {
        // a -> b -> c -> a
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[("b", "^1")]),
            with_deps("b", Version::new(1, 0, 0), &[("c", "^1")]),
            with_deps("c", Version::new(1, 0, 0), &[("a", "^1")]),
        ]);
        let err = resolve(&r).unwrap_err();
        match &err {
            PluginDependencyError::Cycle { path } => {
                assert_eq!(
                    path.first().unwrap(),
                    path.last().unwrap(),
                    "cycle path must be closed"
                );
                // Closed 3-cycle: e.g. [a, b, c, a] — 4 elements
                assert_eq!(path.len(), 4, "closed 3-cycle path: {path:?}");
            },
            _ => panic!("expected Cycle, got {err}"),
        }
    }

    #[test]
    fn cycle_path_excludes_acyclic_upstream() {
        // a -> b -> c -> b  (a is acyclic upstream; b and c form a 2-cycle).
        // Roots are visited in sorted order, so `a` is entered first and is still
        // on the grey stack when the b<->c back-edge is found. The reported cycle
        // path must contain only the cycle members (b, c) and must NOT contain
        // `a`, which merely reaches the cycle — this pins the grey-stack slice to
        // start at the cycle entry, not the DFS root.
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[("b", "^1")]),
            with_deps("b", Version::new(1, 0, 0), &[("c", "^1")]),
            with_deps("c", Version::new(1, 0, 0), &[("b", "^1")]),
        ]);
        let err = resolve(&r).unwrap_err();
        match &err {
            PluginDependencyError::Cycle { path } => {
                assert_eq!(
                    path.first().unwrap(),
                    path.last().unwrap(),
                    "cycle path must be closed"
                );
                // The closed path for the b<->c cycle is [b, c, b] — 3 elements.
                let names: Vec<&str> = path.iter().map(PluginKey::as_str).collect();
                assert!(
                    !names.contains(&"a"),
                    "acyclic upstream node 'a' must not appear in cycle path: {names:?}"
                );
                assert!(
                    names.contains(&"b"),
                    "cycle member 'b' must be in path: {names:?}"
                );
                assert!(
                    names.contains(&"c"),
                    "cycle member 'c' must be in path: {names:?}"
                );
            },
            _ => panic!("expected Cycle, got {err}"),
        }
    }

    // ── Determinism ─────────────────────────────────────────────────────────────

    #[test]
    fn deterministic_across_insertion_order() {
        // chain: a <- b <- c.  Register in three different orders and verify
        // the resolved load order is identical each time.
        let build = |insertion: &[&str]| {
            let plugins: Vec<Arc<ResolvedPlugin>> = insertion
                .iter()
                .map(|&k| {
                    let deps: &[(&str, &str)] = match k {
                        "b" => &[("a", "^1")],
                        "c" => &[("b", "^1")],
                        _ => &[],
                    };
                    with_deps(k, Version::new(1, 0, 0), deps)
                })
                .collect();
            key_strs(resolve(&reg(plugins)).unwrap())
        };

        let order1 = build(&["a", "b", "c"]);
        let order2 = build(&["c", "b", "a"]);
        let order3 = build(&["b", "c", "a"]);

        assert_eq!(
            order1, order2,
            "insertion order must not affect resolve order"
        );
        assert_eq!(
            order1, order3,
            "insertion order must not affect resolve order"
        );
    }

    // ── Display / error formatting ───────────────────────────────────────────────

    #[test]
    fn missing_dep_display() {
        let err = PluginDependencyError::MissingDependency {
            dependent: "b".parse().unwrap(),
            dependency: "a".parse().unwrap(),
            required: "^1".parse().unwrap(),
        };
        let s = err.to_string();
        assert!(s.contains('b'), "expected 'b' in: {s}");
        assert!(s.contains('a'), "expected 'a' in: {s}");
        assert!(
            s.contains("not registered"),
            "expected 'not registered' in: {s}"
        );
    }

    #[test]
    fn version_mismatch_display() {
        let err = PluginDependencyError::VersionMismatch(Box::new(VersionMismatchDetail {
            dependent: "b".parse().unwrap(),
            dependency: "a".parse().unwrap(),
            required: "^2".parse().unwrap(),
            found: Version::new(1, 0, 0),
        }));
        let s = err.to_string();
        assert!(s.contains("^2"), "required in: {s}");
        assert!(s.contains("1.0.0"), "found in: {s}");
    }

    #[test]
    fn cycle_display_closed() {
        let err = PluginDependencyError::Cycle {
            path: vec![
                "a".parse().unwrap(),
                "b".parse().unwrap(),
                "a".parse().unwrap(),
            ],
        };
        let s = err.to_string();
        // Must render as "a -> b -> a"
        assert!(s.contains("a -> b -> a"), "expected closed cycle in: {s}");
    }
}

// ── Property tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod prop_tests {
    use std::sync::Arc;

    use nebula_metadata::{PluginDependency, PluginManifest};
    use proptest::prelude::*;
    use semver::Version;

    use super::resolve;
    use crate::{PluginRegistry, ResolvedPlugin, plugin::Plugin};

    #[derive(Debug)]
    struct StubPlugin(PluginManifest);
    impl Plugin for StubPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.0
        }
    }

    fn resolved(manifest: PluginManifest) -> Arc<ResolvedPlugin> {
        Arc::new(ResolvedPlugin::from(StubPlugin(manifest)).unwrap())
    }

    /// Generate a random acyclic DAG of `n` nodes where node `i` may only
    /// depend on nodes with strictly lower indices — this construction
    /// guarantees no cycles.  Keys are `p0`, `p1`, …, `p{n-1}`.
    fn acyclic_dag(n: usize, dep_mask: &[u64]) -> PluginRegistry {
        let mut r = PluginRegistry::new();
        for i in 0..n {
            let key_str = format!("p{i}");
            let mut builder =
                PluginManifest::builder(&key_str, &key_str).version(Version::new(1, 0, 0));
            // Bits of dep_mask[i] select which lower-indexed nodes to depend on.
            let mask = dep_mask.get(i).copied().unwrap_or(0);
            for j in 0..i {
                if (mask >> j) & 1 == 1 {
                    let dep_str = format!("p{j}");
                    builder = builder.dependency(PluginDependency::new(
                        dep_str.parse().unwrap(),
                        "^1".parse().unwrap(),
                    ));
                }
            }
            r.register(resolved(builder.build().unwrap())).unwrap();
        }
        r
    }

    proptest! {
        #[test]
        fn acyclic_dag_resolves_ok_and_deps_precede_dependents(
            n in 1usize..=8,
            dep_mask in proptest::collection::vec(any::<u64>(), 8),
        ) {
            let r = acyclic_dag(n, &dep_mask);
            let order = resolve(&r).expect("acyclic DAG must always resolve");

            let index_map: std::collections::HashMap<String, usize> = order
                .iter()
                .enumerate()
                .map(|(i, k)| (k.as_str().to_owned(), i))
                .collect();

            for node in &order {
                let rp = r.get(node).unwrap();
                for dep in rp.manifest().dependencies() {
                    let dep_idx = index_map[dep.key().as_str()];
                    let node_idx = index_map[node.as_str()];
                    prop_assert!(
                        dep_idx < node_idx,
                        "dep '{}' (idx {dep_idx}) must precede '{}' (idx {node_idx})",
                        dep.key().as_str(),
                        node.as_str(),
                    );
                }
            }
        }
    }
}
