# Prototype spike plan

**Статус:** concrete plan для validation spike перед writing any spec.

## Почему prototype first

User review нашёл 8 🔴 BROKEN findings в paper design — все касаются type system shape. Paper design **не compilet'ся** в Rust в проверяемой форме; writing spec без validation = гарантированный churn.

**Цель spike:** получить **working Rust crate** where 5-6 realistic credential types, 3 resource types, и 2-3 action types compile и используются. Iterate trait shape пока всё не compile'ится и looks sane.

**Это не production code.** Throwaway crate. После resolution писать proper spec на validated shape.

## Deliverables

1. **Working Cargo crate** `scratch/credential-proto/` с:
   - `src/contract.rs` — Credential trait draft
   - `src/injector.rs` — SchemeInjector trait draft
   - `src/schemes.rs` — 6+ built-in schemes (implementations)
   - `src/credentials.rs` — 5+ concrete Credential impls
   - `src/resources.rs` — 3 Resource patterns (per-request, connection-bound, multi-auth)
   - `src/actions.rs` — 2-3 Action patterns
   - `src/registry.rs` — TypeId → metadata registry
   - `src/accessor.rs` — ctx.credential_at / ctx.credential shape

2. **Design notes** `scratch/credential-proto/NOTES.md`:
   - Each trait iteration attempted
   - What compiled, what failed, why
   - Which findings resolved, which deferred
   - Final shape rationale

3. **Integration test** `scratch/credential-proto/tests/e2e.rs`:
   - Creates 5 credential types
   - Uses each через action → resource → injection
   - Demonstrates capability-based binding (action принимает multiple credential types)
   - Demonstrates multi-credential resource (mTLS + Bearer)
   - Demonstrates multi-step flow (Salesforce JWT atomic)

## Scope — what's IN

### Trait shapes to validate

- `Credential` trait с 4 assoc types + `const CAPS: Capabilities`
- `SchemeInjector` trait с 4 injection methods
- `AnyCredential` object-safe supertrait
- Service trait pattern (`trait BitbucketCredential: Credential + Sealed`)
- Capability markers (`AcceptsBearer`, `AcceptsSigning`, `AcceptsTlsIdentity`, `AcceptsDbConnection`)
- `CredentialRef<C>` + `CredentialGuard<S>` RAII
- `ctx.credential_at(&binding)` API — explicit ref lookup
- `ctx.credential::<C>()` API — type-driven shorthand (validate compile errors at macro level)

### Concrete examples to implement

1. **SlackOAuth2Credential** — per-service OAuth2 (Pattern 1)
2. **BitbucketPatCredential** + **BitbucketAppPasswordCredential** + service trait `BitbucketCredential` (Pattern 2)
3. **AnthropicApiKeyCredential** — API Key Bearer (simple Pattern 1)
4. **AwsSigV4Credential** + **AwsStsCredential** — chainable signing (finding #14 STS chain)
5. **PostgresConnectionCredential** — DB connection scheme
6. **MtlsCredential** — TLS-level injection
7. **SalesforceJwtCredential** — multi-step (atomic only)

### Resources to implement

1. **HttpResource** (per-request injection) — takes `dyn AcceptsBearer` at call time
2. **PostgresPoolResource** (connection-bound) — consumes credential at create, has `on_credential_refresh`
3. **MtlsHttpResource** (dual-auth) — takes `(dyn AcceptsTlsIdentity, dyn AcceptsBearer)`

### Actions to implement

1. **GenericSlackAction** — uses Pattern 1 (concrete `SlackOAuth2Credential`)
2. **GenericBitbucketAction** — uses Pattern 2 (service trait `dyn BitbucketCredential`)
3. **GenericHttpBearerAction** — uses Pattern 3 (capability `dyn AcceptsBearer`)

### Focus — questions to answer

1. **Does `dyn BitbucketCredential` projection to `CredentialGuard<BearerScheme>` compile?**
2. **Does `ctx.credential::<SlackOAuth2Credential>()` compile error at macro level if action has 0 or 2+ slots of that type?**
3. **Does `CredentialRef<dyn AcceptsBearer>` + runtime resolve через TypeId registry actually type-check?**
4. **Does DualAuth<A, B> compile for multi-credential resources? Variadic N?**
5. **Does sealed trait + capability markers через blanket impls actually constrain implementation без blocking plugins completely?**
6. **Does SigningContext::body_hash streaming path work для AWS SigV4 simulation?**
7. **Does credential-builtin crate split compile and resolve correctly?**

## Scope — what's OUT

Explicitly deferred, not included in spike:

- **Real crypto** — MockCrypto для encrypt/decrypt; real AES-256-GCM validation — separate
- **Real HTTP** — `reqwest` replaced с in-memory mock server (wiremock-style)
- **Real storage** — `HashMap` behind trait, not SQLite/Postgres
- **RefreshCoordinator** multi-replica race (#17) — separate spec
- **ProviderRegistry** — stub с hardcoded entries; no admin API
- **Multi-step persistent flow** (#22) — atomic-only start
- **Audit degraded mode** (#29) — stub fail-closed
- **Cache invalidation** — no-op cache
- **Trigger integration** (#35) — out of scope entirely
- **WebSocket events** (#34) — out
- **Schema migration on encrypted rows** (#36) — out
- **Integration с existing nebula-credential** — standalone crate, no attempt to migrate existing code

## Success criteria

Spike is **done** when:

1. Все 7 вопросов (#1-7 выше) answered с concrete code demonstration
2. Все 5 🔴 type-system findings (#1, #2, #3, #14, #32) либо resolved (compile-test shows shape works) либо marked "blocker для spec — needs deeper change"
3. 7+ credential types, 3 resources, 3 actions compile + integration test pass
4. `NOTES.md` documents 2-3 iterations of trait shape with rationale
5. Final `final_trait_shape.rs` file extractable as basis для spec

Spike is **failed** (and we know it's failed) if:

1. Type system shape cannot be made to compile после 3 iterations
2. Compile works но API ergonomics are unusable (e.g. every call needs 5 generic args)
3. Multi-credential resource (#14) cannot be expressed naturally
4. Sealed + plugin tension (#8) cannot be resolved за prototype scope

If failed → **rollback к S1 path**: accept current architecture после P6-P11 (не trying "beat n8n"), just finish rollout cleanup. Cosmetic improvements only.

## Execution plan

**Agent:** `rust-senior`, dispatch mode, isolation `worktree` для keeping scratch out of main branch.

**Prompt sketch** для dispatch:

```
Создай throwaway Cargo crate 'scratch/credential-proto' для validation credential architecture type system.

Context: мы редизайним nebula-credential; есть paper design в
docs/superpowers/drafts/2026-04-24-credential-redesign/01-type-system-draft.md
+ 04-schemes-catalog.md. User review нашёл 8 BROKEN type-system findings
(in 05-known-gaps.md §1-3, 8, 14, 32) которые не могут быть resolved на paper.

Goal: validate trait shape compiles + usable на real examples. Это НЕ
production code — throwaway.

Specific questions to answer (из 06-prototype-plan.md §Success criteria):
1. dyn service trait projection
2. ctx.credential::<C>() compile error при ambiguity
3. CredentialRef<dyn Capability> runtime resolve
4. DualAuth multi-credential resource
5. Sealed + plugin tension
6. SigningContext body_hash для streaming
7. credential-builtin crate split

Deliverables:
- scratch/credential-proto/ working Cargo crate
- 7+ credential examples (см. §Concrete examples to implement)
- 3 resources, 3 actions
- Integration test demonstrating all patterns
- NOTES.md с iterations + rationale + failed attempts

Scope — только trait shape validation. Out: real crypto, HTTP, storage, RefreshCoordinator, Triggers.

Iterate до compile + usable API. Если после 3 iterations не работает — document failure, suggest fundamental redirection.

Estimated: 3-5 days equivalent work.
```

**Dispatch mode:** isolation: "worktree" — создать separate worktree для не ломать main.

**Parallel:** НЕТ parallel agents для этого spike. Sequential, один rust-senior, иначе shape дифференцируется между parallel attempts.

## После spike — three paths

1. **Spike succeeded:** proceed к writing spec using validated trait shape. Real H0 spec с confidence. ~1 неделя writing.

2. **Spike partial success:** some findings resolved, others marked for follow-up. Write narrower spec — e.g. "Pattern 1 + 2 type system only, Pattern 3 (capability-only binding) deferred". Build incrementally.

3. **Spike failed:** current architecture stays. Finish rollout cleanup (P12 Finishing). Document redirection в ADR "why we didn't redesign credential в 2026-04".

Все три — valid outcomes. Critical — не write spec before spike.

## Что нужно от user прежде чем dispatch

1. **Confirm spike vs alternative paths** (finishing / narrow-slice / defer entirely)
2. **Approve scratch/ location** или alternative (e.g., separate branch `prototype/credential-redesign`)
3. **Time budget** — 3-5 days agent work OK? Если нужно faster — narrower scope (fewer credential types, fewer resources)
4. **Failure tolerance** — если spike fails, accept redirection to S1 finishing? Или force redesign regardless?

Waiting for these before dispatching agent.
