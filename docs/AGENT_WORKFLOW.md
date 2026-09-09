# Agent workflow templates

Use these for substantial tasks, interrupted work, or a handoff. Small edits need only
an outcome and verification result. Repository rules live in [AGENTS.md](../AGENTS.md);
this document supplies reusable formats, not another set of architecture rules.

## Task brief

Record in the conversation; use the private vault for design records. Fill from the
request and current code rather than turning every field into a question.

```text
Outcome: observable behavior or artifact that will exist when finished.
Scope: affected crates/files and required downstream consequences.
Constraints: invariants, compatibility, existing edits, explicit exclusions.
Verification: commands or scenarios and their expected results.
Assumptions: decisions being made and any unresolved question that changes the work.
Authorization: any external or destructive action already approved, with its limits.
```

For work spanning several steps or contexts, keep the user's original requirements
and later corrections in a local ignored note; redact secrets before writing it.
Keep decisive wording verbatim and date later changes rather than rewriting the
original request. Distinguish user requirements, implementation assumptions, and
constraints discovered in the code. A new message updates the ongoing task unless
it explicitly cancels or replaces it. An unmet requirement stays open; changing the
plan or marking it deferred does not make it delivered.

For a cross-crate task, map each requirement to its owning change and acceptance
evidence. Every planned change should serve a requirement or a necessary downstream
consequence. Use a short list; a separate tracker or numbered manifest is optional.

For unattended work, also record the user's time or cost limit if one was given and
which actions require a reply. Do not infer approval from silence. At a limit or an
unresolved blocker, report the actual state and the next step; do not label it done.

## Resume or handoff note

Keep this in the conversation or an ignored local location such as
`.claude/summaries/`. Private design decisions belong in the vault. Keep secrets out
of both locations. A new agent verifies the checkout and diff before using the note.

```text
Goal and acceptance criteria:
Request record: original requirements and later corrections, or a local note path.
Checkout: path, branch, HEAD commit.
Existing user edits: paths and what must be preserved.
Completed work: changed paths and resulting behavior; committed or uncommitted.
Decisions: choice, rationale, and source (canon path or ADR id where applicable).
Evidence: command, working directory, feature/backend selection, result, revision.
Remaining work: ordered next steps, failures, and checks not run.
Pending decisions: question, current assumption, and whether work depends on a reply.
Authorization: approved actions and limits; actions still awaiting approval.
References: essential paths, issue links, and local log paths without secret values.
```

Keep only information needed to continue: no transcript copies, raw logs, or discarded
drafts. Preserve a failed attempt only when its cause changes the next step. Evidence
from an earlier revision is historical; rerun affected checks after relevant edits.

## Review and completion

First compare the result with the original request and later corrections, without
using the plan as a substitute. For each requirement, identify observable evidence
or mark it partial, missing, or unverified. Then review the final diff and relevant
surrounding code against the acceptance criteria.
Check architecture boundaries, regressions, test weakening, accidental public surface,
secret exposure, and missing downstream changes. A finding includes severity,
`file:line`, a concrete failure scenario, and a suggested correction. Report limits of
the review even when no findings remain.

For changed behavior, inspect what the tests assert: expected values need an
independent basis, and the relevant failure case must be checked. A passing count
alone does not establish coverage. Report skipped tests and unavailable backends;
reading code, a unit test, and running the end-to-end scenario prove different things.

Use the commands and feature requirements in AGENTS.md and the affected crate guides.
For documentation-only changes, check the diff, links, referenced commands, and typos;
Rust tests add no evidence unless the change affects executable examples or code.
The pre-PR gate in AGENTS.md still applies when preparing a PR.

Capture the actual command exit status, with `pipefail` for output pipelines. Keep
logs locally when the last lines omit the failure cause. Reuse prior evidence only
when its revision and configuration still apply. During shared-checkout work, arrange
an interval without concurrent writes for the combined check; otherwise its result
cannot establish that the final diff passed.

Completion report:

```text
Changed: result and affected files.
Verified: checks actually completed and their scope.
Unverified or blocked: failures, skipped checks, prerequisites, remaining risks.
Next action: only if work remains or a concrete approval is needed.
```

Do not claim merge, deployment, publication, a running background job, or a shared
artifact link unless the action succeeded and its result was verified.
