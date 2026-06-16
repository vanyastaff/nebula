# Memory Index

## Project
- [plugin-resource-erasure-asymmetry](project_plugin_resource_erasure_asymmetry.md) — Plugin ResourceDescriptor is metadata-only vs ActionFactory constructs; plugin resources can't reach live ManagedResource w/o out-of-band KindActivator (M12.4 frontier); + AnyResource stale-name doc rot
- [plugin-contribution-freeze](project_plugin_contribution_freeze.md) — FREEZE decision 2026-06-15: merged erased ResourceFactory (move KindActivator→nebula-resource +metadata()) + load/unload protocol; closes M12.4; KindActivator NOT Clone; #[resource(..)] retired; ADR candidate
- [plugin-dependency-resolver-d6](project_plugin_dependency_resolver_d6.md) — ADR-0095 D6 resolver: field/resolver/error placement + why wiring is a follow-up (no batch plugin-load call site exists)
- [adr0095-u3-durable-emitter](project_adr0095_u3_durable_emitter.md) — ADR-0095 U3 plan: DurableExecutionEmitter→nebula-engine (only crate w/ both edges); latent-emitter scoping fork (DispatchRouting via D1); CORRECTION: no shared Start-msg builder (ControlMsg≠JobDispatchMsg), emitter calls JobDispatchMsg::new direct
- [adr0095-trigger-dispatch-slice](project_adr0095_trigger_dispatch_slice.md) — ADR-0095 vertical slice (supersedes latent-U3 framing): emitter MUST create Created exec-row AFTER winning claim (else Duplicate orphans / resume_execution Rejected); no prod emitter install site (harness-scoped); EngineExecutionSink+RoutingResolver in engine (engine→orchestrator acyclic)
