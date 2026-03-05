# Setting Up GitHub Projects for Nebula

This guide walks you through setting up a GitHub Project Board from scratch.

---

## Prerequisites

- Repository admin access
- GitHub Projects (Beta) enabled

---

## Step-by-Step Setup

### 1. Create a New Project

1. Go to your repository on GitHub
2. Click **Projects** tab
3. Click **New Project** → **Board**
4. Name it: `Nebula Development Board`
5. Click **Create**

### 2. Add Columns

Create the following columns (in order):

#### 📋 Backlog
- **Purpose**: New issues awaiting triage
- **Automation**: None (manual)

#### 🔍 Triage
- **Purpose**: Issues needing discussion or clarification
- **Automation**: Auto-add issues with `status:needs-discussion` label

#### ✅ Ready
- **Purpose**: Approved issues ready for work
- **Automation**: Auto-add issues with `status:ready` label

#### 🚀 In Progress
- **Purpose**: Active work in progress
- **Automation**: Auto-move when issue is assigned

#### 👀 In Review
- **Purpose**: PR open, awaiting review
- **Automation**: Auto-move when PR is opened

#### ✨ Done
- **Purpose**: Completed and merged
- **Automation**: Auto-move when PR is merged

### 3. Configure Automation (Optional)

If using GitHub Actions, create `.github/workflows/project-automation.yml`:

```yaml
name: Project Board Automation

on:
  issues:
    types: [opened, labeled]
  pull_request:
    types: [opened, ready_for_review, closed]

jobs:
  update-project:
    runs-on: ubuntu-latest
    steps:
      - name: Move to appropriate column
        uses: actions/add-to-project@v0.5.0
        with:
          project-url: https://github.com/orgs/YOUR_ORG/projects/YOUR_PROJECT_ID
          github-token: ${{ secrets.ADD_TO_PROJECT_PAT }}
```

### 4. Add Field Definitions

Add custom fields to your project:

1. **Priority** (Single Select)
   - P0 (Critical)
   - P1 (Important)
   - P2 (Normal)
   - P3 (Low)

2. **Difficulty** (Single Select)
   - Good First Issue
   - Medium
   - Hard

3. **Area** (Single Select)
   - Action
   - Engine
   - Runtime
   - Storage
   - API
   - UI
   - Docs
   - Infrastructure

4. **Phase** (Single Select)
   - Phase 1: Core Foundation
   - Phase 2: Execution Engine
   - Phase 3: Credential System
   - Phase 4: Plugin Ecosystem
   - Phase 5: Desktop App

5. **Sprint** (Text)
   - For sprint planning (e.g., "Sprint 1", "Sprint 2")

### 5. Create Saved Views

Create filtered views for different workflows:

#### Current Sprint
- **Filter**: Status = In Progress OR In Review
- **Sort**: Priority (P0 first)
- **Group by**: Assigned to

#### Backlog Grooming
- **Filter**: Status = Backlog OR Triage
- **Sort**: Priority, then Created date
- **Group by**: Area

#### Phase 2 Work
- **Filter**: Phase = Phase 2
- **Sort**: Priority
- **Group by**: Status

#### Good First Issues
- **Filter**: Difficulty = Good First Issue AND Status = Ready
- **Sort**: Created date (oldest first)

### 6. Link Issues to Project

#### Automatically
Add this to `.github/workflows/add-to-project.yml`:

```yaml
name: Add Issues to Project

on:
  issues:
    types: [opened]

jobs:
  add-to-project:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/add-to-project@v0.5.0
        with:
          project-url: https://github.com/users/vanyastaff/projects/YOUR_PROJECT_NUMBER
          github-token: ${{ secrets.ADD_TO_PROJECT_PAT }}
```

#### Manually
1. Open an issue
2. Click **Projects** in the right sidebar
3. Select your project
4. Issue is added to Backlog by default

---

## Daily Workflow

### Morning Standup
1. Review **In Progress** column
2. Check for blockers
3. Move stale items back to Ready

### Throughout the Day
- **New issue created** → Auto-added to Backlog
- **Issue triaged** → Add labels, move to Ready
- **Work started** → Assign yourself, move to In Progress
- **PR opened** → Auto-moves to In Review
- **PR merged** → Auto-moves to Done

### Weekly Review
1. Groom Backlog (Monday)
2. Plan sprint (Monday)
3. Review completed work (Friday)
4. Archive Done items older than 2 weeks

---

## Using the Project Board

### For Contributors

**Finding Work:**
1. Go to project board
2. Switch to "Good First Issues" view
3. Pick an issue from Ready column
4. Comment "I'll work on this"
5. Assign yourself
6. Move to In Progress

**Updating Status:**
- Started work → Move to In Progress
- Opened PR → Auto-moves to In Review
- Blocked → Add `status:blocked` label, comment why

### For Maintainers

**Triaging Issues:**
1. Read the issue
2. Add labels (type, area, priority, difficulty)
3. If unclear → Add `status:needs-discussion`, move to Triage
4. If clear → Add `status:ready`, move to Ready

**Reviewing PRs:**
1. Review code
2. Approve or request changes
3. Merge when ready
4. Issue auto-closes and moves to Done

---

## Tips & Best Practices

### ✅ Do's
- Keep the board updated
- One person per issue
- Use labels consistently
- Link PRs to issues
- Close issues when done

### ❌ Don'ts
- Don't let In Progress grow unbounded
- Don't leave issues unassigned in In Progress
- Don't skip Triage for complex issues
- Don't forget to update labels

---

## Troubleshooting

**Q: Automation isn't working**
- Check GitHub Actions logs
- Verify PAT token has correct permissions
- Ensure project URL is correct

**Q: Too many issues in Backlog**
- Schedule regular grooming sessions
- Close stale issues (no activity in 90 days)
- Move unclear issues to Triage

**Q: In Progress column is too large**
- Limit WIP (work in progress) to 3-5 per person
- Move stale items back to Ready
- Review with team

---

## See Also

- [PROJECT_BOARD.md](../PROJECT_BOARD.md) — Usage guide
- [LABELS.md](../LABELS.md) — Label definitions
- [WORKFLOW.md](../WORKFLOW.md) — Development workflow
- [ISSUES.md](../ISSUES.md) — Issue guidelines

---

**Questions?** Open a [discussion](https://github.com/vanyastaff/nebula/discussions).

