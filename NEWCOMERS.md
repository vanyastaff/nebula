# Welcome New Contributors! 👋

Thank you for your interest in contributing to Nebula! This guide will get you started in **5 minutes**.

---

## 🎯 Quick Start (Choose Your Path)

### Path 1: I Want to Code

1. **Find an issue**: Browse [Good First Issues](https://github.com/vanyastaff/nebula/issues?q=is:issue+is:open+label:difficulty:good-first-issue)
2. **Comment**: "I'd like to work on this"
3. **Set up**: Follow [setup instructions](#setup-development-environment)
4. **Code**: Make your changes
5. **Submit**: Open a Pull Request

**Time:** 30 minutes to several hours depending on the issue

### Path 2: I Found a Bug

1. **Search**: Check if [someone already reported it](https://github.com/vanyastaff/nebula/issues)
2. **Report**: Use the [Bug Report Template](https://github.com/vanyastaff/nebula/issues/new?template=01-bug-report.yml)
3. **Wait**: A maintainer will triage it

**Time:** 5-10 minutes

### Path 3: I Have an Idea

1. **Check**: Search [existing feature requests](https://github.com/vanyastaff/nebula/issues?q=label:type:feature)
2. **Propose**: Use the [Feature Request Template](https://github.com/vanyastaff/nebula/issues/new?template=02-feature-request.yml)
3. **Discuss**: Maintainers will provide feedback

**Time:** 10-15 minutes

### Path 4: I Want to Learn

1. **Read**: Start with [README.md](../README.md)
2. **Explore**: Check [vision/ARCHITECTURE.md](../vision/ARCHITECTURE.md)
3. **Build**: Clone and run `cargo build`
4. **Ask**: Use [GitHub Discussions](https://github.com/vanyastaff/nebula/discussions)

**Time:** 30-60 minutes

---

## 🛠️ Setup Development Environment

### 1. Install Prerequisites

**Rust** (required):
```bash
# Install rustup (Rust installer)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify installation
rustc --version  # Should show 1.93+
```

**Git** (required):
```bash
# Verify Git is installed
git --version
```

### 2. Fork & Clone

```bash
# Fork on GitHub first (click "Fork" button)

# Then clone your fork
git clone https://github.com/YOUR_USERNAME/nebula.git
cd nebula

# Add upstream remote
git remote add upstream https://github.com/vanyastaff/nebula.git
```

### 3. Build & Test

```bash
# Build the project
cargo build

# Run tests
cargo test

# Check code quality
cargo clippy -- -D warnings
cargo fmt --check
```

**Expected time:** 5-10 minutes (depending on internet speed)

---

## 📝 Making Your First Contribution

### Step 1: Pick an Issue

**Recommended labels for beginners:**
- [`difficulty:good-first-issue`](https://github.com/vanyastaff/nebula/issues?q=label:difficulty:good-first-issue)
- [`type:docs`](https://github.com/vanyastaff/nebula/issues?q=label:type:docs)

**Claim the issue:**
```
Comment on the issue: "I'd like to work on this!"
```

### Step 2: Create a Branch

```bash
# Make sure you're on main
git checkout main

# Pull latest changes
git pull upstream main

# Create a feature branch
git checkout -b fix/issue-description-123
```

**Branch naming:**
- `feat/` → New feature
- `fix/` → Bug fix
- `docs/` → Documentation

See [WORKFLOW.md](../WORKFLOW.md#branch-naming) for full guide.

### Step 3: Make Changes

1. **Write code** (or update docs)
2. **Add tests** (if applicable)
3. **Run tests**: `cargo test`
4. **Format code**: `cargo fmt`
5. **Check lints**: `cargo clippy -- -D warnings`

### Step 4: Commit Your Work

```bash
# Stage changes
git add .

# Commit with conventional format
git commit -m "fix(engine): resolve panic in workflow execution

Fixes a panic that occurred when credentials were deleted
during workflow execution.

Closes #123"
```

**Commit message format:**
```
type(scope): subject

body (optional)

footer (optional, e.g., "Closes #123")
```

See [WORKFLOW.md#commit-conventions](../WORKFLOW.md#commit-conventions) for details.

### Step 5: Push & Open PR

```bash
# Push to your fork
git push origin fix/issue-description-123
```

**On GitHub:**
1. Go to your fork
2. Click "Compare & pull request"
3. Fill out the PR template
4. Click "Create pull request"

**PR Checklist:**
- [ ] Tests pass
- [ ] Code is formatted
- [ ] Clippy warnings fixed
- [ ] Issue linked (e.g., "Closes #123")

---

## 🤔 Common Questions

### How do I find something to work on?

**Filter issues by label:**
- [Good First Issues](https://github.com/vanyastaff/nebula/issues?q=label:difficulty:good-first-issue)
- [Documentation](https://github.com/vanyastaff/nebula/issues?q=label:type:docs)
- [Help Wanted](https://github.com/vanyastaff/nebula/issues?q=label:status:ready)

### Can I work on an issue without commenting first?

**Yes**, but:
- Comment to avoid duplicate work
- Maintainers can provide guidance
- Issue might already be assigned

### How long should I wait for a response?

- **Issues**: 1-3 days for triage
- **PRs**: 2-7 days for review (depends on size)

If no response after 7 days, ping with a comment.

### What if I get stuck?

1. **Read docs**: Check [ARCHITECTURE.md](../vision/ARCHITECTURE.md)
2. **Search issues**: Maybe someone had the same problem
3. **Ask**: Comment on the issue or open a [discussion](https://github.com/vanyastaff/nebula/discussions)
4. **Be patient**: Maintainers are volunteers

### My tests are failing, what do I do?

```bash
# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Check logs
RUST_LOG=debug cargo test
```

Still stuck? Ask in the issue comments.

### How do I sync my fork with upstream?

```bash
# Fetch upstream changes
git fetch upstream

# Switch to main
git checkout main

# Merge upstream changes
git merge upstream/main

# Push to your fork
git push origin main
```

---

## 📚 Essential Reading

| Document | When to Read |
|----------|--------------|
| [README.md](../README.md) | First thing (5 min) |
| [QUICK_START.md](../QUICK_START.md) | Before coding (5 min) |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Before first PR (15 min) |
| [WORKFLOW.md](../WORKFLOW.md) | When creating branches/commits (10 min) |
| [vision/ARCHITECTURE.md](../vision/ARCHITECTURE.md) | When working on complex features (30 min) |

---

## 🎉 After Your First Contribution

**Congratulations!** You're now a Nebula contributor!

**Next steps:**
1. Add yourself to the [contributors list](../README.md#contributors) (if applicable)
2. Find another issue to work on
3. Help other new contributors
4. Share your experience

**Stay connected:**
- Watch the repo for updates
- Join [Discussions](https://github.com/vanyastaff/nebula/discussions)
- Follow the [ROADMAP](../vision/ROADMAP.md)

---

## 🚨 Important Reminders

- **Be respectful**: Read the [Code of Conduct](../CONTRIBUTING.md#code-of-conduct)
- **Test your code**: Always run `cargo test` before pushing
- **Format your code**: Run `cargo fmt`
- **Follow conventions**: Use the [commit format](../WORKFLOW.md#commit-conventions)
- **Link issues**: Reference issue numbers in PRs (e.g., "Closes #123")

---

## 🆘 Need Help?

- **Questions**: [GitHub Discussions](https://github.com/vanyastaff/nebula/discussions)
- **Bugs**: [Bug Report Template](https://github.com/vanyastaff/nebula/issues/new?template=01-bug-report.yml)
- **Features**: [Feature Request Template](https://github.com/vanyastaff/nebula/issues/new?template=02-feature-request.yml)
- **Docs**: [Documentation Issue Template](https://github.com/vanyastaff/nebula/issues/new?template=03-documentation.yml)

---

**Welcome aboard! We're excited to have you contribute to Nebula!** 🚀

