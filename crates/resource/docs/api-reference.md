# nebula-resource — API Reference

The authoritative, always-current API reference for `nebula-resource` is the
generated rustdoc. A hand-maintained signature mirror drifts from the code;
this page intentionally does not duplicate it.

## Where to look

- **Generated rustdoc** — every public type, trait, method, and signature,
  with the doc comments that explain contracts and invariants:

  ```sh
  cargo doc -p nebula-resource --open
  ```

- **[`../README.md`](../README.md)** — the shipped public surface in prose:
  the `Resource` trait and its associated types, the two topologies
  (`Pooled` and `Resident`), the single
  `Manager::register(RegistrationSpec { … })`
  registration funnel, the structural `SlotIdentity` cross-tenant barrier
  (`Unbound` / `Structural`), the `acquire_<topology>` /
  `acquire_<topology>_for_identity` acquire family and `acquire_erased_for`,
  the engine-driven slot operations (`refresh_slot` /
  `refresh_slot_for_identity`, `revoke_slot` / `revoke_slot_for_identity`),
  `reload_config`, `lookup`, and `ResourceGuard`.

## Topic-specific docs

| Topic | Doc |
|-------|-----|
| Topology selection and authoring | [`topology-reference.md`](topology-reference.md) |
| Pool internals | [`pooling.md`](pooling.md) |
| Recovery / thundering-herd serializer | [`recovery.md`](recovery.md) |
| Event catalog | [`events.md`](events.md) |
| Concrete integration adapters | [`adapters.md`](adapters.md) |

See also [`README.md`](README.md) in this directory for the documentation
map.
