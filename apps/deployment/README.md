# nebula-deployment

Shared assembly for the first-party server and worker applications. This
unpublished package belongs to the application layer, not the SDK or a reusable
product-layer runtime.

`CoreRelease` admits the linked core plugin and freezes its exact artifact/runtime
identity once. The API uses that registry for validation and dispatch; worker
assembly installs the matching plugin and registry in its engine.

`worker` composes the engine, credential observation, stored resource activation,
durable fanout, status projection and recovery into the reusable worker runtime.
Callers supply admitted storage roles and read-only credential resolution. The
package neither opens databases nor reads environment variables or OS signals.
Shared storage does not transfer aggregate write authority.

The standalone worker owns its environment adapter in `apps/worker/src/config.rs`.
The server owns HTTP, identity, credential commands and its admitted deployment
pool. The ordinary server starts an in-process execution worker by default and
can explicitly delegate execution to separate PostgreSQL workers. Both use this
assembly. The server's `runtime-repair-red` profile additionally injects optional
clock and event inputs behind its matching feature for evidence only.

Validation uses the worker smoke scenarios and server profile lifecycle tests in
addition to this package's tests. A successful assembly test does not establish a
complete local desktop or Docker deployment.
