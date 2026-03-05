# Maintainer Guide

Quick reference for project maintainers.

---

## 🏷️ Issue Triage

### When a New Issue Arrives

1. **Read the issue carefully**
2. **Check for duplicates** (search existing issues)
3. **Add labels**:
   - Type (bug, feature, docs, etc.)
   - Area (engine, runtime, storage, etc.)
   - Priority (P0-P3)
   - Difficulty (if obvious)
4. **Decide on status**:
   - Clear? → `status:ready` + move to Ready column
   - Needs discussion? → `status:needs-discussion` + move to Triage
   - Blocked? → `status:blocked` + comment why
5. **Comment** if more info is needed or to welcome contributor

### Label Quick Reference

See [LABELS.md](../LABELS.md) for full details.

**Must have:**
- One `type:*` label
- At least one `area:*` label
- One `priority:*` label (if P0-P1)

**Optional:**
- `difficulty:*`
- `status:*`
- `stage:*`

---

## 👀 Pull Request Review

### Review Checklist

- [ ] **Linked issue**: PR references an issue (Closes #123)
- [ ] **Tests pass**: CI is green
- [ ] **Code quality**: Clippy warnings addressed
- [ ] **Formatting**: Code is formatted (`cargo fmt`)
- [ ] **Tests included**: New code has tests
- [ ] **Breaking changes**: Documented if present
- [ ] **Commit messages**: Follow conventions
- [ ] **Documentation**: Updated if API changed

### Review Process

1. **Read the PR description**
2. **Check CI status** (must be green)
3. **Review code changes**:
   - Correctness
   - Design (follows architecture)
   - Performance (no obvious issues)
   - Readability
   - Tests
4. **Leave feedback**:
   - Be constructive and respectful
   - Explain reasoning
   - Suggest alternatives
5. **Approve or request changes**
6. **Merge when ready**:
   - Squash commits if messy history
   - Use descriptive merge message
   - Delete branch after merge

### Review Response Time

- **Small PRs** (< 200 lines): 1-2 days
- **Medium PRs** (200-500 lines): 2-5 days
- **Large PRs** (> 500 lines): 5-7 days

If you can't review within this timeframe, comment to let the author know.

---

## 🚀 Merging & Releases

### Merging a PR

```bash
# Via GitHub UI (preferred):
# 1. Click "Squash and merge" for PRs with messy history
# 2. Click "Merge commit" for clean PRs
# 3. Delete branch after merge

# Via command line (if needed):
git checkout main
git pull upstream main
git merge --squash pr-branch
git commit -m "feat(crate): description (#123)"
git push upstream main
```

### Release Process

See [WORKFLOW.md#versioning--releases](../WORKFLOW.md#versioning--releases) for full details.

**Quick steps:**
1. Update version in all `Cargo.toml` files
2. Update CHANGELOG.md
3. Create release PR: `chore: release v0.x.0`
4. Merge to main
5. Tag release: `git tag -a v0.x.0 -m "Release v0.x.0"`
6. Push tag: `git push origin v0.x.0`
7. Create GitHub Release from tag
8. Publish to crates.io: `cargo publish`

---

## 🎯 Project Board Management

### Daily Tasks

- **Review Backlog** (5 min): Triage new issues
- **Check In Progress** (5 min): Unblock contributors
- **Review PRs** (30-60 min): Code review

### Weekly Tasks

- **Monday**: Groom backlog, plan sprint
- **Wednesday**: Check sprint progress
- **Friday**: Review completed work, retrospective

### Moving Cards

- **Backlog → Triage**: Needs discussion
- **Triage → Ready**: Approved and clear
- **Ready → In Progress**: Someone starts work
- **In Progress → In Review**: PR opened
- **In Review → Done**: PR merged

See [PROJECT_BOARD.md](../PROJECT_BOARD.md) for details.

---

## 🏃 Sprint Planning

### Sprint Template

```markdown
## Sprint X Goal
One-line summary

## Issues
- #123 — Issue title (@assignee)
- #456 — Issue title (@assignee)

## Risks
Any blockers?

## Success Criteria
How do we know we succeeded?
```

### Planning Process

1. **Review last sprint** (10 min)
   - What was completed?
   - What was blocked?
   - Lessons learned?

2. **Groom backlog** (15 min)
   - Triage new issues
   - Clarify unclear issues
   - Update priorities

3. **Select issues** (20 min)
   - Pick 5-10 Ready issues
   - Balance by area and difficulty
   - Assign to team members

4. **Set sprint goal** (5 min)
   - One clear objective
   - Communicate to team

---

## 🚨 Handling Critical Issues

### P0 Issues (Critical)

**Examples:**
- Security vulnerability
- Data loss
- System completely broken

**Response:**
1. Acknowledge immediately (< 1 hour)
2. Assess severity
3. Create hotfix branch
4. Fix and test
5. Fast-track PR review
6. Merge and deploy ASAP
7. Post-mortem (optional, for learning)

### Security Vulnerabilities

**DO NOT** open a public issue!

1. Acknowledge reporter privately
2. Assess severity (use CVSS)
3. Fix in private branch
4. Coordinate disclosure
5. Publish fix + advisory

---

## 👥 Community Management

### Welcoming New Contributors

When someone makes their first contribution:

```markdown
Welcome to Nebula, @username! 🎉

Thank you for your contribution! A maintainer will review your PR soon.

While you wait, feel free to:
- Browse other [good first issues](link)
- Join our [discussions](link)
- Read about our [architecture](link)

Thanks for being part of the community!
```

### Handling Difficult Situations

- **Rude comments**: Remove, warn, or ban (per Code of Conduct)
- **Low-quality PRs**: Be respectful, suggest improvements
- **Abandoned PRs**: Wait 2 weeks, then close with comment
- **Stale issues**: Wait 90 days, then close with `status:stale` label

---

## 📊 Metrics to Track

### Weekly

- Issues opened vs. closed
- PRs opened vs. merged
- Average time to first response
- Average time to merge

### Monthly

- Contributor growth
- Code coverage trends
- Performance benchmarks
- Community engagement (discussions, stars)

---

## 🔧 Maintenance Tasks

### Monthly

- [ ] Review and update dependencies
- [ ] Run security audit: `cargo audit`
- [ ] Check for CVEs: `cargo deny check`
- [ ] Review and close stale issues
- [ ] Update ROADMAP.md with progress

### Quarterly

- [ ] Review label system (add/remove as needed)
- [ ] Update ARCHITECTURE.md if structure changed
- [ ] Retrospective: What's working? What's not?
- [ ] Update CONTRIBUTING.md with lessons learned

---

## 📞 Escalation

### When to Escalate

- Security vulnerabilities
- Legal concerns
- Code of Conduct violations
- Major architectural decisions

### How to Escalate

1. Document the issue
2. Gather relevant context
3. Discuss with core team
4. Make decision
5. Document decision in DECISIONS.md (for architecture)

---

## 🎓 Resources for Maintainers

- [WORKFLOW.md](../WORKFLOW.md) — Branch, commit, release process
- [PROJECT_BOARD.md](../PROJECT_BOARD.md) — Using the project board
- [LABELS.md](../LABELS.md) — Label definitions
- [ISSUES.md](../ISSUES.md) — Issue guidelines
- [vision/ROADMAP.md](../vision/ROADMAP.md) — Long-term plan

---

## 💬 Communication

### Response Templates

**Needs more info:**
```markdown
Thank you for the report! To help us investigate, could you provide:
- Rust version (`rustc --version`)
- OS and version
- Steps to reproduce
- Expected vs. actual behavior
```

**Not reproducible:**
```markdown
Thank you for the report. Unfortunately, we cannot reproduce this issue.
Could you provide more details or a minimal example?

If we don't hear back in 14 days, we'll close this issue.
```

**Closing as duplicate:**
```markdown
Thank you! This looks like a duplicate of #123.
Closing this issue in favor of that one.

Feel free to add any additional context to #123!
```

**Won't fix:**
```markdown
Thank you for the suggestion. After discussion, we've decided not to
pursue this because [reason].

We appreciate your input! Feel free to discuss further or propose alternatives.
```

---

**Questions?** Contact other maintainers or open a discussion.

