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

/// A registered plugin reduced to the data dependency resolution needs.
struct NodeSnapshot {
    /// The plugin's own key.
    key: PluginKey,
    /// The plugin's version (checked against dependents' requirements).
    version: Version,
    /// `(dependency key, version requirement)` pairs this plugin declares.
    deps: Vec<(PluginKey, VersionReq)>,
}

/// Compute a topological load order for all plugins in `reg`.
///
/// Returns `Ok(order)` where every dependency appears before its dependent,
/// or `Err` with the first detected error (missing dependency, version
/// mismatch, or cycle).
///
/// Roots are visited in ascending string order of their keys so that the
/// result is deterministic regardless of `HashMap` iteration order. Plugins
/// are indexed up front, so every internal lookup during the traversal is an
/// in-bounds slice access — the DFS has no fallible internal lookups.
///
/// # Errors
///
/// - [`PluginDependencyError::MissingDependency`] — a declared dependency key
///   is absent from the registry.
/// - [`PluginDependencyError::VersionMismatch`] — the registered plugin's
///   version does not satisfy the declared [`VersionReq`].
/// - [`PluginDependencyError::Cycle`] — the dependency graph contains a
///   directed cycle; the error carries the closed path.
///
/// Only the first problem is returned, with a deterministic precedence:
/// unsatisfiable declared dependencies ([`PluginDependencyError::MissingDependency`]
/// and [`PluginDependencyError::VersionMismatch`]) are detected while building the
/// edge graph in ascending dependent-key order — before any
/// [`PluginDependencyError::Cycle`] is reported.
pub(crate) fn resolve(reg: &PluginRegistry) -> Result<Vec<PluginKey>, PluginDependencyError> {
    // Snapshot every registered plugin as (key, version, declared deps), sorted
    // by key so the traversal — and thus the output — is deterministic
    // regardless of `HashMap` iteration order.
    let mut entries: Vec<NodeSnapshot> = reg
        .iter()
        .map(|(key, rp)| NodeSnapshot {
            key: key.clone(),
            version: rp.version().clone(),
            deps: rp
                .manifest()
                .dependencies()
                .iter()
                .map(|d| (d.key().clone(), d.req().clone()))
                .collect(),
        })
        .collect();
    entries.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));

    let n = entries.len();
    let nodes: Vec<PluginKey> = entries.iter().map(|e| e.key.clone()).collect();
    let versions: Vec<Version> = entries.iter().map(|e| e.version.clone()).collect();
    let key_to_index: HashMap<&PluginKey, usize> =
        nodes.iter().enumerate().map(|(i, key)| (key, i)).collect();

    // Resolve each node's declared dependencies to node indices, surfacing the
    // two caller-facing failures here so the DFS operates on a clean index
    // graph (every edge is a valid `index -> index`).
    let mut adjacency: Vec<Vec<usize>> = Vec::with_capacity(n);
    for (dependent_index, entry) in entries.iter().enumerate() {
        let mut edges = Vec::with_capacity(entry.deps.len());
        for (dep_key, req) in &entry.deps {
            let Some(&dep_index) = key_to_index.get(dep_key) else {
                return Err(PluginDependencyError::MissingDependency {
                    dependent: nodes[dependent_index].clone(),
                    dependency: dep_key.clone(),
                    required: req.clone(),
                });
            };
            if !req.matches(&versions[dep_index]) {
                return Err(PluginDependencyError::VersionMismatch(Box::new(
                    VersionMismatchDetail {
                        dependent: nodes[dependent_index].clone(),
                        dependency: dep_key.clone(),
                        required: req.clone(),
                        found: versions[dep_index].clone(),
                    },
                )));
            }
            edges.push(dep_index);
        }
        adjacency.push(edges);
    }

    // Iterative white/grey/black DFS over node indices. `grey_position` maps a
    // grey node to its slot in `grey_stack` (`NOT_GREY` when not on the path),
    // so the cycle entry is located without a fallible search.
    const NOT_GREY: usize = usize::MAX;
    let mut color = vec![Color::White; n];
    let mut grey_stack: Vec<usize> = Vec::new();
    let mut grey_position = vec![NOT_GREY; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);

    // Each frame: (node, index of the next outgoing edge to visit).
    let mut call_stack: Vec<(usize, usize)> = Vec::new();

    for start in 0..n {
        if color[start] != Color::White {
            continue;
        }
        color[start] = Color::Grey;
        grey_position[start] = grey_stack.len();
        grey_stack.push(start);
        call_stack.push((start, 0));

        // Drive the iterative DFS until the current connected component is done.
        while let Some(&(node, cursor)) = call_stack.last() {
            if let Some(&child) = adjacency[node].get(cursor) {
                if let Some(frame) = call_stack.last_mut() {
                    frame.1 += 1;
                }
                match color[child] {
                    Color::Grey => {
                        // Back-edge: the cycle is the grey path from the entry
                        // node down to here, closed by repeating the entry node
                        // (e.g. [a, b, c, a]).
                        let entry = grey_position[child];
                        let mut path: Vec<PluginKey> = grey_stack[entry..]
                            .iter()
                            .map(|&i| nodes[i].clone())
                            .collect();
                        path.push(nodes[child].clone());
                        return Err(PluginDependencyError::Cycle { path });
                    },
                    Color::Black => {
                        // Already fully processed — no further work needed.
                    },
                    Color::White => {
                        color[child] = Color::Grey;
                        grey_position[child] = grey_stack.len();
                        grey_stack.push(child);
                        call_stack.push((child, 0));
                    },
                }
            } else {
                // All outgoing edges processed — finalize this node.
                call_stack.pop();
                color[node] = Color::Black;
                grey_position[node] = NOT_GREY;
                grey_stack.pop();
                order.push(node);
            }
        }
    }

    let result: Vec<PluginKey> = order.iter().map(|&i| nodes[i].clone()).collect();

    // Topo-order invariant (debug builds only): every dependency precedes its
    // dependent in the output.
    debug_assert!(
        is_topologically_sorted(&adjacency, &order),
        "topological invariant violated: a dependency appears after its dependent in the output"
    );

    Ok(result)
}

/// Debug-only check that `order` (a permutation of the node indices `0..n`)
/// places every dependency before its dependent for all edges in `adjacency`.
fn is_topologically_sorted(adjacency: &[Vec<usize>], order: &[usize]) -> bool {
    let mut output_position = vec![0usize; order.len()];
    for (position, &node) in order.iter().enumerate() {
        output_position[node] = position;
    }
    adjacency.iter().enumerate().all(|(node, deps)| {
        deps.iter()
            .all(|&dep| output_position[dep] < output_position[node])
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

    // ── Error precedence (deterministic; pins the resolution-phase ordering) ─────

    #[test]
    fn missing_dependency_precedes_cycle() {
        // The graph contains BOTH a 2-cycle (a <-> b) and a node `x` whose
        // dependency `ghost` is not registered. Unsatisfiable declared
        // dependencies are detected while building the edge graph, before any
        // cycle check runs, so `MissingDependency` is reported — never `Cycle`.
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[("b", "^1")]),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^1")]),
            with_deps("x", Version::new(1, 0, 0), &[("ghost", "^1")]),
        ]);
        let err = resolve(&r).unwrap_err();
        assert!(
            matches!(
                &err,
                PluginDependencyError::MissingDependency { dependency, .. }
                if dependency.as_str() == "ghost"
            ),
            "an unsatisfiable dependency must win over the cycle, got: {err}"
        );
    }

    #[test]
    fn version_mismatch_precedes_cycle() {
        // The graph contains BOTH a 2-cycle (a <-> b) and a node `x` requiring a
        // version of `a` that is not satisfied. The mismatch is detected during
        // edge resolution, before cycle detection, so `VersionMismatch` is
        // reported — never `Cycle`.
        let r = reg(vec![
            with_deps("a", Version::new(1, 0, 0), &[("b", "^1")]),
            with_deps("b", Version::new(1, 0, 0), &[("a", "^1")]),
            with_deps("x", Version::new(1, 0, 0), &[("a", "^2")]),
        ]);
        let err = resolve(&r).unwrap_err();
        match &err {
            PluginDependencyError::VersionMismatch(detail) => {
                assert_eq!(detail.dependent.as_str(), "x");
                assert_eq!(detail.dependency.as_str(), "a");
            },
            _ => panic!("a version mismatch must win over the cycle, got: {err}"),
        }
    }

    #[test]
    fn first_error_is_deterministic_by_dependent_key() {
        // Two independent unsatisfiable dependencies: `m` requires a missing
        // `ghost`, and `z` requires `a` at an unsatisfiable version. The edge
        // graph is built in ascending dependent-key order, so the lower-keyed
        // dependent (`m`) is always reported first — regardless of registration
        // order, and regardless of which error kind each carries.
        let plugins = || {
            vec![
                versioned("a", Version::new(1, 0, 0)),
                with_deps("m", Version::new(1, 0, 0), &[("ghost", "^1")]),
                with_deps("z", Version::new(1, 0, 0), &[("a", "^2")]),
            ]
        };
        let forward = resolve(&reg(plugins())).unwrap_err();
        let mut reversed = plugins();
        reversed.reverse();
        let backward = resolve(&reg(reversed)).unwrap_err();

        for err in [&forward, &backward] {
            assert!(
                matches!(
                    err,
                    PluginDependencyError::MissingDependency { dependent, .. }
                    if dependent.as_str() == "m"
                ),
                "lowest-keyed dependent 'm' must be reported first, got: {err}"
            );
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
