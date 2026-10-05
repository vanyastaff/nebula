//! Disposable parent regression for hook-local Git repository environment.

use super::fixture_process::fixture_command;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Exercise real fixture operations in a child with poisoned repository selection.
/// Every foreign path belongs to a disposable parent, never the developer checkout.
pub(crate) fn assert_hook_repository_isolation(test_name: &str, exercise: impl FnOnce()) {
    const CHILD_TEST: &str = "NEBULA_XTASK_GIT_ENV_CHILD_TEST";
    if std::env::var(CHILD_TEST).as_deref() == Ok(test_name) {
        exercise();
        return;
    }

    let parent = tempfile::tempdir().expect("create disposable hook parent");
    parent_git(parent.path(), &["init", "-q", "-b", "main"]);
    parent_git(
        parent.path(),
        &["config", "user.email", "hook-parent@example.invalid"],
    );
    parent_git(
        parent.path(),
        &["config", "user.name", "Disposable Hook Parent"],
    );
    fs::write(parent.path().join("parent.txt"), "parent canary\n").expect("write parent canary");
    parent_git(parent.path(), &["add", "."]);
    parent_git(
        parent.path(),
        &["commit", "-qm", "disposable parent baseline"],
    );
    let git_dir = parent.path().join(".git");
    let before = parent_snapshot(&git_dir);

    // Start from a cleared child environment before deliberately adding foreign
    // pointers. Even the negative control cannot inherit a real checkout's paths.
    let child = fixture_command(std::env::current_exe().expect("locate integration test binary"))
        .args([test_name, "--exact", "--nocapture"])
        .env(CHILD_TEST, test_name)
        .env("GIT_DIR", &git_dir)
        .env("GIT_COMMON_DIR", &git_dir)
        .env("GIT_WORK_TREE", parent.path())
        .env("GIT_INDEX_FILE", git_dir.join("index"))
        .env("GIT_OBJECT_DIRECTORY", git_dir.join("objects"))
        .env("GIT_CONFIG", git_dir.join("config"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.name")
        .env("GIT_CONFIG_VALUE_0", "Injected Hook Identity")
        .output()
        .expect("run fixture operations under disposable hook environment");

    let after = parent_snapshot(&git_dir);
    assert!(
        before == after,
        "fixture operations changed disposable parent HEAD/config/refs/index/objects:\n{}\n\
         child status: {}\nchild stdout:\n{}\nchild stderr:\n{}",
        snapshot_difference(&before, &after),
        child.status,
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(
        child.status.success(),
        "isolated fixture operations failed: {}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}

fn parent_git(parent: &Path, arguments: &[&str]) {
    let output = fixture_command("git")
        .current_dir(parent)
        .args(arguments)
        .output()
        .expect("run disposable parent Git command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parent_snapshot(git_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for relative in ["HEAD", "config", "index", "refs", "objects", "packed-refs"] {
        let path = git_dir.join(relative);
        if path.exists() {
            snapshot_path(git_dir, &path, &mut files);
        }
    }
    files
}

/// Names every added, removed, or changed parent file so a leak is diagnosable
/// from CI logs; small text files (HEAD, config, refs) also show both sides.
fn snapshot_difference(
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> String {
    let text = |bytes: &[u8]| {
        if bytes.len() <= 512 && std::str::from_utf8(bytes).is_ok() {
            format!("{:?}", String::from_utf8_lossy(bytes))
        } else {
            format!("<{} bytes>", bytes.len())
        }
    };
    let mut lines = Vec::new();
    for (path, old) in before {
        match after.get(path) {
            None => lines.push(format!("  removed {}", path.display())),
            Some(new) if new != old => lines.push(format!(
                "  changed {}: {} -> {}",
                path.display(),
                text(old),
                text(new)
            )),
            Some(_) => {},
        }
    }
    for (path, new) in after {
        if !before.contains_key(path) {
            lines.push(format!("  added {}: {}", path.display(), text(new)));
        }
    }
    lines.join("\n")
}

fn snapshot_path(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("read disposable parent directory") {
            snapshot_path(root, &entry.expect("read parent entry").path(), files);
        }
    } else {
        files.insert(
            path.strip_prefix(root)
                .expect("snapshot stays in disposable parent")
                .to_path_buf(),
            fs::read(path).expect("read disposable parent bytes"),
        );
    }
}
