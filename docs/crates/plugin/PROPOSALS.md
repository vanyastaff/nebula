# Proposals

## P-001: Discovery API

**Type:** Non-breaking (additive)

**Motivation:** Load all plugins from a directory or from config without manual register of each.

**Proposal:** Optional discovery API: e.g. discover(path) or discover_from_config(config) that scans path or reads config, loads (static or dynamic) and registers into provided registry. Document in ROADMAP Phase 3.

**Expected benefits:** Easier deployment and operator experience.

**Costs:** File I/O, async or blocking decision; config format to define.

**Risks:** Discovery order or duplicate keys if not careful.

**Status:** Draft

---

## P-002: Version Resolution Policy

**Type:** Non-breaking

**Motivation:** PluginVersions holds multiple versions per key; need policy for which version engine/API use.

**Proposal:** Document resolution: latest compatible, or pinned version; add get(key, version) or resolve(key) that returns single plugin instance by policy.

**Expected benefits:** Clear behavior for versioned plugins.

**Costs:** API and doc maintenance.

**Status:** Draft
