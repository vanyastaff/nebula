//! Quality-gate tasks for the Nebula workspace.
//!
//! Citations for failures are printed on stderr — see `docs/QUALITY_GATES.md`.

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    match cmd.as_str() {
        "check-junior" => check_junior(),
        "check-surface" => check_surface(),
        "check-adr-sync" => check_adr_sync(),
        "quality" => quality(),
        "precommit" => precommit(),
        _ => {
            eprintln!(
                "usage: cargo run -p xtask -- <check-junior|check-surface|check-adr-sync|quality|precommit>"
            );
            ExitCode::from(2)
        },
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask crate must live in <workspace>/xtask")
        .to_path_buf()
}

fn check_junior() -> ExitCode {
    let root = workspace_root();
    let mut violations: Vec<String> = Vec::new();

    for rel in &["crates", "apps", "examples"] {
        let base = root.join(rel);
        if base.is_dir() {
            walk_rs(&base, &root, &mut violations);
        }
    }

    if !violations.is_empty() {
        eprintln!(
            "xtask check-junior: FAILED\n\n\
             Rationale: API Guidelines **C-GOOD-ERR** — public functions should expose concrete, \
             meaningful error types (`Error + Send + Sync`), not opaque `Box<dyn Error>` at crate boundaries.\n\
             https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err\n\n\
             (Arc/`Mutex` / `unsafe` are enforced by Clippy workspace lints + `clippy.toml`; see `docs/QUALITY_GATES.md`.)\n\n\
             Violations:"
        );
        for v in &violations {
            eprintln!("  - {v}");
        }
        return ExitCode::from(1);
    }

    println!("xtask check-junior: ok");
    ExitCode::SUCCESS
}

fn walk_rs(dir: &Path, root: &Path, violations: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        let name = p
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == "target" || name == ".git" {
            continue;
        }
        if p.is_dir() {
            walk_rs(&p, root, violations);
            continue;
        }
        if p.extension() != Some(OsStr::new("rs")) {
            continue;
        }
        let rel = p.strip_prefix(root).unwrap_or(&p);
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        if rel_s.contains("/tests/") {
            continue;
        }
        scan_junior_file(&p, &rel_s, violations);
    }
}

fn scan_junior_file(path: &Path, rel: &str, violations: &mut Vec<String>) {
    let Ok(src) = fs::read_to_string(path) else {
        return;
    };

    // Public `Box<dyn Error>` in signatures — C-GOOD-ERR (may span lines before `{` / `;`).
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        let is_pub_fn = t.starts_with("pub ") && (t.contains(" fn ") || t.contains("async fn"));
        if !is_pub_fn {
            i += 1;
            continue;
        }
        let start_line = i + 1;
        let mut sig = String::new();
        let mut j = i;
        let max_sig_lines = 40;
        loop {
            sig.push_str(lines[j].trim());
            sig.push(' ');
            if lines[j].contains('{') {
                break;
            }
            j += 1;
            if j >= lines.len() || j - i >= max_sig_lines {
                break;
            }
        }

        let dyn_error = sig.contains("dyn Error")
            || sig.contains("dyn std::error::Error")
            || sig.contains("dyn core::error::Error");
        if sig.contains("Box<") && dyn_error {
            violations.push(format!(
                "{rel}:{start_line}: public function uses `Box<dyn Error>` — prefer concrete error type per \
                 https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err",
            ));
        }
        i = j + 1;
    }
}

fn check_surface() -> ExitCode {
    let root = workspace_root();
    let mut by_name: HashMap<String, Vec<String>> = HashMap::new();

    for rel in &["crates", "apps", "examples"] {
        let base = root.join(rel);
        if base.is_dir() {
            walk_surface(&base, &root, &mut by_name);
        }
    }

    let mut dup: Vec<(String, Vec<String>)> = by_name
        .into_iter()
        .filter(|(_, crates)| crates.len() > 1)
        .collect();
    dup.sort_by(|a, b| a.0.cmp(&b.0));

    if !dup.is_empty() {
        eprintln!(
            "xtask check-surface: FAILED\n\n\
             Same `pub struct`/`enum` name ending in `Key` or `Id` appears in multiple packages — \
             verify against `docs/GLOSSARY.md` **Crate** column (owning crate) and `docs/AGENT_PROTOCOL.md`.\n\n\
             Collisions:"
        );
        for (name, pkgs) in &dup {
            eprintln!("  {name}: {}", pkgs.join(", "));
        }
        return ExitCode::from(1);
    }

    println!("xtask check-surface: ok");
    ExitCode::SUCCESS
}

fn walk_surface(dir: &Path, root: &Path, by_name: &mut HashMap<String, Vec<String>>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "target" || name == ".git" {
                continue;
            }
            walk_surface(&p, root, by_name);
        } else if p.extension() == Some(OsStr::new("rs")) {
            let pkg = package_for_file(root, &p);
            let Ok(src) = fs::read_to_string(&p) else {
                continue;
            };
            for line in src.lines() {
                let t = line.trim_start();
                if let Some(rest) = t.strip_prefix("pub struct ")
                    && let Some(name) = surface_item_name(rest)
                    && (name.ends_with("Key") || name.ends_with("Id"))
                {
                    record(by_name, name, &pkg);
                } else if let Some(rest) = t.strip_prefix("pub enum ")
                    && let Some(name) = surface_item_name(rest)
                    && (name.ends_with("Key") || name.ends_with("Id"))
                {
                    record(by_name, name, &pkg);
                }
            }
        }
    }
}

/// First identifier after `pub struct` / `pub enum` — stops at ` `, `<`, `(`, `{`, `;`.
fn surface_item_name(rest: &str) -> Option<&str> {
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '(' | '{' | ';'))
        .unwrap_or(rest.len());
    let name = rest.get(..end).filter(|s| !s.is_empty())?;
    Some(name)
}

fn record(by_name: &mut HashMap<String, Vec<String>>, sym: &str, pkg: &str) {
    let e = by_name.entry(sym.to_string()).or_default();
    if !e.iter().any(|p| p == pkg) {
        e.push(pkg.to_string());
    }
}

fn package_for_file(root: &Path, file: &Path) -> String {
    let mut p = file.to_path_buf();
    loop {
        let cargo = p.join("Cargo.toml");
        if cargo.is_file()
            && let Ok(s) = fs::read_to_string(&cargo)
        {
            for line in s.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("name = ") {
                    let name = rest.trim().trim_matches('"');
                    return name.to_string();
                }
            }
        }
        if !p.pop() || p == root {
            break;
        }
    }
    "unknown".into()
}

fn check_adr_sync() -> ExitCode {
    let root = workspace_root();
    let adr = root.join("docs").join("adr");
    if !adr.is_dir() {
        println!("xtask check-adr-sync: no docs/adr");
        return ExitCode::SUCCESS;
    }

    let mut issues: Vec<String> = Vec::new();
    for ent in fs::read_dir(&adr).into_iter().flatten().flatten() {
        let p = ent.path();
        if p.extension() != Some(OsStr::new("md")) {
            continue;
        }
        let Ok(txt) = fs::read_to_string(&p) else {
            continue;
        };
        let Some(fm) = extract_front_matter(&txt) else {
            continue;
        };
        if fm.contains("migration-in-progress")
            && !fm.contains("affects-symbols")
            && !fm.contains("affects_symbols")
        {
            let rel = p.strip_prefix(&root).unwrap_or(&p);
            issues.push(format!(
                "{}: ADR with migration-in-progress should list affected symbols (front matter) for traceability",
                rel.display()
            ));
        }
    }

    if !issues.is_empty() {
        eprintln!("xtask check-adr-sync: FAILED\n");
        for i in &issues {
            eprintln!("  - {i}");
        }
        return ExitCode::from(1);
    }

    println!("xtask check-adr-sync: ok");
    ExitCode::SUCCESS
}

fn extract_front_matter(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut out = String::new();
    for line in lines {
        if line == "---" {
            return Some(out);
        }
        out.push_str(line);
        out.push('\n');
    }
    None
}

fn quality() -> ExitCode {
    let root = workspace_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["+nightly", "fmt", "--all", "--", "--check"])
        .status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        eprintln!("quality: fmt --check failed");
        return ExitCode::from(1);
    }
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        eprintln!("quality: clippy failed");
        return ExitCode::from(1);
    }
    let c1 = check_junior();
    if c1 != ExitCode::SUCCESS {
        return c1;
    }
    let c2 = check_surface();
    if c2 != ExitCode::SUCCESS {
        return c2;
    }
    check_adr_sync()
}

fn precommit() -> ExitCode {
    let q = quality();
    if q != ExitCode::SUCCESS {
        return q;
    }
    let root = workspace_root();
    let status = Command::new("cargo")
        .current_dir(&root)
        .args([
            "nextest",
            "run",
            "--workspace",
            "--profile",
            "ci",
            "--no-tests=pass",
        ])
        .status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        eprintln!("precommit: cargo nextest run failed");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
