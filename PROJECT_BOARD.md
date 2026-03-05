# GitHub Project Board Setup

This document describes how to set up and use GitHub Projects for managing Nebula's workflow and roadmap.

---

## Overview

We use **GitHub Projects (Beta)** as a kanban-style board for:
- Sprint planning
- Issue triage and prioritization
- Progress tracking
- Roadmap visibility

---

## Board Structure

### Columns

Create a project with the following columns:

1. **📋 Backlog**
   - All new issues start here
   - Issues awaiting triage
   - Future ideas and enhancements

2. **🔍 Triage**
   - Issues needing discussion or clarification
   - Blocked by unclear requirements
   - Awaiting stakeholder decision

3. **✅ Ready**
   - Approved and ready to work on
   - Clear acceptance criteria
   - No blockers
   - Great for contributors to pick up

4. **🚀 In Progress**
   - Someone is actively working on it
   - Assigned to a person
   - Target completion date set

5. **👀 In Review**
   - PR is open for code review
   - Awaiting maintainer feedback

6. **✨ Done**
   - PR merged
   - Deployed to main
   - Issue closed

---

## Workflow

### Issue Lifecycle

```
New Issue Created
    ↓
📋 Backlog (triage not started)
    ↓
🔍 Triage (discussion, clarification)
    ↓
✅ Ready (approved, clear requirements)
    ↓
🚀 In Progress (assigned, work started)
    ↓
👀 In Review (PR open)
    ↓
✨ Done (merged, closed)
```

### Moving Between Columns

#### Backlog → Triage

**When:** Issue is assigned for review

**Who:** Maintainer or reviewer

**Action:**
- Read the issue
- Ask clarifying questions (if needed)
- Assign labels (see [LABELS.md](LABELS.md))
- Move to Triage or Ready

#### Triage → Ready

**When:** Issue is clear and actionable

**Who:** Maintainer

**Conditions:**
- ✅ Acceptance criteria defined
- ✅ No blockers identified
- ✅ Priority set
- ✅ Difficulty estimated (if applicable)
- ✅ Related issues linked

**Action:**
- Add `status:ready` label
- Move to Ready column
- Post comment: "Ready for work! CC @contributor-interest"

#### Ready → In Progress

**When:** Someone commits to working on it

**Who:** Contributor

**Action:**
- Assign yourself
- Comment: "I'm working on this"
- Move to In Progress
- Create a branch with the issue number: `feat/xyz-#123`

#### In Progress → In Review

**When:** PR is opened

**Who:** Contributor

**Action:**
- Move issue to In Review
- Link PR in the issue (GitHub does this automatically if you reference the issue)
- Request reviews

#### In Review → Done

**When:** PR is merged

**Who:** Maintainer

**Action:**
- GitHub automatically closes the issue when PR is merged
- Card moves to Done column (if configured)

---

## Views & Filters

### Sprint Board

**Purpose:** Weekly or bi-weekly sprint planning

**Setup:**
- Filter by: `status:in-progress` + `status:ready`
- Columns: Ready → In Progress → In Review → Done
- Assigned person + due date visible

### Backlog Grooming

**Purpose:** Triage and prepare next sprint

**Setup:**
- Filter by: `status:blocked` + `status:needs-discussion`
- Sort by: Priority (P0, P1, P2, P3)
- Owner: Maintainers

### Roadmap View

**Purpose:** Phase-level planning

**Setup:**
- Filter by: `stage:phase2` (or whichever phase)
- Group by: Area (action, engine, runtime, etc.)
- Show: Priority + Difficulty

---

## Automation

### GitHub Actions

If you set up GitHub Actions, you can automate column transitions:

```yaml
# .github/workflows/project-management.yml
name: Auto-project-update

on:
  pull_request:
    types: [opened, ready_for_review]

jobs:
  move-to-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/github-script@v7
        with:
          script: |
            // Move to In Review column when PR opens
            // (requires project ID, column ID)
```

### Label-Based Automation

Use labels to trigger actions:

- `status:ready` → Move to Ready column
- `status:in-progress` → Move to In Progress
- `status:blocked` → Move to Triage

---

## Best Practices

### ✅ Do's

- **Keep it updated** — Move cards as work progresses
- **One person per issue** — Assign the primary owner
- **Use due dates** — For time-sensitive items
- **Link related issues** — "Related to #456"
- **Close when done** — Merged = closed

### ❌ Don'ts

- **Don't leave issues unassigned** — Always have an owner
- **Don't have too many In Progress** — Focus on completing current work
- **Don't force issues through columns** — Skip Triage only if truly clear
- **Don't update manually if automation exists** — Let the bot move cards

---

## Sprint Planning Example

### Monday Morning

**Goal:** Plan the week's work

**Process:**
1. Review `Backlog` column (10 min)
   - Triage any new issues from the weekend
   - Assign labels and priority
   - Move clear items to Ready

2. Groom `Triage` column (15 min)
   - Discuss blockers
   - Clarify requirements
   - Move to Ready if ready; to Backlog if postponed

3. Review `In Progress` (10 min)
   - Check for blockers
   - Unblock as needed
   - Adjust deadlines if needed

4. Plan the sprint (20 min)
   - Identify 5–10 Ready items
   - Discuss with team
   - Assign to team members
   - Move to In Progress

### Friday EOD

**Goal:** Review the week

**Process:**
1. Move merged PRs to Done
2. Close completed issues
3. Discuss any blockers
4. Note what didn't get done (for next week)

---

## Metrics & Reporting

### Velocity

Track items completed per week:

```
Week 1: 5 issues closed
Week 2: 7 issues closed
Week 3: 3 issues closed (blocked by X)

Velocity: ~5 issues/week (trend)
```

### Burndown

For sprint-based planning:

```
Sprint goal: 20 issues
Monday: 20 issues
Wednesday: 12 issues remaining
Friday: 3 issues remaining
```

### By Area

See which areas are getting the most work:

```
engine: 8 issues (40%)
storage: 5 issues (25%)
api: 4 issues (20%)
docs: 3 issues (15%)
```

---

## Templates

### Issue Template

When creating an issue, include:

```markdown
## Description
...

## Acceptance Criteria
- [ ] Criterion 1
- [ ] Criterion 2
- [ ] Criterion 3

## Related Issues
Closes #123
Related to #456

## Type
(Check one)
- [ ] Bug
- [ ] Feature
- [ ] Enhancement
- [ ] Docs
```

### Sprint Planning Template

At the start of each sprint:

```markdown
## Sprint Goal
One-line summary of what we're trying to accomplish.

## Issues
- #123 — Issue title (assigned to @person)
- #456 — Issue title (assigned to @person)

## Risks
Any blockers or risks?

## Success Criteria
How will we know we won the sprint?
```

---

## Troubleshooting

**Q: Issue is stuck in In Progress for weeks**
A: Check if the person is still working on it. If not, reassign or move back to Ready.

**Q: Too many items in In Progress**
A: This is a sign of context switching. Limit to 3–5 concurrent items per person.

**Q: We're not hitting sprint goals**
A: Retrospective: Was the goal too ambitious? Did blockers appear? Adjust for next sprint.

**Q: New contributors can't find work**
A: Filter by `difficulty:good-first-issue` + `status:ready`. These should always be available.

---

## See Also

- [ISSUES.md](ISSUES.md) — How to report and triage issues
- [LABELS.md](LABELS.md) — Label system and meanings
- [WORKFLOW.md](WORKFLOW.md) — Branch and commit conventions
- [vision/ROADMAP.md](../vision/ROADMAP.md) — Long-term roadmap

---

**Questions?** Open a [discussion](https://github.com/vanyastaff/nebula/discussions) or contact a maintainer.

