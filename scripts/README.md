# Scripts

Automation scripts for setting up and managing the Nebula project.

---

## Setup Scripts

Choose the method that works best for you:

### Method 1: GitHub CLI (Recommended)

`setup-github.ps1` (Windows) / `setup-github.sh` (Linux/macOS)

Automatically creates GitHub labels using GitHub CLI.

**Prerequisites:**
- [GitHub CLI](https://cli.github.com/) installed
- Authenticated: `gh auth login`

**Usage (Windows - PowerShell):**
```powershell
.\scripts\setup-github.ps1
```

**Usage (Linux/macOS - Bash):**
```bash
chmod +x scripts/setup-github.sh
./scripts/setup-github.sh
```

**What it does:**
- Creates 40 labels across 6 categories:
  - **Type** (6): bug, feature, enhancement, docs, chore, question
  - **Area** (16): action, engine, runtime, storage, credential, api, etc.
  - **Priority** (4): p0 (critical), p1 (important), p2 (normal), p3 (low)
  - **Difficulty** (3): good-first-issue, medium, hard
  - **Status** (6): blocked, needs-discussion, in-progress, ready, on-hold, needs-triage
  - **Stage** (5): phase1, phase2, phase3, phase4, phase5

**Output:**
```
🚀 Setting up GitHub repository for Nebula...
✅ GitHub CLI authenticated
📦 Repository: vanyastaff/nebula

🏷️  Creating labels...
  ✅ Created: type:bug
  ✅ Created: type:feature
  ...
  
📊 Summary:
  Total: 40 labels

🎉 GitHub setup complete!
```

---

### Method 2: Python + GitHub API (No CLI Required)

`setup-github-api.py`

Uses Python and GitHub REST API directly. No GitHub CLI needed!

**Prerequisites:**
- Python 3.6+ installed
- GitHub Personal Access Token with `repo` scope
  - Create at: https://github.com/settings/tokens

**Usage:**

1. **Create GitHub Token:**
   - Go to https://github.com/settings/tokens
   - Click "Generate new token (classic)"
   - Give it a name: "Nebula Labels Setup"
   - Select scope: `repo` (full control)
   - Click "Generate token"
   - **Copy the token** (you won't see it again!)

2. **Set Environment Variable:**

   Windows (PowerShell):
   ```powershell
   $env:GITHUB_TOKEN = "ghp_your_token_here"
   ```

   Linux/macOS:
   ```bash
   export GITHUB_TOKEN="ghp_your_token_here"
   ```

3. **Run Script:**
   ```bash
   python scripts/setup-github-api.py
   ```

**Output:**
```
🚀 Setting up GitHub labels for Nebula...
✅ GitHub token found
📦 Repository: vanyastaff/nebula

🏷️  Creating 40 labels...
  ✅ Created: type:bug
  ✅ Created: type:feature
  ...

🎉 GitHub setup complete!
```

---

### Method 3: Manual Import (Web UI)

Use `labels.json` file with a GitHub app or manually.

1. Go to https://github.com/vanyastaff/nebula/labels
2. For each label in `scripts/labels.json`:
   - Click "New label"
   - Enter name, color, and description
   - Click "Create label"

*Note: This is tedious (40 labels!) but works if other methods fail.*

---

## Which Method to Use?

| Method | Best For | Pros | Cons |
|--------|----------|------|------|
| **Method 1 (gh CLI)** | Users with gh already installed | Fast, official tool | Requires gh setup |
| **Method 2 (Python)** | Everyone else | No dependencies except Python | Requires token |
| **Method 3 (Manual)** | Last resort | Always works | Very time-consuming |

**Recommendation:** Try Method 1 first. If `gh` is not installed, use Method 2.

---

## Label Reference

See the GitHub repository labels page for complete label definitions.

### Quick Reference

**Type (pick one):**
- `type:bug` — Something broken
- `type:feature` — New capability
- `type:enhancement` — Improve existing
- `type:docs` — Documentation
- `type:chore` — Build, CI, deps

**Priority (pick one):**
- `priority:p0` — Critical (fix now)
- `priority:p1` — Important (next sprint)
- `priority:p2` — Normal (backlog)
- `priority:p3` — Low (future)

**Difficulty (pick one):**
- `difficulty:good-first-issue` — For newcomers
- `difficulty:medium` — Moderate
- `difficulty:hard` — Complex

**Status (applied by maintainers):**
- `status:ready` — Approved, ready to work
- `status:in-progress` — Someone working on it
- `status:blocked` — Waiting on something
- `status:needs-discussion` — Needs design input

---

## Troubleshooting

### "GitHub CLI not found"

**Install GitHub CLI:**
- Windows: `winget install GitHub.cli`
- macOS: `brew install gh`
- Linux: See https://github.com/cli/cli/blob/trunk/docs/install_linux.md

### "Not authenticated"

```bash
gh auth login
```

Follow the prompts to authenticate.

### "Permission denied" (bash script)

```bash
chmod +x scripts/setup-github.sh
```

### "Labels already exist"

The script will update existing labels instead of creating duplicates. This is safe to run multiple times.

### "Rate limit exceeded"

GitHub API has rate limits. Wait a few minutes and try again.

---

## Manual Label Creation

If you can't use GitHub CLI, you can create labels manually:

1. Go to https://github.com/vanyastaff/nebula/labels
2. Click "New label"
3. Use the label definitions from the GitHub labels page

---

## See Also

- [.github/PROJECT_SETUP.md](../.github/PROJECT_SETUP.md) — Step-by-step project setup

---

## Benchmark Scripts

### Resilience A/B Benchmarks

- `bench-resilience.ps1` (Windows)
- `bench-resilience.sh` (Linux/macOS)

Automates Criterion runs for `nebula-resilience` (`manager`, `rate_limiter`, `circuit_breaker`, `compose`) with baseline support.

**PowerShell examples:**

```powershell
# Single run
./scripts/bench-resilience.ps1

# Save baseline named "main"
./scripts/bench-resilience.ps1 -Mode baseline -Baseline main

# Compare current branch against saved baseline
./scripts/bench-resilience.ps1 -Mode compare -Baseline main
```

**Bash examples:**

```bash
# Single run
./scripts/bench-resilience.sh

# Save baseline named "main"
./scripts/bench-resilience.sh baseline main

# Compare current branch against saved baseline
./scripts/bench-resilience.sh compare main
```

Reports are generated in `target/criterion/`.



