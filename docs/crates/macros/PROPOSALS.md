# Proposals

## P-001: Expansion Debugging Doc

**Type:** Non-breaking

**Motivation:** Authors sometimes need to inspect generated code for debugging or learning.

**Proposal:** Document use of `cargo expand` (or equivalent) in README or TEST_STRATEGY; add to doc map.

**Expected benefits:** Lower friction when debugging macro output.

**Costs:** Doc only; cargo expand is external tool.

**Status:** Draft

---

## P-002: Improved Diagnostic Quality

**Type:** Non-breaking

**Motivation:** Proc-macro errors can be opaque; better span and message improve DX.

**Proposal:** Where possible, emit errors with span pointing to the attribute or field that is wrong; suggest correct syntax in message. Incremental improvement per derive.

**Expected benefits:** Authors fix issues faster.

**Costs:** Dev time; may require syn/quote patterns for span propagation.

**Status:** Draft
