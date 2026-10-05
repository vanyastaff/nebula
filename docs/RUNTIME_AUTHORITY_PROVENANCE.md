# Runtime-authority provenance

Test Matrix retains producer-owned candidates and performs its same-run
integrity check. Its `protected-provenance` job then calls the repository's
**Protected Runtime Authority Provenance** reusable workflow at protected
`main`. That workflow checks the candidate with protected semantic policy and
signs its exact archive digest. The `Tests` aggregator requires this job to
succeed. North Star effective states remain `partial`; no registry-schema or
branch-protection setting is changed.

## Protected execution and immutable source identity

The protected workflow accepts no caller inputs. Its GitHub context is the
calling workflow's original context: `github.sha` is the original tested
synthetic merge SHA for a PR, and `github.run_id`/`github.run_attempt` identify
the original producer run. This avoids attempting to reconstruct a past merge
commit from mutable PR API metadata. GitHub's run/check-suite `head_sha` is the
PR head, while associated PR base/head metadata can be refreshed after a run.
Matching current merge parents cannot establish an old tested merge identity.

Before any checkout, the trusted reusable YAML asks GitHub's authenticated
runner OIDC endpoint for a fresh token over TLS. It never logs or retains the
token. It requires the exact `job_workflow_ref` for this repository's reusable
workflow at `refs/heads/main`, the GitHub OIDC issuer, and the original source
SHA/ref/repository/run/attempt matching the callee's immutable GitHub context.
The token's `job_workflow_sha` selects the immutable protected-policy checkout.
No PR checkout, candidate code, caller input, or candidate policy declaration
is loaded before this anchor. The subsequent signed-attestation verification
cryptographically checks the protected signer and original source identities.

The elevated `actions: read`, `id-token: write`, and `attestations: write`
permissions appear only on the protected-provenance caller/reusable job. The
caller grants the token scope required by the protected callee; ordinary test
jobs retain their existing permissions. Checkout disables persisted credentials.
No candidate `.cargo` configuration, executable, script or dependency is run.

The protected resolver uses GitHub-authenticated API records to check the source
workflow/run/attempt, check suite, protected main branch, successful PostgreSQL
producer job and artifact inventory. The enclosing workflow is still active
while this reusable job runs; the producer job must already be completed and
successful. The protected original source context supplies the expected tested
SHA; API candidate metadata and mutable associated PR metadata cannot override it.

Exactly one candidate must have been created during that attempt's successful
producer-job interval. Missing, expired, ambiguous, wrong-attempt or truncated
inventories fail closed. More than 100 jobs/artifacts needs an explicit reader
extension rather than silently accepting a truncated inventory. Pushes and
manual runs admit only `main`; PRs admit the original `refs/pull/*/merge` context;
merge groups admit the original main-queue context.

The independently fetched GitHub artifact archive digest is checked before
extraction. Extraction accepts bounded regular data files under the known
runtime roots and the narrowly typed NS19 report/archive subtree below. It rejects path escapes, platform-normalized wire names,
symlinks, duplicate entries, extra code and oversized files. The in-artifact
manifest is an inventory hint under those authenticated bytes.

## Protected semantic policy

The verifier is compiled exclusively from the OIDC-selected protected checkout.
The existing exact registry and `POLICY_SOURCE` digest comparisons remain
unchanged. Candidate declarations cannot select another semantic implementation
or authorize a policy transition. The verifier also compares every candidate's
recorded repository/run/attempt/tested SHA against the protected source context.

A PR can remove/replace the reusable invocation, but it cannot produce an
attestation with the authorized protected workflow identity for those bytes.
Evidence consumers must require and verify that attestation independently of
producer-controlled workflow labels or an unsigned candidate upload.

## Staged deployment

This reusable workflow must exist on protected `main` before callers can use
the protected reference. A PR-local relative workflow would move the verifier
and candidate policy together and would defeat this trust boundary.

1. Bootstrap PR: deploy the same-run digest CLI, diagnostic/evidence split,
   reusable workflow, trusted resolver/tests and this runbook to protected
   `main`, without activating the new protected caller or its `Tests` dependency.
   This PR cannot yet claim independent signed evidence. Merge remains a
   separate maintainer action.
2. Activation PR: add the Test Matrix `protected-provenance` call to the now
   deployed protected reusable workflow and make `Tests` require its result.
   The callee uses the original PR merge context while compiling the protected
   bootstrap policy. Retain a real successful Actions run and signed candidate.
   A full combined diff attempted before bootstrap deliberately fails because
   the trusted reusable target is unavailable; do not replace it with PR code.
3. Review policy changes against the unchanged deployed protected oracle. A PR
   changing semantic implementation or producer inventory is expected to fail
   until a maintainer intentionally authorizes and deploys that policy. After
   deployment produce a fresh candidate. Do not restamp an older artifact,
   accept its self-declared policy digest, or weaken exact comparisons.

The old `b7d49630` verifier lacks CI2's required manifest-digest CLI argument
and has a different policy self-hash. It is not a substitute for the deployed
bootstrap verifier. A registry schema change allowing release-level `passed`
requires its own decision; this workflow does not make that change.

## Signed digest and consumer verification

The protected job attests the exact original candidate ZIP only after its
protected semantic verification succeeds. The attestation is retained through
GitHub's independent attestation API and as a signature bundle outside the
candidate archive. Before publishing the signed-evidence artifact, the job
checks its certificate against the exact protected reusable signer/ref, its
immutable signer-policy SHA, the original caller source SHA/ref, GitHub OIDC
issuer and hosted-runner constraint.

Consumers supply their authorized protected policy revision and selected source
run identity independently of downloaded files:

```bash
gh attestation verify runtime-authority-candidate.zip \
  --repo vanyastaff/nebula \
  --cert-identity https://github.com/vanyastaff/nebula/.github/workflows/runtime-authority-provenance.yml@refs/heads/main \
  --signer-digest "$AUTHORIZED_POLICY_SHA" \
  --source-digest "$EXPECTED_TESTED_SHA" --source-ref "$EXPECTED_SOURCE_REF" \
  --cert-oidc-issuer https://token.actions.githubusercontent.com \
  --deny-self-hosted-runners
```

For offline retained signatures add `--bundle runtime-authority-attestation.json`.
Wrong issuer/signer/ref/digest, missing signatures and stale source identities
are failures. After signature admission, compare the candidate's recorded
producer identity with the independently selected source run/attempt and rerun
the protected semantic verifier when the consuming policy requires it. The
certificate's source digest is the original caller source revision; its signer
digest is the protected reusable workflow revision.

The signature proves which protected workflow judged which immutable bytes.
It does not prove a malicious producer reported truthful observations, imply
provider exactly-once behavior, or authorize release-level `passed`.

## Verification

```bash
python3 -m unittest discover -s scripts/tests -p 'test_runtime_authority_provenance.py'
actionlint .github/workflows/runtime-authority-provenance.yml .github/workflows/test-matrix.yml
cargo nextest run -p nebula-xtask
cargo xtask north-star-gates validate
```

Tests execute the actual pre-checkout bootstrap admission code, production
metadata resolver and extractor. They distinguish wrong protected signer/source
claims, mutable PR base/merge refresh, source replay, wrong workflow/repository/
attempt, failed producers, archive replacement and unsafe extraction. Local
mock-OIDC/API checks are not live attestation evidence. The earlier read-only
GitHub API/ZIP experiment validates transport shape only, not a signed source
anchor. Retain live activation evidence before closing #991.


The same protected job also admits NS20 when its OIDC-bound original caller
workflow is `.github/workflows/ci.yml`. This selects only the protected SDK
release policy, successful `SDK release quality` producer and
`sdk-release-quality-candidate` artifact. Caller inputs cannot select a profile.
The protected SDK verifier reads `crates/sdk/Cargo.toml` from the authenticated
GitHub contents API at the original source SHA. Compatibility baseline comes
from the original reusable event base/before, or the authenticated exact source
commit first parent when the event has no baseline. SDK archives contain only
the flat case log inventory and `sdk-release-quality.json`; the policy checks
all five cases and keeps the effective state `partial`. The protected bootstrap
must deploy `scripts/sdk-release-quality.py` with this workflow before activating
the CI caller. No second job receives elevated permissions.


NS19 binary transport is confined to
`NS19/published-manifest-precision/archives/<name-version>.crate`. Only these
regular data files receive a 64 MiB per-file and 512 MiB aggregate budget.
Every other runtime JSON/log file retains its 4 MiB limit and separate 256 MiB
aggregate budget; the SDK profile admits no NS19 binary paths. The runtime ZIP
transport cap is the sum of both payload budgets plus 1 MiB of bounded framing,
not a larger ordinary-file budget. Protected packaging verification must rehash
the exact metadata-derived archive set and parse its actual normalized manifests
before attestation. Package bytes remain data and are never executed by the
protected verifier. Deployment requires the matching protected packaging policy
and its runtime-authority policy fingerprint on main before caller activation.

The admitted NS19 report has the sole path
`NS19/published-manifest-precision/published-manifest-precision.json` and retains
the ordinary 4 MiB JSON budget. The protected job invokes
`cargo xtask packaging verify-archives --report <report> --archive-root <archives>`
before runtime semantic qualification and attestation. The same immutable
protected checkout supplies metadata and `packaging.rs`; the latter belongs to
`POLICY_SOURCE`. Missing report/archives or any independent archive verification
error rejects the runtime evidence job rather than skipping NS19.


Bootstrap integration base is foundation
`7a8917fd7491fc259df03f1a1a589d6e0beaf1b5`. Its lease-chaos fail-empty gate,
CI2 trusted manifest hash and CI3 metadata-driven formatting/fixture isolation
must be retained. This workflow packet does not contain the separately owned
NS19 packaging policy/CLI, NS11/NS21 semantic predicates and typed bundle
inventory, or NS20 SDK policy script. Those reviewed policy sources and matching
producer/registry topology must accompany protected bootstrap deployment before
activation. In particular the packaging source is included in `POLICY_SOURCE`,
and the bundle retains the admitted NS19 report and exact metadata-selected
binary archive inventory at the specified paths. A foundation-only deployment
of this YAML cannot qualify the new policies and is not a completed bootstrap.
The activation-only structural test travels with caller activation; bootstrap
updates only the existing CI2 upload ordering/split regression.
