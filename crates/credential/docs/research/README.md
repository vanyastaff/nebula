# Credential design — conference research (primary sources)

Raw transcripts behind [`../CONFERENCE.md`](../CONFERENCE.md) and the
[`../DESIGN.md`](../DESIGN.md) §21–§24 corrections. Captured 2026-06-12.

Each "seat" is one subagent: six industry architects grounded in their real
codebase via DeepWiki (Temporal, n8n, Apache Airflow, Dagster, Prefect, Windmill,
Kestra, Restate, HashiCorp Vault, AWS SDK for Rust) plus adversarial critics
grounded in `crates/credential/src`.

| File | Round | Seats |
|------|-------|-------|
| [round1-players-and-critics.md](round1-players-and-critics.md) | 1 | first pass — players (Airflow/Windmill/Vault/AWS landed; others rate-limited) |
| [round1-critics-retry.md](round1-critics-retry.md) | 1 | 4 adversarial critics (arch/types/security/DX) + n8n + Temporal re-run |
| [round2-differentiation-scalability.md](round2-differentiation-scalability.md) | 2 | all 10 players + scale-critic + moat-skeptic on differentiation + scalability |

These are evidence, not canon. The decisions they drove live in `CONFERENCE.md`
(synthesis) and `DESIGN.md` (§21 adoptions, §22 industry failure matrix, §23
differentiation/moat, §24 scalability walls).
