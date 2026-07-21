use std::{path::Path, process::Command};

use crate::XtaskError;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Comparison {
    MergeBase,
    Direct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathRole {
    Single,
    Old,
    New,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ChangedPath {
    pub(crate) kind: ChangeKind,
    pub(crate) role: PathRole,
    pub(crate) path: String,
}

#[derive(Debug)]
pub(crate) struct Changes {
    pub(crate) paths: Vec<ChangedPath>,
    pub(crate) conservative_reason: Option<String>,
}

pub(crate) fn git_diff(
    workspace_root: &Path,
    base: &str,
    head: &str,
    comparison: Comparison,
) -> Result<Changes, XtaskError> {
    if base.starts_with('-') || head.starts_with('-') {
        return Err(XtaskError::GitFailed(
            "revision names must not start with `-`".to_owned(),
        ));
    }
    let mut command = Command::new("git");
    command.current_dir(workspace_root).args([
        "diff",
        "--name-status",
        "-z",
        "-M",
        "-C",
        "--find-copies-harder",
    ]);
    match comparison {
        Comparison::MergeBase => {
            command.arg(format!("{base}...{head}"));
        },
        Comparison::Direct => {
            command.args([base, head]);
        },
    }
    let output = command.arg("--").output().map_err(XtaskError::GitIo)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(XtaskError::GitFailed(if stderr.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            stderr
        }));
    }
    parse_name_status(&output.stdout)
}

fn parse_name_status(output: &[u8]) -> Result<Changes, XtaskError> {
    let mut tokens = output.split(|byte| *byte == 0).peekable();
    let mut paths = Vec::new();
    let mut conservative_reason = None;

    while let Some(status_bytes) = tokens.next() {
        if status_bytes.is_empty() {
            if tokens.peek().is_none() {
                break;
            }
            return Err(XtaskError::InvalidGitOutput(
                "empty status token".to_owned(),
            ));
        }
        let status = std::str::from_utf8(status_bytes)
            .map_err(|_| XtaskError::InvalidGitOutput("non-UTF-8 status".to_owned()))?;
        let code = status.as_bytes()[0];
        let kind = match code {
            b'A' => ChangeKind::Added,
            b'M' => ChangeKind::Modified,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            _ => ChangeKind::Other,
        };
        let path_count = if matches!(kind, ChangeKind::Renamed | ChangeKind::Copied) {
            2
        } else {
            1
        };
        for index in 0..path_count {
            let raw_path = tokens.next().ok_or_else(|| {
                XtaskError::InvalidGitOutput(format!("status `{status}` is missing a path"))
            })?;
            let role = if path_count == 1 {
                PathRole::Single
            } else if index == 0 {
                PathRole::Old
            } else {
                PathRole::New
            };
            match normalize_path(raw_path) {
                Ok(path) => paths.push(ChangedPath { kind, role, path }),
                Err(reason) => {
                    conservative_reason.get_or_insert(reason);
                },
            }
        }
    }

    paths.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| role_rank(left.role).cmp(&role_rank(right.role)))
    });
    paths.dedup_by(|left, right| {
        left.path == right.path && left.role == right.role && left.kind == right.kind
    });
    Ok(Changes {
        paths,
        conservative_reason,
    })
}

fn normalize_path(raw: &[u8]) -> Result<String, String> {
    let path = std::str::from_utf8(raw).map_err(|_| "non-UTF-8-path".to_owned())?;
    if path.contains('\\') {
        return Err("backslash-path".to_owned());
    }
    if path.is_empty()
        || path.starts_with('/')
        || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
    {
        return Err("non-relative-path".to_owned());
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {},
            ".." => return Err("path-traversal".to_owned()),
            value => components.push(value),
        }
    }
    if components.is_empty() {
        return Err("empty-normalized-path".to_owned());
    }
    Ok(components.join("/"))
}

const fn role_rank(role: PathRole) -> u8 {
    match role {
        PathRole::Single => 0,
        PathRole::Old => 1,
        PathRole::New => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::{ChangeKind, PathRole, parse_name_status};

    #[test]
    fn rename_and_copy_include_both_old_and_new_paths() {
        let parsed = parse_name_status(b"R100\0old/a.rs\0new/a.rs\0C075\0old/b.rs\0new/b.rs\0")
            .expect("name-status parses");

        assert_eq!(parsed.paths.len(), 4);
        assert!(parsed.paths.iter().any(|path| {
            path.kind == ChangeKind::Renamed
                && path.role == PathRole::Old
                && path.path == "old/a.rs"
        }));
        assert!(parsed.paths.iter().any(|path| {
            path.kind == ChangeKind::Copied && path.role == PathRole::New && path.path == "new/b.rs"
        }));
    }

    #[test]
    fn absolute_and_traversal_paths_force_conservative_plan() {
        let parsed =
            parse_name_status(b"M\0../outside\0M\0/rooted\0").expect("name-status framing parses");

        assert!(parsed.conservative_reason.is_some());
        assert!(parsed.paths.is_empty());
    }

    #[test]
    fn backslash_path_forces_conservative_plan_without_rewriting() {
        let parsed = parse_name_status(b"M\0crates\\parent\\src\\lib.rs\0")
            .expect("name-status framing parses");

        assert_eq!(
            parsed.conservative_reason.as_deref(),
            Some("backslash-path")
        );
        assert!(parsed.paths.is_empty());
    }
}
