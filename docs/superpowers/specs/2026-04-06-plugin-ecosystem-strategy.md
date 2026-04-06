# Nebula — Plugin Ecosystem & Monetization Strategy

> From conference Round 13: WordPress, GitLab, Shopify, JetBrains, Stripe panelists.

---

## 1. Plugin Distribution Roadmap

| Phase | Format | Install experience |
|-------|--------|-------------------|
| v1 | Rust crate in monorepo | `cargo add nebula-plugin-slack` → recompile |
| v1.1 | Pre-built binary bundles | Download `nebula-with-slack-postgres-http` binary |
| v2 | WASM plugins | `nebula plugin install slack` → runtime load, no recompile |
| v3 | Multi-language SDK | Write plugin in Python/TS → compile to WASM → install |

### Plugin Manifest (v1.1)

```toml
# nebula-plugin.toml
[plugin]
name = "slack"
version = "1.0.0"
nebula_version = ">=1.0, <2.0"
author = "Nebula Team"
license = "MIT"
description = "Slack messaging integration"

[actions]
"slack.send_message" = { version = "1.0" }
"slack.create_channel" = { version = "1.0" }

[credentials]
"slack_oauth2" = { pattern = "OAuth2" }

[data_tags]
produces = ["comm.slack.message"]
consumes = ["text", "json"]
```

---

## 2. Quality & Certification

### Automated Pipeline (all plugins)
1. `cargo-audit` — known CVE check
2. `cargo-deny` — license + ban check
3. `cargo-geiger` — unsafe code count
4. Integration test suite — plugin compiles + basic smoke test against Nebula release candidate

### Certification Tiers
| Tier | Badge | Requirements |
|------|-------|-------------|
| **Community** | None | Passes automated pipeline |
| **Verified** | ✓ | + manual code review by core team |
| **Certified** | ✓✓ | + backward compat test matrix + SLA on maintenance |

---

## 3. Monetization Model

### For Nebula (the project)
- **Open source engine:** MIT/Apache-2.0. Free forever.
- **Nebula Cloud:** Hosted service. Per-execution pricing.
- **Enterprise:** SSO, RBAC, audit, SLA, compliance attestation.

### For Plugin Developers
- **Free plugins:** Hosted in marketplace for free. No commission.
- **Paid plugins:** Billing via Stripe Connect integration.
  - v1: Developer handles own billing (link to external site)
  - v2: Nebula Marketplace billing (85% to developer, 15% platform fee)
- **Bounty program:** $1K-5K for high-priority connectors (Salesforce, HubSpot, Stripe, etc.)

### Revenue Split
```
Plugin sale $49/month
  → $41.65 to developer (85%)
  → $7.35 to Nebula Marketplace (15%)
    → CDN, hosting, review staff, support infrastructure
```

---

## 4. Ecosystem Sustainability

### "Essential 50" Program
Core team commits to maintaining 50 essential plugins:
- HTTP, SQL (Postgres, MySQL, SQLite), Slack, Email, Webhook
- AWS (S3, Lambda, SQS), GCP, Azure
- Stripe, GitHub, GitLab, Jira, Notion
- OpenAI, Anthropic, Google AI
- Redis, MongoDB, Kafka, RabbitMQ
- Twilio, SendGrid, Discord
- Salesforce, HubSpot (if funded)

### Abandonment Policy
1. Plugin inactive 6 months → "Maintenance Mode" warning
2. Inactive 12 months → Community can fork under same name
3. Critical plugins (Essential 50) → Core team adopts

### Support Triage
- Plugin errors carry `plugin_key` in error metadata
- Nebula Cloud SLA covers engine + Essential 50 plugins
- Community plugins: support via plugin author's channels
- Error messages: "This error originates from the `slack` plugin. Contact: github.com/nebula-plugins/slack/issues"

---

## 5. Backward Compatibility Testing

### Partner CI Matrix (v1.1)
Every Nebula release candidate triggers:
1. Build all Essential 50 plugins against RC
2. Run each plugin's integration test suite
3. Report: PASS / FAIL / DEGRADED per plugin
4. RC blocked if any Essential 50 plugin fails

### Plugin Author Contract
- Plugin declares `nebula_version = ">=1.0, <2.0"` in manifest
- Nebula major version = potential breaking change → plugin author tests
- Nebula minor version = additive only → plugins MUST still work
- Violation = bug in Nebula, not in the plugin

---

## 6. Not In Scope (Future)

- Visual plugin builder (drag-and-drop action creation)
- Plugin analytics dashboard (installs, usage, revenue)
- A/B testing for plugin marketplace listings
- Enterprise private plugin registries
- Plugin dependency resolution (plugin A requires plugin B)
