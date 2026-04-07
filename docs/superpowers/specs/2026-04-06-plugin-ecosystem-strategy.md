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
| **Official** | ✓ | Under `nebula-plugins/` org, reviewed by core team |
| **Essential** | ✓✓ | Official + funded maintenance + backward compat CI |

---

## 2.5. Everything Open Source — No DRM, No Paid Plugins

**All plugins are open source. No paid plugins. No marketplace. No DRM.**

Why:
- One canonical plugin per service → nothing to compete on → nothing to sell
- Plugin Fund pays developers → they don't need to sell
- No file protection needed → no piracy problem
- No billing infrastructure needed → massive simplification
- No license key system → no support tickets about keys
- Community can fork, fix, contribute back → ecosystem health

What we DON'T need (removed from strategy):
- ~~Revenue split (85/15)~~
- ~~Stripe Connect billing integration~~
- ~~Paid plugins / premium connectors~~
- ~~DRM / license verification~~
- ~~Marketplace infrastructure (ratings, reviews, disputes)~~

What we DO need:
- **Plugin Fund** — 10% of cloud revenue → bounties + maintenance grants
- **Bounty board** — public list: "Salesforce connector: $5K bounty" 
- **Maintainer grants** — $500-2K/month for active maintainers of Essential plugins
- **Hub page** — catalog, not store. README + install + docs per plugin

---

## 3. One Plugin Per Service — Collaborative Model

### Anti-pattern: VSCode/WordPress marketplace
500 competing Slack plugins = fragmented quality, user confusion, abandoned forks. **Nebula rejects this.**

### Nebula approach: ONE canonical plugin per service
- **One** `nebula-plugin-slack`, **one** `nebula-plugin-stripe`, **one** `nebula-plugin-postgres`
- Multiple contributors work TOGETHER on the same plugin (like a crate, not a marketplace listing)
- Plugin Hub page = catalog of canonical plugins, not a competitive marketplace

### How developers contribute
1. **Claim or join:** Developer finds `nebula-plugin-salesforce` doesn't exist → creates RFC. Or exists → opens PR to add actions.
2. **Collaborative development:** Multiple developers from different companies co-maintain one plugin. Like how `serde` has many contributors, not 50 competing serialization crates.
3. **Crate ownership:** Plugin published to crates.io under `nebula-plugins` org. Multiple maintainers with publish rights.

### Nebula Plugin Fund
Open-source fund (from cloud revenue) pays contributors:
- **Bounties:** $1K-10K for implementing a priority plugin from scratch
- **Maintenance grants:** $500/month for maintaining critical plugins (ongoing)
- **Feature bounties:** $200-2K for adding specific actions to existing plugins

```
Cloud revenue → 10% to Plugin Fund
Plugin Fund → bounties + maintenance grants + infrastructure
```

### Why this works better
| Marketplace model | Nebula model |
|-------------------|-------------|
| 50 Slack plugins, 3 good | 1 Slack plugin, excellent |
| Authors compete, fragment effort | Authors collaborate, compound effort |
| Abandoned plugins = user pain | Community maintains, fund sustains |
| "Which Slack plugin?" confusion | One canonical choice, always |
| Revenue split creates incentive to fork | Grants create incentive to contribute |

### Hub page (not marketplace)
- **nebula.dev/plugins** — catalog of all canonical plugins
- Each plugin: README, actions list, credential requirements, install instructions
- **No ratings, no competing listings** — one entry per service
- "Want a plugin that doesn't exist? → Start an RFC"
- "Want to improve an existing plugin? → Open a PR"

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

### Maintainer Succession
Since each plugin has ONE canonical repo with multiple maintainers:
1. Maintainer goes inactive → other co-maintainers continue
2. All maintainers inactive 3 months → core team posts "maintainer wanted" call
3. No new maintainer 6 months → core team adopts or archives
4. Critical plugins (Essential 50) → always have ≥2 active maintainers

### Support Triage
- Plugin errors carry `plugin_key` in error metadata
- Nebula Cloud SLA covers engine + Essential 50 plugins
- Plugin issues filed on plugin repo (e.g., `nebula-plugins/slack/issues`)
- Error messages: "This error originates from the `slack` plugin. Report: github.com/nebula-plugins/slack/issues"

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
