//! `cargo xtask layer-map`: keeps the `AGENTS.md` layer map, the `deny.toml`
//! layer wrappers and the real dependency graph in step.
//!
//! `cargo deny check` only enforces the wrappers it is given: a crate with no
//! `wrappers` entry may be depended on by anything, and a wrapper list that
//! names a lower layer turns an upward dependency into an allowed one. This
//! check closes both gaps:
//!
//! - every product crate (a package under `crates/`) sits in a layer of the
//!   map, a `-macros` crate in its parent's;
//! - a crate with workspace dependents has a `wrappers` entry, and every name
//!   in it is a current dependent, so the list is the declared set of
//!   dependents rather than a stale allow-list;
//! - no product crate depends on a crate in a higher layer, except as a
//!   dev-dependency (tests may wire a higher layer).

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
};

use cargo_metadata::{DependencyKind, MetadataCommand};
use thiserror::Error;

/// A dependency of one workspace package on another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edge {
    pub(crate) dependent: String,
    pub(crate) dependency: String,
    /// Every declaration of this dependency is a dev-dependency.
    pub(crate) dev_only: bool,
}

/// Everything the check reads, already parsed.
#[derive(Debug, Default)]
pub(crate) struct Inputs {
    /// Workspace package name → whether it is a product crate.
    pub(crate) packages: BTreeMap<String, bool>,
    pub(crate) edges: Vec<Edge>,
    /// Layer name → short crate names (without `nebula-`), lowest layer first.
    pub(crate) layers: Vec<(String, BTreeSet<String>)>,
    /// `deny.toml` wrapped crate → its `wrappers`.
    pub(crate) wrappers: BTreeMap<String, BTreeSet<String>>,
}

/// One way the map, the wrappers and the graph disagree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violation {
    Unmapped {
        package: String,
    },
    UnknownLayerCrate {
        name: String,
    },
    Upward {
        dependent: String,
        dependent_layer: String,
        dependency: String,
        dependency_layer: String,
    },
    MissingWrappers {
        package: String,
        dependents: Vec<String>,
    },
    UnknownWrappedCrate {
        package: String,
    },
    StaleWrapper {
        package: String,
        wrapper: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unmapped { package } => write!(
                f,
                "`{package}` is not in the AGENTS.md layer map; add it to its layer"
            ),
            Self::UnknownLayerCrate { name } => write!(
                f,
                "the AGENTS.md layer map names `{name}`, which is not a workspace crate"
            ),
            Self::Upward {
                dependent,
                dependent_layer,
                dependency,
                dependency_layer,
            } => write!(
                f,
                "`{dependent}` ({dependent_layer}) depends on `{dependency}` \
                 ({dependency_layer}), a higher layer"
            ),
            Self::MissingWrappers {
                package,
                dependents,
            } => write!(
                f,
                "`{package}` has no deny.toml `wrappers` entry; its dependents are {}",
                dependents.join(", ")
            ),
            Self::UnknownWrappedCrate { package } => write!(
                f,
                "deny.toml wraps `{package}`, which is not a workspace crate"
            ),
            Self::StaleWrapper { package, wrapper } => write!(
                f,
                "deny.toml lists `{wrapper}` as a wrapper of `{package}`, but it no longer \
                 depends on it"
            ),
        }
    }
}

#[derive(Debug, Error)]
pub enum LayerMapError {
    #[error("cannot read `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`{path}` is invalid TOML: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("deny.toml: {0}")]
    Deny(String),
    #[error("AGENTS.md has no layer table under `## Layered Dependency Map`")]
    NoLayerTable,
    #[error("cargo metadata failed: {0}")]
    Metadata(#[from] cargo_metadata::Error),
    #[error("layer map out of step with the dependency graph:\n{}", list(.0))]
    Violations(Vec<Violation>),
}

fn list(violations: &[Violation]) -> String {
    violations
        .iter()
        .map(|violation| format!("  - {violation}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Load the workspace at `root` and check it.
pub(crate) fn check_workspace(root: &Path) -> Result<(), LayerMapError> {
    let inputs = load(root)?;
    let violations = check(&inputs);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(LayerMapError::Violations(violations))
    }
}

fn load(root: &Path) -> Result<Inputs, LayerMapError> {
    let metadata = MetadataCommand::new().current_dir(root).no_deps().exec()?;
    let crates = metadata.workspace_root.join("crates");
    let packages: BTreeMap<String, bool> = metadata
        .packages
        .iter()
        .map(|package| {
            (
                package.name.to_string(),
                package.manifest_path.starts_with(&crates),
            )
        })
        .collect();
    let mut kinds: BTreeMap<(String, String), bool> = BTreeMap::new();
    for package in &metadata.packages {
        for dependency in &package.dependencies {
            if !packages.contains_key(dependency.name.as_str()) {
                continue;
            }
            let dev = dependency.kind == DependencyKind::Development;
            kinds
                .entry((package.name.to_string(), dependency.name.clone()))
                .and_modify(|dev_only| *dev_only &= dev)
                .or_insert(dev);
        }
    }
    let edges = kinds
        .into_iter()
        .map(|((dependent, dependency), dev_only)| Edge {
            dependent,
            dependency,
            dev_only,
        })
        .collect();
    Ok(Inputs {
        packages,
        edges,
        layers: parse_layers(&read(&root.join("AGENTS.md"))?)?,
        wrappers: parse_wrappers(&root.join("deny.toml"))?,
    })
}

fn read(path: &Path) -> Result<String, LayerMapError> {
    std::fs::read_to_string(path).map_err(|source| LayerMapError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// The layer table under `## Layered Dependency Map`, lowest layer first.
///
/// Rows read `| **Layer** | `a`, `b` |`, highest layer first.
pub(crate) fn parse_layers(agents: &str) -> Result<Vec<(String, BTreeSet<String>)>, LayerMapError> {
    let section = agents
        .split_once("## Layered Dependency Map")
        .map(|(_, rest)| rest)
        .ok_or(LayerMapError::NoLayerTable)?;
    let section = section.split("\n## ").next().unwrap_or(section);
    let mut layers = Vec::new();
    for line in section.lines() {
        let mut cells = line.trim().strip_prefix('|').unwrap_or("").split('|');
        let (Some(name), Some(crates)) = (cells.next(), cells.next()) else {
            continue;
        };
        let Some(name) = name
            .trim()
            .strip_prefix("**")
            .and_then(|n| n.strip_suffix("**"))
        else {
            continue;
        };
        let names = crates
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        layers.push((name.to_owned(), names));
    }
    if layers.is_empty() {
        return Err(LayerMapError::NoLayerTable);
    }
    layers.reverse();
    Ok(layers)
}

fn parse_wrappers(path: &Path) -> Result<BTreeMap<String, BTreeSet<String>>, LayerMapError> {
    let deny: toml::Table = toml::from_str(&read(path)?).map_err(|source| LayerMapError::Toml {
        path: path.to_path_buf(),
        source,
    })?;
    let entries = deny
        .get("bans")
        .and_then(|bans| bans.get("deny"))
        .and_then(toml::Value::as_array)
        .ok_or_else(|| LayerMapError::Deny("missing `[bans].deny`".to_owned()))?;
    let mut wrappers = BTreeMap::new();
    for entry in entries {
        let Some(list) = entry.get("wrappers").and_then(toml::Value::as_array) else {
            continue;
        };
        let name = entry
            .get("crate")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| LayerMapError::Deny("a `wrappers` entry has no `crate`".to_owned()))?;
        let list = list
            .iter()
            .map(|wrapper| {
                wrapper.as_str().map(str::to_owned).ok_or_else(|| {
                    LayerMapError::Deny(format!("`{name}` has a non-string wrapper"))
                })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if wrappers.insert(name.to_owned(), list).is_some() {
            return Err(LayerMapError::Deny(format!("`{name}` is wrapped twice")));
        }
    }
    Ok(wrappers)
}

/// The short name a crate has in the layer map: `nebula-` dropped, and a
/// `-macros` crate named after its parent.
fn short_name(package: &str) -> &str {
    let short = package.strip_prefix("nebula-").unwrap_or(package);
    short.strip_suffix("-macros").unwrap_or(short)
}

pub(crate) fn check(inputs: &Inputs) -> Vec<Violation> {
    let mut violations = BTreeSet::new();
    let layer_of = |package: &str| {
        inputs
            .layers
            .iter()
            .position(|(_, names)| names.contains(short_name(package)))
    };
    let product = |package: &str| inputs.packages.get(package).copied().unwrap_or(false);

    let shorts: BTreeSet<&str> = inputs
        .packages
        .keys()
        .map(|name| short_name(name))
        .collect();
    for (_, names) in &inputs.layers {
        for name in names {
            if !shorts.contains(name.as_str()) {
                violations.insert(Violation::UnknownLayerCrate { name: name.clone() });
            }
        }
    }
    for (package, is_product) in &inputs.packages {
        if *is_product && layer_of(package).is_none() {
            violations.insert(Violation::Unmapped {
                package: package.clone(),
            });
        }
    }

    let mut dependents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in &inputs.edges {
        dependents
            .entry(edge.dependency.as_str())
            .or_default()
            .insert(edge.dependent.as_str());
        if edge.dev_only || !product(&edge.dependent) || !product(&edge.dependency) {
            continue;
        }
        if let (Some(from), Some(to)) = (layer_of(&edge.dependent), layer_of(&edge.dependency))
            && from < to
        {
            violations.insert(Violation::Upward {
                dependent: edge.dependent.clone(),
                dependent_layer: inputs.layers[from].0.clone(),
                dependency: edge.dependency.clone(),
                dependency_layer: inputs.layers[to].0.clone(),
            });
        }
    }

    for (package, of) in &dependents {
        if product(package) && !inputs.wrappers.contains_key(*package) {
            violations.insert(Violation::MissingWrappers {
                package: (*package).to_owned(),
                dependents: of.iter().map(|name| (*name).to_owned()).collect(),
            });
        }
    }
    for (package, wrappers) in &inputs.wrappers {
        if !inputs.packages.contains_key(package) {
            violations.insert(Violation::UnknownWrappedCrate {
                package: package.clone(),
            });
            continue;
        }
        let actual = dependents.get(package.as_str());
        for wrapper in wrappers {
            if !actual.is_some_and(|of| of.contains(wrapper.as_str())) {
                violations.insert(Violation::StaleWrapper {
                    package: package.clone(),
                    wrapper: wrapper.clone(),
                });
            }
        }
    }
    violations.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENTS: &str = "\
# Guide

## Layered Dependency Map

| Layer | Crates |
|-------|--------|
| **API** | `api` |
| **Core** | `core`, `schema` |
| **Cross-cutting** | `error` |

## Next section

| **Not** | `a-layer` |
";

    fn inputs(edges: &[(&str, &str, bool)], wrappers: &[(&str, &[&str])]) -> Inputs {
        Inputs {
            packages: [
                ("nebula-api", true),
                ("nebula-core", true),
                ("nebula-schema", true),
                ("nebula-schema-macros", true),
                ("nebula-error", true),
                ("nebula-server", false),
            ]
            .into_iter()
            .map(|(name, product)| (name.to_owned(), product))
            .collect(),
            edges: edges
                .iter()
                .map(|(dependent, dependency, dev_only)| Edge {
                    dependent: (*dependent).to_owned(),
                    dependency: (*dependency).to_owned(),
                    dev_only: *dev_only,
                })
                .collect(),
            layers: parse_layers(AGENTS).unwrap(),
            wrappers: wrappers
                .iter()
                .map(|(name, list)| {
                    (
                        (*name).to_owned(),
                        list.iter().map(|wrapper| (*wrapper).to_owned()).collect(),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn the_layer_table_is_read_lowest_first_and_stops_at_its_section() {
        let layers = parse_layers(AGENTS).unwrap();
        let names: Vec<_> = layers.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["Cross-cutting", "Core", "API"]);
        assert!(layers[1].1.contains("schema"));
    }

    #[test]
    fn a_consistent_graph_passes() {
        let inputs = inputs(
            &[
                ("nebula-schema", "nebula-core", false),
                ("nebula-schema", "nebula-schema-macros", false),
                ("nebula-api", "nebula-schema", false),
                ("nebula-server", "nebula-api", false),
            ],
            &[
                ("nebula-core", &["nebula-schema"]),
                ("nebula-schema-macros", &["nebula-schema"]),
                ("nebula-schema", &["nebula-api"]),
                ("nebula-api", &["nebula-server"]),
            ],
        );
        assert_eq!(check(&inputs), []);
    }

    #[test]
    fn an_upward_dependency_fails_unless_it_is_dev_only() {
        let wrappers: &[(&str, &[&str])] = &[("nebula-api", &["nebula-core"])];
        assert_eq!(
            check(&inputs(&[("nebula-core", "nebula-api", false)], wrappers)),
            [Violation::Upward {
                dependent: "nebula-core".into(),
                dependent_layer: "Core".into(),
                dependency: "nebula-api".into(),
                dependency_layer: "API".into(),
            }]
        );
        assert_eq!(
            check(&inputs(&[("nebula-core", "nebula-api", true)], wrappers)),
            []
        );
    }

    #[test]
    fn a_depended_on_crate_needs_wrappers_and_they_must_be_current() {
        assert_eq!(
            check(&inputs(
                &[("nebula-core", "nebula-error", false)],
                &[("nebula-api", &["nebula-server"])],
            )),
            [
                Violation::MissingWrappers {
                    package: "nebula-error".into(),
                    dependents: vec!["nebula-core".into()],
                },
                Violation::StaleWrapper {
                    package: "nebula-api".into(),
                    wrapper: "nebula-server".into(),
                },
            ]
        );
    }

    #[test]
    fn unmapped_crates_and_unknown_names_fail() {
        let mut inputs = inputs(&[], &[("nebula-gone", &[])]);
        inputs.packages.insert("nebula-new".into(), true);
        inputs.layers[0].1.insert("ghost".into());
        assert_eq!(
            check(&inputs),
            [
                Violation::Unmapped {
                    package: "nebula-new".into()
                },
                Violation::UnknownLayerCrate {
                    name: "ghost".into()
                },
                Violation::UnknownWrappedCrate {
                    package: "nebula-gone".into()
                },
            ]
        );
    }
}
