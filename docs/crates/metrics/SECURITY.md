# Security

## Threat Model

- **Assets:** Metric values (counts, durations, gauges); metric names; labels. May reveal execution patterns, error rates, resource usage.
- **Trust boundaries:** Metrics are process-internal; export endpoint (future) exposes data to Prometheus/collectors. Scrape endpoint may be network-accessible.
- **Attacker capabilities:** If scrape endpoint exposed, attacker can read metrics; no auth by default in Prometheus scrape.

## Security Controls

- **Authn/authz:** Prometheus scrape typically unauthenticated; restrict `/metrics` to internal network or add auth in api layer.
- **Isolation/sandboxing:** Metrics recording in same process; no isolation.
- **Secret handling:** Metric labels must not contain credentials, PII. Document label sanitization.
- **Input validation:** Metric names from code, not user input; avoid user-controlled labels with high cardinality.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|------------|-----------|----------|
| Scrape endpoint exposed publicly | Restrict to internal network; auth | Network audit | Firewall; auth |
| High cardinality DoS | Limit label cardinality; bounded histograms | Monitor metric count | Alert; cap labels |
| Sensitive data in labels | Code review; no secrets in labels | Audit | Remove; redact |
| Metric injection | Names from code only | — | — |

## Security Requirements

- **Must-have:** No credentials or PII in metric names/labels; scrape endpoint access controlled in production.
- **Should-have:** Document label sanitization; cardinality limits.

## Security Test Plan

- **Static analysis:** No unsafe; audit label sources.
- **Dynamic tests:** Verify no secrets in export output.
- **Fuzz/property tests:** Optional; metric name/label fuzz.
