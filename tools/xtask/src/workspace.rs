use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

use cargo_metadata::{CargoOpt, MetadataCommand, PackageId};
use serde::Deserialize;

use crate::{XtaskError, model::PlanEntry};

#[derive(Debug)]
pub(crate) struct Workspace {
    root: PathBuf,
    members: BTreeSet<PackageId>,
    packages: BTreeMap<PackageId, PackageInfo>,
    reverse_dependencies: BTreeMap<PackageId, BTreeSet<PackageId>>,
}

#[derive(Debug)]
struct PackageInfo {
    name: String,
    manifest_directory: PathBuf,
    test_features: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Owner<'a> {
    Package(&'a PackageId),
    Ambiguous,
    None,
}

impl Workspace {
    pub(crate) fn load(cwd: &Path) -> Result<Self, XtaskError> {
        let mut command = MetadataCommand::new();
        command
            .current_dir(cwd)
            .features(CargoOpt::AllFeatures)
            .other_options(vec!["--locked".to_owned()]);
        let metadata = command.exec()?;
        let root = metadata.workspace_root.clone().into_std_path_buf();
        let members = metadata
            .workspace_members
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut packages = BTreeMap::new();
        let mut package_names = BTreeSet::new();

        for member_id in &members {
            let package = metadata
                .packages
                .iter()
                .find(|package| &package.id == member_id)
                .ok_or_else(|| XtaskError::MissingWorkspacePackage(member_id.to_string()))?;
            let name = package.name.to_string();
            if !package_names.insert(name.clone()) {
                return Err(XtaskError::DuplicatePackageName(name));
            }
            let manifest_directory = package
                .manifest_path
                .parent()
                .ok_or_else(|| XtaskError::ManifestOutsideWorkspace {
                    manifest: package.manifest_path.clone().into_std_path_buf(),
                    root: root.clone(),
                })?
                .strip_prefix(&metadata.workspace_root)
                .map_err(|_| XtaskError::ManifestOutsideWorkspace {
                    manifest: package.manifest_path.clone().into_std_path_buf(),
                    root: root.clone(),
                })?
                .as_std_path()
                .to_path_buf();
            let test_features = test_features(package)?;
            packages.insert(
                member_id.clone(),
                PackageInfo {
                    name,
                    manifest_directory,
                    test_features,
                },
            );
        }

        let resolve = metadata.resolve.ok_or(XtaskError::MissingResolve)?;
        let mut reverse_dependencies = members
            .iter()
            .cloned()
            .map(|id| (id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for node in resolve.nodes {
            if !members.contains(&node.id) {
                continue;
            }
            for dependency in node.deps {
                if dependency.pkg == node.id || !members.contains(&dependency.pkg) {
                    continue;
                }
                if let Some(dependents) = reverse_dependencies.get_mut(&dependency.pkg) {
                    dependents.insert(node.id.clone());
                }
            }
        }

        Ok(Self {
            root,
            members,
            packages,
            reverse_dependencies,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn all_entries(&self) -> Result<Vec<PlanEntry>, XtaskError> {
        self.entries(self.members.iter().cloned())
    }

    pub(crate) fn entries(
        &self,
        roots: impl IntoIterator<Item = PackageId>,
    ) -> Result<Vec<PlanEntry>, XtaskError> {
        let ids = self.reverse_closure(roots);
        let mut entries = ids
            .iter()
            .map(|id| {
                self.packages
                    .get(id)
                    .map(|package| PlanEntry {
                        package: package.name.clone(),
                        test_features: package.test_features.clone(),
                    })
                    .ok_or_else(|| XtaskError::MissingWorkspacePackage(id.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by(|left, right| left.package.cmp(&right.package));
        entries.dedup_by(|left, right| left.package == right.package);
        Ok(entries)
    }

    pub(crate) fn owner(&self, path: &Path) -> Owner<'_> {
        let mut best: Option<(&PackageId, usize)> = None;
        let mut ambiguous = false;
        for (id, package) in &self.packages {
            if !path.starts_with(&package.manifest_directory) {
                continue;
            }
            let depth = package.manifest_directory.components().count();
            match best {
                None => {
                    best = Some((id, depth));
                    ambiguous = false;
                },
                Some((_, best_depth)) if depth > best_depth => {
                    best = Some((id, depth));
                    ambiguous = false;
                },
                Some((best_id, best_depth)) if depth == best_depth && best_id != id => {
                    ambiguous = true;
                },
                Some(_) => {},
            }
        }
        match (best, ambiguous) {
            (_, true) => Owner::Ambiguous,
            (Some((id, _)), false) => Owner::Package(id),
            (None, false) => Owner::None,
        }
    }

    fn reverse_closure(&self, roots: impl IntoIterator<Item = PackageId>) -> BTreeSet<PackageId> {
        reverse_closure(&self.reverse_dependencies, roots)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
struct CiMetadata {
    test_features: Vec<String>,
}

fn test_features(package: &cargo_metadata::Package) -> Result<Vec<String>, XtaskError> {
    let Some(value) = package.metadata.pointer("/nebula/ci") else {
        return Ok(Vec::new());
    };
    let policy = serde_json::from_value::<CiMetadata>(value.clone()).map_err(|error| {
        XtaskError::InvalidCiMetadata {
            package: package.name.to_string(),
            detail: error.to_string(),
        }
    })?;
    let features = policy.test_features.into_iter().collect::<BTreeSet<_>>();
    for feature in &features {
        if !package.features.contains_key(feature) {
            return Err(XtaskError::UnknownTestFeature {
                package: package.name.to_string(),
                feature: feature.clone(),
            });
        }
    }
    Ok(features.into_iter().collect())
}

fn reverse_closure<T>(
    reverse_dependencies: &BTreeMap<T, BTreeSet<T>>,
    roots: impl IntoIterator<Item = T>,
) -> BTreeSet<T>
where
    T: Clone + Ord,
{
    let mut selected = BTreeSet::new();
    let mut queue = roots.into_iter().collect::<VecDeque<_>>();
    while let Some(id) = queue.pop_front() {
        if !selected.insert(id.clone()) {
            continue;
        }
        if let Some(dependents) = reverse_dependencies.get(&id) {
            queue.extend(dependents.iter().cloned());
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::reverse_closure;

    #[test]
    fn reverse_closure_terminates_on_cycles_and_ignores_self_edges() {
        let graph = BTreeMap::from([
            ("a", BTreeSet::from(["a", "b"])),
            ("b", BTreeSet::from(["c"])),
            ("c", BTreeSet::from(["a"])),
        ]);

        assert_eq!(
            reverse_closure(&graph, ["a"]),
            BTreeSet::from(["a", "b", "c"])
        );
    }
}
