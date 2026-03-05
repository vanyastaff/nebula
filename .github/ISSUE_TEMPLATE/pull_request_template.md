name: Pull Request
description: Create a PR to contribute changes
body:
  - type: markdown
    attributes:
      value: |
        Thank you for contributing! Please fill out this form to help us review your PR.

  - type: textarea
    id: description
    attributes:
      label: Description
      description: What does this PR do? What problem does it solve?
      placeholder: "Implements workflow versioning, allowing multiple versions to coexist"
    validations:
      required: true

  - type: input
    id: issue
    attributes:
      label: Related Issue
      description: "Link to the issue this PR closes (e.g., #123)"
      placeholder: "Closes #42"
    validations:
      required: false

  - type: textarea
    id: changes
    attributes:
      label: Changes
      description: Brief list of what was changed
      placeholder: |
        - Add `version` field to Workflow struct
        - Implement workflow cloning for new versions
        - Update API endpoints to support versioning
        - Add tests for version management
    validations:
      required: true

  - type: dropdown
    id: type
    attributes:
      label: Type of Change
      description: What kind of change is this?
      options:
        - "🐛 Bug Fix"
        - "✨ Feature"
        - "🚀 Enhancement"
        - "📚 Documentation"
        - "🧹 Chore"
        - "📊 Performance"
      validations:
        required: true

  - type: textarea
    id: testing
    attributes:
      label: Testing
      description: How did you test this change?
      placeholder: |
        - Added 15 new unit tests in `tests/versioning.rs`
        - Ran full test suite: `cargo test`
        - Manually tested workflow creation and versioning via API
    validations:
      required: true

  - type: checkboxes
    id: checklist
    attributes:
      label: Pre-Submission Checklist
      options:
        - label: Tests pass locally (`cargo test`)
          required: true
        - label: Clippy passes (`cargo clippy -- -D warnings`)
          required: true
        - label: Code is formatted (`cargo fmt`)
          required: true
        - label: Commit messages follow conventions (see WORKFLOW.md)
          required: true
        - label: No breaking changes (or breaking changes are discussed in PR description)
          required: true
        - label: Documentation updated (if needed)
          required: false
        - label: Related issues linked
          required: false

  - type: textarea
    id: breaking
    attributes:
      label: Breaking Changes
      description: "If this PR includes breaking changes, describe them here"
      placeholder: |
        N/A - This is fully backward compatible

  - type: textarea
    id: notes
    attributes:
      label: Additional Notes
      description: Any additional context for reviewers?
      placeholder: "This PR is part of Phase 2 (Execution Engine)"

