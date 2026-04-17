<!-- Template for docs/adr/NNNN-kebab-title.md. No central index — discovery via filename prefix (NNNN) and frontmatter. -->

---
id: NNNN
title: <short kebab title>
status: proposed | accepted | superseded | deprecated
date: YYYY-MM-DD
supersedes: []           # list of ADR ids this replaces
superseded_by: []        # filled in later if this ADR is replaced
tags: [schema, credential, execution, ...]
related: [other/docs.md]
---

# NNNN. <Title>

## Context

What situation prompts this decision. Include constraints, prior state, and what
forcing function triggered the decision.

## Decision

The decision itself, stated plainly. One or two paragraphs. If the decision is
"no, we will not do X" — say that clearly.

## Consequences

What changes because of this decision. Include:

- Positive consequences (what improves).
- Negative consequences (what we accept).
- Follow-up work this enables or requires.

## Alternatives considered

Alternatives we evaluated and rejected, each with the reason.

## Seam / verification

If this ADR locks in an L2 invariant, name the seam (code location) and test
that enforces it.
