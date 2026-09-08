//! Firewall-coverage conformance: every `&Scope`-keyed port trait in
//! `nebula-storage-port` MUST have a `Scoped*` decorator in
//! `nebula-tenancy` (spec §6.2, threat model §6.1).
//!
//! `crate::lib` asserts the multi-tenancy firewall is closed *by
//! construction*. That is only true if **every** port trait whose
//! signature carries a caller-supplied `&Scope` is wrapped by a
//! scope-substituting decorator. A raw `Arc<dyn …Store>` handed to a
//! non-HTTP consumer with no `Scoped*` wrapper lets that consumer pass an
//! arbitrary `&Scope` — a cross-tenant read on `get`/`list` and a
//! cross-tenant write on `create`/`update`/`soft_delete` (BOLA / IDOR).
//!
//! The test parses every public trait in `storage-port/src/store`, rejects
//! module-level macros and renamed `Scope` imports that would hide a port from
//! the inventory, and requires every discovered trait to have one explicit
//! classification. Generic `assert_scoped::<Decorator, dyn PortTrait>()`
//! bounds then prove each scope-keyed decorator implements the exact port.
//!
//! Parent-id-keyed identity stores (no `&Scope` in the signature) are a
//! *different* authorization model and are deliberately enumerated in the
//! checklist below with the rationale for why a `Scoped*` is not
//! applicable, so the decision is explicit rather than an omission.

use std::sync::Arc;

use nebula_storage_port::Scope;
use nebula_storage_port::store::{
    CheckpointStore, ControlQueue, ExecutionJournalReader, ExecutionStore, ExecutionTurnHandoff,
    IdempotencyGuard, IdempotencyStore, NodeResultStore, OperationLedger,
    OperationLedgerAdjudicator, ResourceStore, ResumeTokenStore, StartAcceptanceStore,
    TriggerStore, WebhookActivationStore, WorkflowStore, WorkflowVersionStore,
};
use nebula_tenancy::{
    ScopedCheckpointStore, ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore,
    ScopedExecutionTurnHandoff, ScopedIdempotencyGuard, ScopedIdempotencyStore,
    ScopedNodeResultStore, ScopedOperationLedger, ScopedOperationLedgerAdjudicator,
    ScopedResourceStore, ScopedResumeTokenStore, ScopedStartAcceptanceStore, ScopedTriggerStore,
    ScopedWebhookActivationStore, ScopedWorkflowStore, ScopedWorkflowVersionStore,
};
use syn::{FnArg, GenericArgument, Item, PathArguments, TraitItem, Type, UseTree};

/// Compile-time proof that `D` is a scope-substituting decorator for the
/// object-safe port trait `P`: it is `Send + Sync` (usable as
/// `Arc<dyn P>` behind the firewall) and constructible from
/// `(Arc<dyn P>, Scope)` — i.e. it binds a tenant `Scope`. That `D`
/// actually *implements* `P` (not merely wraps it) is proven separately
/// and per-port by the `Arc<D> -> Arc<dyn P>` coercion `const` the
/// `scope_decorator!` macro emits; `ScopeDecorator` alone is only a
/// constructor shim and cannot establish the port impl.
///
/// If a `&Scope`-keyed port trait is added without a matching decorator,
/// the corresponding `assert_scoped` call below fails to compile with an
/// unsatisfied-bound error naming the missing `Scoped*` type.
fn assert_scoped<D, P>()
where
    P: ?Sized + Send + Sync + 'static,
    D: ScopeDecorator<P> + Send + Sync + 'static,
{
}

/// Marker every `Scoped*` decorator satisfies for its port trait: it can
/// be built from a raw `Arc<dyn P>` plus the tenant `Scope` it binds.
trait ScopeDecorator<P: ?Sized> {
    #[expect(dead_code)] // guard-justified: linked only for the type-bound proof above.
    fn bind(inner: Arc<P>, scope: Scope) -> Self;
}

macro_rules! scope_decorator {
    ($decorator:ty, $port:path) => {
        impl ScopeDecorator<dyn $port> for $decorator {
            fn bind(inner: Arc<dyn $port>, scope: Scope) -> Self {
                <$decorator>::new(inner, scope)
            }
        }

        // Compile-time proof that the decorator *is* the port trait, not
        // merely constructible from one. The unsized coercion
        // `Arc<$decorator>` -> `Arc<dyn $port>` only type-checks when
        // `$decorator: $port` (and `dyn $port` is object-safe), so if a
        // `Scoped*` keeps its constructor but drops its
        // `impl $port for $decorator`, this fails to compile — closing the
        // gap where `ScopeDecorator` alone (a `new` shim) could not prove
        // the firewall actually substitutes the scope on the port surface.
        const _: fn(::std::sync::Arc<$decorator>) -> ::std::sync::Arc<dyn $port> = |d| d;
    };
}

scope_decorator!(ScopedExecutionStore, ExecutionStore);
scope_decorator!(ScopedCheckpointStore, CheckpointStore);
scope_decorator!(ScopedWorkflowStore, WorkflowStore);
scope_decorator!(ScopedWorkflowVersionStore, WorkflowVersionStore);
scope_decorator!(ScopedNodeResultStore, NodeResultStore);
scope_decorator!(ScopedIdempotencyStore, IdempotencyStore);
scope_decorator!(ScopedIdempotencyGuard, IdempotencyGuard);
scope_decorator!(ScopedControlQueue, ControlQueue);
scope_decorator!(ScopedExecutionJournalReader, ExecutionJournalReader);
scope_decorator!(ScopedWebhookActivationStore, WebhookActivationStore);
scope_decorator!(ScopedResourceStore, ResourceStore);
scope_decorator!(ScopedResumeTokenStore, ResumeTokenStore);
scope_decorator!(ScopedTriggerStore, TriggerStore);
scope_decorator!(ScopedOperationLedger, OperationLedger);
scope_decorator!(ScopedOperationLedgerAdjudicator, OperationLedgerAdjudicator);
scope_decorator!(ScopedStartAcceptanceStore, StartAcceptanceStore);
scope_decorator!(ScopedExecutionTurnHandoff, ExecutionTurnHandoff);

const DIRECT_SCOPE_PORTS: &[&str] = &[
    "CheckpointStore",
    "ExecutionJournalReader",
    "ExecutionStore",
    "IdempotencyGuard",
    "IdempotencyStore",
    "NodeResultStore",
    "OperationLedger",
    "OperationLedgerAdjudicator",
    "ResourceStore",
    "ResumeTokenStore",
    "StartAcceptanceStore",
    "TriggerStore",
    "WebhookActivationStore",
    "WorkflowStore",
    "WorkflowVersionStore",
];

const EMBEDDED_SCOPE_PORTS: &[&str] = &["ControlQueue", "ExecutionTurnHandoff"];

const INTENTIONALLY_UNSCOPED_PORTS: &[&str] = &[
    "AuditStore",
    "BlobStore",
    "CredentialPersistence",
    "JobDispatchQueue",
    "MembershipStore",
    "OrgStore",
    "PlanFlavorCatalog",
    "PlanFlavorCatalogAdmin",
    "PlanFlavorCatalogWriter",
    "QuotaStore",
    "RefreshClaimStore",
    "ResumeProducer",
    "StartReservationMaintenance",
    "TurnRecovery",
    "UserStore",
    "WorkspaceStore",
];

fn path_ends_with_scope(path: &syn::Path, scope_names: &[String]) -> bool {
    path.segments.last().is_some_and(|segment| {
        scope_names
            .iter()
            .any(|name| name == &segment.ident.to_string())
    })
}

fn type_mentions_scope(ty: &Type, scope_names: &[String]) -> bool {
    match ty {
        Type::Reference(reference) => type_mentions_scope(&reference.elem, scope_names),
        Type::Array(array) => type_mentions_scope(&array.elem, scope_names),
        Type::Group(group) => type_mentions_scope(&group.elem, scope_names),
        Type::Paren(parenthesized) => type_mentions_scope(&parenthesized.elem, scope_names),
        Type::Path(path) => {
            path_ends_with_scope(&path.path, scope_names)
                || path.path.segments.iter().any(|segment| {
                    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                        return false;
                    };
                    arguments.args.iter().any(|argument| match argument {
                        GenericArgument::Type(argument) => {
                            type_mentions_scope(argument, scope_names)
                        },
                        _ => false,
                    })
                })
        },
        Type::Ptr(pointer) => type_mentions_scope(&pointer.elem, scope_names),
        Type::Slice(slice) => type_mentions_scope(&slice.elem, scope_names),
        Type::Tuple(tuple) => tuple
            .elems
            .iter()
            .any(|element| type_mentions_scope(element, scope_names)),
        _ => false,
    }
}

fn type_contains_scope_reference(ty: &Type, scope_names: &[String]) -> bool {
    match ty {
        Type::Reference(reference) => type_mentions_scope(&reference.elem, scope_names),
        Type::Array(array) => type_contains_scope_reference(&array.elem, scope_names),
        Type::Group(group) => type_contains_scope_reference(&group.elem, scope_names),
        Type::Paren(parenthesized) => {
            type_contains_scope_reference(&parenthesized.elem, scope_names)
        },
        Type::Path(path) => path.path.segments.iter().any(|segment| {
            let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                return false;
            };
            arguments.args.iter().any(|argument| match argument {
                GenericArgument::Type(argument) => {
                    type_contains_scope_reference(argument, scope_names)
                },
                _ => false,
            })
        }),
        Type::Ptr(pointer) => type_contains_scope_reference(&pointer.elem, scope_names),
        Type::Slice(slice) => type_contains_scope_reference(&slice.elem, scope_names),
        Type::Tuple(tuple) => tuple
            .elems
            .iter()
            .any(|element| type_contains_scope_reference(element, scope_names)),
        _ => false,
    }
}

fn use_tree_renames_scope(tree: &UseTree) -> bool {
    match tree {
        UseTree::Rename(rename) => rename.ident == "Scope",
        UseTree::Group(group) => group.items.iter().any(use_tree_renames_scope),
        UseTree::Path(path) => use_tree_renames_scope(&path.tree),
        UseTree::Name(_) | UseTree::Glob(_) => false,
    }
}

fn method_contains_scope_reference(method: &syn::TraitItemFn, scope_names: &[String]) -> bool {
    method.sig.inputs.iter().any(|input| match input {
        FnArg::Receiver(_) => false,
        FnArg::Typed(argument) => type_contains_scope_reference(&argument.ty, scope_names),
    })
}

fn collect_port_traits(
    items: &[Item],
    inherited_scope_names: &[String],
    traits: &mut Vec<(String, bool)>,
) {
    let mut scope_names = inherited_scope_names.to_vec();
    loop {
        let previous_len = scope_names.len();
        for item in items {
            if let Item::Type(alias) = item
                && type_mentions_scope(&alias.ty, &scope_names)
            {
                let alias_name = alias.ident.to_string();
                if !scope_names.contains(&alias_name) {
                    scope_names.push(alias_name);
                }
            }
        }
        if scope_names.len() == previous_len {
            break;
        }
    }

    for item in items {
        match item {
            Item::Use(import) => assert!(
                !use_tree_renames_scope(&import.tree),
                "Scope imports in storage ports must retain the Scope name"
            ),
            Item::Macro(_) => panic!(
                "module-level macros are not allowed in storage-port store modules because they can hide public port traits from the tenancy inventory"
            ),
            Item::Mod(module) => {
                // An inline module is inspected here; a file-backed one is
                // reached by the directory walk, which visits every `.rs` file
                // under `store/` including `mod.rs`. Rejecting `mod x;` would
                // now reject the ordinary module layout rather than close a
                // hole, since nothing can hide behind a file the walk opens.
                if let Some((_, nested_items)) = &module.content {
                    collect_port_traits(nested_items, &scope_names, traits);
                }
            },
            Item::Trait(port) if matches!(port.vis, syn::Visibility::Public(_)) => {
                let has_scope_reference = port.items.iter().any(|item| match item {
                    TraitItem::Fn(method) => method_contains_scope_reference(method, &scope_names),
                    _ => false,
                });
                traits.push((port.ident.to_string(), has_scope_reference));
            },
            _ => {},
        }
    }
}

fn declared_port_traits() -> Vec<(String, bool)> {
    let store_root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../storage-port/src/store");
    let mut traits = Vec::new();
    collect_port_traits_in_dir(&store_root, &mut traits);
    traits.sort_unstable();
    traits
}

/// Walk the whole module tree, `mod.rs` included.
///
/// A non-recursive scan that also skipped `mod.rs` left two ways for a port to
/// escape a test whose entire purpose is completeness: declare the trait in
/// `store/mod.rs`, or in `store/<submodule>/`. Either one kept the inventory
/// green while the port shipped without a decorator.
fn collect_port_traits_in_dir(dir: &std::path::Path, traits: &mut Vec<(String, bool)>) {
    let entries = std::fs::read_dir(dir).expect("storage-port store directory exists");
    for entry in entries {
        let path = entry.expect("store entry is readable").path();
        if path.is_dir() {
            collect_port_traits_in_dir(&path, traits);
            continue;
        }
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("store source is readable");
        let syntax = syn::parse_file(&source).expect("storage-port store source parses as Rust");
        collect_port_traits(&syntax.items, &["Scope".to_owned()], traits);
    }
}

/// The source-derived inventory fails when a new port is not classified.
#[test]
fn every_port_has_an_explicit_tenancy_classification() {
    let declared = declared_port_traits();
    let mut classified = DIRECT_SCOPE_PORTS
        .iter()
        .chain(EMBEDDED_SCOPE_PORTS)
        .chain(INTENTIONALLY_UNSCOPED_PORTS)
        .copied()
        .collect::<Vec<_>>();
    classified.sort_unstable();
    assert_eq!(
        declared
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        classified,
        "every public storage port must be classified when it is introduced"
    );
    assert_eq!(
        declared
            .iter()
            .filter_map(|(name, has_direct_scope)| has_direct_scope.then_some(name.as_str()))
            .collect::<Vec<_>>(),
        DIRECT_SCOPE_PORTS,
        "every port with a direct &Scope parameter must have a scope-substituting decorator"
    );

    // Atomic execution unit (§12.2): create/get/lease/commit all `&Scope`.
    assert_scoped::<ScopedExecutionStore, dyn ExecutionStore>();
    assert_scoped::<ScopedCheckpointStore, dyn CheckpointStore>();
    // Workflow + version split: row carries an embedded `Scope` (rebound).
    assert_scoped::<ScopedWorkflowStore, dyn WorkflowStore>();
    assert_scoped::<ScopedWorkflowVersionStore, dyn WorkflowVersionStore>();
    // Per-node result cache: `&Scope`-keyed put/get.
    assert_scoped::<ScopedNodeResultStore, dyn NodeResultStore>();
    // Idempotency dedup: `&Scope` namespaces the key (no replay oracle).
    assert_scoped::<ScopedIdempotencyGuard, dyn IdempotencyGuard>();
    assert_scoped::<ScopedIdempotencyStore, dyn IdempotencyStore>();
    // Control queue: enqueued msg carries a `Scope` (rebound).
    assert_scoped::<ScopedControlQueue, dyn ControlQueue>();
    // Execution journal read path: `&Scope`-keyed.
    assert_scoped::<ScopedExecutionJournalReader, dyn ExecutionJournalReader>();
    // Webhook activation: `&Scope`-keyed.
    assert_scoped::<ScopedWebhookActivationStore, dyn WebhookActivationStore>();
    // Identity zoo, workspace-scoped (the BOLA/IDOR class this guards):
    assert_scoped::<ScopedResourceStore, dyn ResourceStore>();
    assert_scoped::<ScopedTriggerStore, dyn TriggerStore>();
    // Resume-token revocation is scope-keyed (`revoke_on_terminal` takes
    // `&Scope`); consume has no scope parameter by design (hash = only key).
    assert_scoped::<ScopedResumeTokenStore, dyn ResumeTokenStore>();
    assert_scoped::<ScopedOperationLedger, dyn OperationLedger>();
    assert_scoped::<ScopedOperationLedgerAdjudicator, dyn OperationLedgerAdjudicator>();
    assert_scoped::<ScopedStartAcceptanceStore, dyn StartAcceptanceStore>();
    assert_scoped::<ScopedExecutionTurnHandoff, dyn ExecutionTurnHandoff>();
}

/* Decision record for the identity-zoo traits that are **not**
/// `&Scope`-keyed. These authorize on a parent id (org/workspace id) or a
/// global key, resolved at the composition root *before* the call — they
/// have no caller-supplied `&Scope` surface a confused deputy could
/// forge, so a `Scoped*` substitution decorator is not applicable. This
/// test is documentation-as-code: if one of these traits ever grows a
/// `&Scope` parameter, the matching `assert_scoped` line must be added
/// above (and a decorator shipped), and this comment block updated.
///
/// | Trait             | Keying                                   | Why no `Scoped*`                                                                 |
/// |-------------------|------------------------------------------|----------------------------------------------------------------------------------|
/// | `UserStore`       | global user id / email                   | Users are global, not tenant-scoped; lookups are first-writer-wins on email.      |
/// | `OrgStore`        | global org id / slug                     | Org *is* the tenancy root; there is no enclosing scope to substitute.             |
/// | `WorkspaceStore`  | parent `org_id` + workspace id           | Parent-org authorization is resolved at the composition root, not via `&Scope`.   |
/// | `MembershipStore` | (`scope_kind`, `scope_id`, principal)    | The authz domain itself; substituting a scope would corrupt the ACL it defines.  |
/// | `QuotaStore`      | parent `org_id`                          | Org-level CAS counters; org id is the resolved boundary, no `&Scope` surface.     |
/// | `AuditStore`      | parent `org_id` (append-only)            | Append-only org-scoped log; org id resolved at root; nothing to substitute.      |
/// | `BlobStore`       | parent `workspace_id`                    | Workspace-id-keyed at the root; no `&Scope` arg a deputy could forge.             |
///
/// `&Scope`-keyed traits MUST get a decorator (enumerated above);
/// parent-id-keyed traits get this documented decision and no decorator. */
