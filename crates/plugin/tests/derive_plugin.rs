//! Integration tests for the `#[derive(Plugin)]` macro.

use nebula_plugin::{Plugin, PluginManifest};
use semver::{BuildMetadata, Prerelease, Version};

#[derive(Debug, Plugin)]
#[plugin(key = "simple", name = "Simple", version = "2.3.4")]
struct SimplePlugin;

#[derive(Debug, Plugin)]
#[plugin(key = "pre", name = "Pre-release", version = "1.0.0-alpha.1+build.42")]
struct PreReleasePlugin;

#[derive(Debug, Plugin)]
#[plugin(key = "defaulted", name = "Defaulted")]
struct DefaultedPlugin;

#[derive(Debug, Plugin)]
#[plugin(
    key = "grouped",
    name = "Grouped",
    description = "A plugin with groups",
    version = "0.5.0",
    group = ["network", "api"]
)]
struct GroupedPlugin;

#[test]
fn plain_semver_roundtrips() {
    let manifest: &PluginManifest = SimplePlugin.manifest();
    assert_eq!(manifest.key().as_str(), "simple");
    assert_eq!(manifest.name(), "Simple");
    assert_eq!(manifest.version(), &Version::new(2, 3, 4));
}

#[test]
fn prerelease_and_build_metadata_survive() {
    let manifest: &PluginManifest = PreReleasePlugin.manifest();
    let version = manifest.version();

    assert_eq!(version.major, 1);
    assert_eq!(version.minor, 0);
    assert_eq!(version.patch, 0);
    assert_eq!(
        version.pre,
        Prerelease::new("alpha.1").expect("valid pre-release")
    );
    assert_eq!(
        version.build,
        BuildMetadata::new("build.42").expect("valid build metadata")
    );
}

#[test]
fn default_version_is_1_0_0() {
    let manifest: &PluginManifest = DefaultedPlugin.manifest();
    assert_eq!(manifest.version(), &Version::new(1, 0, 0));
}

#[test]
fn group_and_description_propagate() {
    let manifest: &PluginManifest = GroupedPlugin.manifest();
    assert_eq!(manifest.group(), &["network", "api"]);
    assert_eq!(manifest.description(), "A plugin with groups");
    assert_eq!(manifest.version(), &Version::new(0, 5, 0));
}
