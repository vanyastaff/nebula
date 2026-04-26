# aofctl — Architectural Issues

Source: `gh issue list --repo agenticdevops/aof --state all --limit 30`

## Issues with architectural significance

| # | Title | State | Labels | URL |
|---|-------|-------|--------|-----|
| 47 | Horizontal scaling — Redis/NATS message queue | OPEN | P1 | https://github.com/agenticdevops/aof/issues/47 |
| 46 | Multi-org support — Per-org credentials | OPEN | P1 | https://github.com/agenticdevops/aof/issues/46 |
| 22 | Config hot-reload — No restart required | OPEN | P2 | https://github.com/agenticdevops/aof/issues/22 |
| 74 | Structured I/O (Input/Output Schemas) similar to Agno | CLOSED (fixed) | enhancement | https://github.com/agenticdevops/aof/issues/74 |
| 71 | MCP Server Catalog | OPEN | P0, type/docs, type/mcp | https://github.com/agenticdevops/aof/issues/71 |
| 84 | Agent Config Parser Error: memory field type mismatch | CLOSED (bug) | bug | https://github.com/agenticdevops/aof/issues/84 |
| 77 | Peer mode forces consensus when agents provide complementary results | CLOSED | bug, enhancement | https://github.com/agenticdevops/aof/issues/77 |
| 89 | Daemon agent loading fails on non-Agent YAML files (Trigger, Fleet) | CLOSED | bug | https://github.com/agenticdevops/aof/issues/89 |
| 95 | library:// URI fails when running from different directory | CLOSED | bug | https://github.com/agenticdevops/aof/issues/95 |
| 92 | GitHub webhook responses not posted back to PR/issue | CLOSED | bug | https://github.com/agenticdevops/aof/issues/92 |

## Summary
The dominant issues reveal two architectural gaps:
1. **No horizontal scaling** — single binary, no distributed coordination (issue #47 is P1 open)
2. **No multi-org isolation** — single credential namespace, planned as future feature (#46)
3. **Config parsing edge cases** — YAML deserialization surprises for complex fields (#84, #89, #95)
4. **Multi-agent consensus over-reach** — peer mode applies consensus even when outputs are not competing (#77)
