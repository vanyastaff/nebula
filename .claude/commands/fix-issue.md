---
description: Systematically analyze and fix a GitHub issue
---

Analyze and fix GitHub issue #{{arg:issue_number}}:

## Process

1. **Fetch issue details**
   ```bash
   gh issue view {{arg:issue_number}} --json title,body,labels,state
   ```

2. **Create todo list** for tracking progress with these tasks:
   - Analyze issue root cause
   - Read affected files and understand architecture
   - Apply architectural pattern (not quick patch)
   - Test compilation and functionality
   - Document solution in GitHub
   - Close issue with detailed summary

3. **Apply architectural approach**
   - Identify root cause (not just symptoms)
   - Choose appropriate pattern:
     * Extension Trait for ergonomics
     * Type Erasure for object safety
     * Scoped Callback for resource management
     * Newtype for type safety
   - NOT quick patches or #[allow] directives

4. **Verify fix**
   ```bash
   cargo check --workspace
   cargo test -p <affected-crate> --lib
   ```

5. **Document solution**
   Create comprehensive comment explaining:
   - What was broken (root cause)
   - Why it happened (architectural problem)
   - How it was fixed (pattern applied)
   - Test results (proof it works)

6. **Close issue**
   ```bash
   gh issue close {{arg:issue_number}} --comment "Resolved! See details above."
   ```

## Guidelines

- **Principle**: "продолжаем делать реальный рефакторинг правильный а не просто чтобы ошибка исчезла"
- Apply proper architectural patterns
- Document architectural decisions
- Verify with tests
- Leave code better than found

## Example Usage

```
/fix-issue 53
```

This will systematically fix issue #53 following the proper refactoring workflow.
