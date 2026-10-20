//! Relative links in tracked Markdown files must name a file or directory that exists.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocLinksError {
    #[error("cannot list tracked Markdown files: {0}")]
    GitStart(std::io::Error),
    #[error("`git ls-files` failed: {0}")]
    GitFailed(String),
    #[error("cannot read `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{count} broken relative link(s) in Markdown:\n{report}")]
    Broken { count: usize, report: String },
}

pub(crate) fn check(cwd: &Path) -> Result<(), crate::XtaskError> {
    let root = crate::workspace::find_root(cwd)?;
    let output = Command::new("git")
        .current_dir(&root)
        .args(["ls-files", "-z", "--", "*.md"])
        .output()
        .map_err(DocLinksError::GitStart)?;
    if !output.status.success() {
        return Err(
            DocLinksError::GitFailed(String::from_utf8_lossy(&output.stderr).into_owned()).into(),
        );
    }
    let mut broken = Vec::new();
    for file in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let file = PathBuf::from(String::from_utf8_lossy(file).into_owned());
        let path = root.join(&file);
        let source = std::fs::read_to_string(&path).map_err(|source| DocLinksError::Read {
            path: path.clone(),
            source,
        })?;
        let directory = path.parent().unwrap_or(&root);
        for target in relative_targets(&source) {
            let resolved = match target.strip_prefix('/') {
                Some(from_root) => root.join(from_root),
                None => directory.join(target),
            };
            if !resolved.exists() {
                broken.push(format!("  {}: {target}", file.display()));
            }
        }
    }
    if broken.is_empty() {
        return Ok(());
    }
    Err(DocLinksError::Broken {
        count: broken.len(),
        report: broken.join("\n"),
    }
    .into())
}

/// Path parts of inline `[text](target)` links and `[label]: target`
/// definitions, outside code. URLs and same-page anchors are skipped.
fn relative_targets(source: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut fence: Option<&str> = None;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = ["```", "~~~"]
            .into_iter()
            .find(|marker| trimmed.starts_with(marker))
        {
            fence = Some(marker);
            continue;
        }
        let prose = outside_code_spans(line);
        for segment in &prose {
            let mut rest = *segment;
            while let Some(start) = rest.find("](") {
                rest = &rest[start + 2..];
                if let Some(end) = rest.find(')') {
                    targets.push(&rest[..end]);
                    rest = &rest[end..];
                }
            }
        }
        if let Some(definition) = trimmed.strip_prefix('[')
            && let Some((_, target)) = definition.split_once("]: ")
            && !definition.starts_with('^')
        {
            targets.push(target);
        }
    }
    targets.into_iter().filter_map(path_part).collect()
}

fn outside_code_spans(line: &str) -> Vec<&str> {
    line.split('`').step_by(2).collect()
}

fn path_part(target: &str) -> Option<&str> {
    let target = target.trim();
    let target = target.split_once(' ').map_or(target, |(path, _title)| path);
    let target = target.trim_start_matches('<').trim_end_matches('>');
    let path = target
        .split_once('#')
        .map_or(target, |(path, _anchor)| path);
    let has_scheme = path.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic())
    });
    (!path.is_empty() && !has_scheme).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::relative_targets;

    #[test]
    fn collects_relative_link_paths_without_anchors_or_titles() {
        let source = "See [a](docs/A.md#part) and [b](../b.md \"title\").\n[ref]: ./c.md\n";
        assert_eq!(relative_targets(source), ["docs/A.md", "../b.md", "./c.md"]);
    }

    #[test]
    fn skips_urls_anchors_and_code() {
        let source = "[x](https://e.com) [y](#here) [z](mailto:a@b) `[c](gone.md)`\n\
                      ```\n[d](gone.md)\n```\n[^note]: text\n";
        assert!(relative_targets(source).is_empty());
    }
}
