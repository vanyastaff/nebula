# Proposals

## P001: Transactional staged reload API

Type: Non-breaking (additive)

Motivation:
- some consumers need explicit pre-apply hooks before config swap.

Proposal:
- add staged reload callbacks (`pre_validate`, `pre_apply`, `post_apply`) with rollback hook.

Expected benefits:
- safer dynamic reconfiguration for critical services.

Costs:
- higher complexity in reload lifecycle.

Risks:
- callback misuse can block reload paths.

Compatibility impact:
- additive if optional.

Status: Review

## P002: Typed path helper layer

Type: Potentially breaking (if replacing string paths)

Motivation:
- plain string paths are typo-prone.

Proposal:
- introduce typed path builders/constants and optional compile-time checked macros.

Expected benefits:
- fewer runtime path errors.

Costs:
- API expansion and migration docs.

Risks:
- ergonomic burden if too strict.

Compatibility impact:
- additive first, breaking only if string APIs deprecated later.

Status: Draft

## P003: Remote source trust and auth policy framework

Type: Non-breaking (additive)

Motivation:
- source enum includes remote classes but production policies are incomplete.

Proposal:
- define source trust levels, auth strategy traits, and signature/version verification.

Expected benefits:
- safer remote config adoption.

Costs:
- implementation and operational overhead.

Risks:
- misconfiguration can still create blast radius if defaults weak.

Compatibility impact:
- additive.

Status: Draft

## P004: Merge strategy profiles

Type: Potentially breaking (behavioral)

Motivation:
- current merge behavior is generic; some domains need strict conflict handling.

Proposal:
- support merge profiles (`last-write-wins`, `strict-no-overwrite`, `deep-merge-lists`).

Expected benefits:
- domain-aligned merge semantics.

Costs:
- increased complexity and migration burden.

Risks:
- silent behavior drift if profile changes unintentionally.

Compatibility impact:
- major-version candidate if default profile changes.

Status: Draft
