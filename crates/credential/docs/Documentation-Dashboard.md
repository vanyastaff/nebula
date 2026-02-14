---
title: nebula-credential Documentation Dashboard
tags: [dashboard, meta, documentation-status]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
version: 1.1.0
completion: 80/102 tasks (78%)
---

# nebula-credential Documentation Dashboard

> [!NOTE] Purpose
> This dashboard provides an overview of all documentation pages for `nebula-credential`, tracking completion status, quality metrics, and cross-reference coverage.

## Quick Links

- [[README|Main Documentation]]
- [[Getting-Started/Quick-Start|Quick Start Guide]]
- [[Architecture|System Architecture]]
- [[Reference/API-Reference|API Reference]]
- [[Reference/Glossary|Glossary]]

---

## Documentation Status by Priority

### Priority 1 (P1) - Critical Pages

```dataview
TABLE 
  status as Status,
  lang as Language,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE contains(tags, "p1") OR contains(tags, "priority-1")
SORT status ASC, last_updated DESC
```

**Expected P1 Pages**:
- Quick Start Guide
- Core Concepts
- Installation Guide
- Common Examples (OAuth2, Database, AWS)

### Priority 2 (P2) - Important Pages

```dataview
TABLE 
  status as Status,
  lang as Language,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE contains(tags, "p2") OR contains(tags, "priority-2")
SORT status ASC, last_updated DESC
```

### Priority 3 (P3) - Supplementary Pages

```dataview
TABLE 
  status as Status,
  lang as Language,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE contains(tags, "p3") OR contains(tags, "priority-3")
SORT status ASC, last_updated DESC
```

---

## Status Overview

### Draft Pages (Needs Completion)

```dataview
TABLE 
  file.folder as Folder,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE status = "draft"
SORT file.folder ASC, file.name ASC
```

> [!WARNING] Action Required
> Draft pages need content completion, validation, and promotion to `in-progress` or `published` status.

### In-Progress Pages (Under Review)

```dataview
TABLE 
  file.folder as Folder,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE status = "in-progress"
SORT last_updated DESC
```

### Published Pages (Complete)

```dataview
TABLE 
  file.folder as Folder,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE status = "published"
SORT last_updated DESC
```

### Outdated Pages (Needs Update)

```dataview
TABLE 
  file.folder as Folder,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE status = "outdated"
SORT last_updated ASC
```

> [!DANGER] Urgent
> Outdated pages contain information that may be incorrect due to upstream changes. Update immediately.

---

## Content Type Coverage

### Getting Started Pages

```dataview
TABLE 
  status as Status,
  estimated_reading as "Reading Time",
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/Getting-Started"
SORT file.name ASC
```

**Coverage Target**: 3-5 pages (Quick Start, Core Concepts, Installation, First Example)

### Examples

```dataview
TABLE 
  status as Status,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/Examples"
SORT file.name ASC
```

**Coverage Target**: 15+ examples (OAuth2, SAML, LDAP, JWT, mTLS, Kerberos, Database, AWS, API Keys)

### How-To Guides

```dataview
TABLE 
  status as Status,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/How-To"
SORT file.name ASC
```

**Coverage Target**: 5-7 guides (Store, Retrieve, Rotate, Configure Caching, Enable Audit Logging)

### Integration Guides

```dataview
TABLE 
  status as Status,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/Integrations"
SORT file.name ASC
```

**Coverage Target**: 5 provider guides (Local, AWS, Vault, Azure, K8s) + Migration Guide

### Advanced Topics

```dataview
TABLE 
  status as Status,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/Advanced"
SORT file.name ASC
```

**Coverage Target**: 10+ pages (Security, Key Management, Compliance, Performance, Custom Providers)

### Troubleshooting

```dataview
TABLE 
  status as Status,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/Troubleshooting"
SORT file.name ASC
```

**Coverage Target**: 7+ guides (Common Errors, Decryption Failures, OAuth2 Issues, Rotation Failures, Scope Violations)

### Reference

```dataview
TABLE 
  status as Status,
  tags,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential/Reference"
SORT file.name ASC
```

**Coverage Target**: API Reference, Configuration Options, Glossary

---

## Language Coverage

### Russian Pages

```dataview
TABLE 
  file.folder as Folder,
  status as Status,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE lang = "ru"
SORT file.folder ASC, file.name ASC
```

### English Pages

```dataview
TABLE 
  file.folder as Folder,
  status as Status,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE lang = "en"
SORT file.folder ASC, file.name ASC
```

### Bilingual Coverage Gap

> [!TIP] Translation Priority
> P1 pages (Getting Started, Core Examples) should have both RU and EN versions. Use `-RU.md` suffix for Russian variants.

**Translation Targets**:
- [ ] Quick-Start.md → Quick-Start-RU.md
- [ ] Core-Concepts.md → Core-Concepts-RU.md
- [ ] OAuth2 examples (GitHub, Google) → Russian versions
- [ ] Database examples (PostgreSQL, MySQL) → Russian versions
- [ ] AWS examples → Russian versions

---

## Quality Metrics

### Pages with Code Examples

```dataview
TABLE 
  file.folder as Folder,
  status as Status
FROM "02-Crates/nebula-credential"
WHERE contains(tags, "example") OR contains(tags, "code-example")
SORT file.folder ASC, file.name ASC
```

**Target**: All example pages must have complete, runnable code with prerequisites and expected output.

### Pages with Diagrams

```dataview
TABLE 
  file.folder as Folder,
  status as Status
FROM "02-Crates/nebula-credential"
WHERE contains(tags, "diagram") OR contains(tags, "mermaid")
SORT file.folder ASC, file.name ASC
```

**Target**: Architecture pages, OAuth2 flows, SAML flows, state machines should include Mermaid diagrams.

### Recently Updated Pages (Last 7 Days)

```dataview
TABLE 
  file.folder as Folder,
  status as Status,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE last_updated >= date(today) - dur(7 days)
SORT last_updated DESC
```

### Stale Pages (>90 Days Since Update)

```dataview
TABLE 
  file.folder as Folder,
  status as Status,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE last_updated < date(today) - dur(90 days) AND status = "published"
SORT last_updated ASC
```

> [!WARNING] Review Needed
> Pages not updated in 90+ days may contain outdated information. Schedule review.

---

## Cross-Reference Health

### Pages by Outgoing Link Count

```dataview
TABLE 
  length(file.outlinks) as "Outgoing Links",
  status as Status,
  file.folder as Folder
FROM "02-Crates/nebula-credential"
WHERE file.name != "Documentation-Dashboard" AND file.name != "README"
SORT length(file.outlinks) ASC
LIMIT 20
```

> [!TIP] Constitution Requirement
> Every concept page MUST have ≥3 outgoing wikilinks. Pages with <3 links need more cross-references.

### Orphan Pages (No Incoming Links)

```dataview
TABLE 
  file.folder as Folder,
  status as Status,
  last_updated as "Last Updated"
FROM "02-Crates/nebula-credential"
WHERE length(file.inlinks) = 0 AND file.name != "README" AND file.name != "Documentation-Dashboard"
SORT file.folder ASC, file.name ASC
```

> [!DANGER] Action Required
> Orphan pages are not discoverable. Add links from relevant pages or navigation hubs.

---

## Completion Statistics

### Overall Progress

```dataview
TABLE WITHOUT ID
  choice(status = "published", "✅", choice(status = "in-progress", "🔄", choice(status = "draft", "📝", "❌"))) as "",
  count(rows) as "Count"
FROM "02-Crates/nebula-credential"
WHERE file.name != "Documentation-Dashboard"
GROUP BY status
SORT status DESC
```

**Legend**:
- ✅ Published (Complete)
- 🔄 In Progress (Under development)
- 📝 Draft (Needs work)
- ❌ Outdated (Needs update)

### Coverage by Folder

```dataview
TABLE WITHOUT ID
  file.folder as "Folder",
  count(rows) as "Total Pages",
  sum(choice(status = "published", 1, 0)) as "Published",
  sum(choice(status = "in-progress", 1, 0)) as "In Progress",
  sum(choice(status = "draft", 1, 0)) as "Draft"
FROM "02-Crates/nebula-credential"
WHERE file.name != "Documentation-Dashboard"
GROUP BY file.folder
SORT file.folder ASC
```

---

## Action Items

### High Priority Tasks

- [ ] Complete all P1 Getting Started pages (Quick Start, Core Concepts, Installation)
- [ ] Validate Quick Start guide with 3 test users (<10 minute completion)
- [ ] Create OAuth2, Database, and AWS examples for US2
- [ ] Add Mermaid diagrams to Architecture.md (credential lifecycle state machine)
- [ ] Add bilingual (RU) versions of all P1 pages

### Medium Priority Tasks

- [ ] Complete all integration guides (AWS, Vault, Azure, K8s, Local)
- [ ] Create rotation guides with blue-green pattern examples
- [ ] Add troubleshooting guides for common errors
- [ ] Create API Reference from data-model-code.md
- [ ] Validate all wikilinks resolve correctly

### Low Priority Tasks

- [ ] Add "See Also" sections with 5-7 links to all pages
- [ ] Create advanced security topics (compliance, threat model)
- [ ] Add custom provider development guide
- [ ] Create performance tuning guide
- [ ] Spell check and grammar check all published pages

---

## Maintenance Schedule

### Daily
- Monitor recent updates
- Check for broken links in new pages
- Validate frontmatter on new pages

### Weekly
- Review draft pages and promote ready ones to in-progress
- Update this dashboard with new pages
- Check for stale pages (>30 days)

### Monthly
- Audit tag usage and consolidate duplicates
- Review orphan pages and add cross-references
- Update translation progress

### Quarterly
- Full documentation audit
- Update version numbers for upstream dependencies
- Review and update outdated pages
- Validate all code examples still compile

---

## See Also

- [[README|nebula-credential Documentation Home]]
- [[Architecture|System Architecture]]
- [[Reference/Glossary|Glossary of Terms]]
- [[Getting-Started/Quick-Start|Quick Start Guide]]
- [[Examples/OAuth2-Flow|OAuth2 Authentication Examples]]
