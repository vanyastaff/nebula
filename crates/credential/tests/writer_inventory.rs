//! Sole-management-writer conformance: the write-path inventory behind the
//! claim that [`CredentialController`](nebula_credential::service::CredentialController)
//! is the *sole management writer* of claim state (issue #998).
//!
//! The claim is only true if every *production* call into the claim-store
//! write surface flows through the authority structure the design fixes:
//!
//! - `try_claim` / `mark_sentinel` / `heartbeat` / `release` are the
//!   lease holder's surface, and the lease holder is `RefreshCoordinator`
//!   (with `release` also reached from its `RefreshLease` guard);
//! - `reclaim_stuck` is the background sweep's surface, called only by the
//!   `run_one_sweep` task wired to the coordinator;
//! - `adjudicate` is the operator-command surface, called only by
//!   `CredentialController::reconcile`.
//!
//! This test parses the production sources (mirroring
//! `crates/tenancy/tests/scope_decorator_coverage.rs`) and requires every
//! discovered write call site to match that classification exactly. Write
//! calls are detected in both syntaxes — method calls (`repo.release(..)`)
//! and path-form calls (`RefreshClaimAdjudicator::adjudicate(..)`) — and the
//! assertions are anchored on **types and impls, not file paths**: a move
//! that keeps the module graph intact cannot break the inventory, while a
//! new write site, a widened adapter surface, a second adjudicator holder,
//! or a re-surfaced `default_in_memory_coordinator` fails it.
//!
//! Parse sets:
//! - `crates/credential/src` — derived from the crate's `mod` declarations
//!   (starting at `lib.rs`; file-backed `mod`/`pub mod` and `#[path = ..]`
//!   modules are resolved recursively). `#[cfg(test)]`-declared files and
//!   inline `#[cfg(test)]` subtrees are excluded. A stray uncompiled `.rs`
//!   file is not part of the module graph and is invisible by design.
//! - `crates/storage/src/credential/refresh_claim` — directory walk, kept
//!   deliberately: the adapter directory is held to the stricter
//!   no-module-macro rule (which needs every file in the directory), and a
//!   rename of an adapter file with nothing else updated must not break the
//!   inventory.
//! - `apps/server/src/{compose,credential_runtime,credential_composition}.rs`
//!   — named entries.
//! - `crates/engine/src` — full directory walk. Engine files contribute the
//!   pub-item check (assertion (e)) only; their write-shaped calls (the
//!   execution plane has its own `.release`/`.reclaim_stuck`) are not part of
//!   the claim-store inventory.
//!
//! A file that a walked directory reaches only through a `#[cfg(test)]`-gated
//! `mod` is test code wherever its tests sit. The walk still reads it — the
//! no-module-macro rule is directory-wide and may not skip a file — but it
//! contributes no production surface, the same exclusion an inline
//! `#[cfg(test)]` subtree already gets.
//!
//! Known limitation (documented, not enforced): module-level macros
//! (`identity_state!`, `bitflags!`) legitimately exist in `crates/credential`
//! sources, so macro-generated code is invisible to this parse. The six write
//! sites are hand-written today (a write name followed by call parentheses
//! inside a macro's token stream is still inventoried); the adapter directory
//! is held to the stricter no-module-macro rule the tenancy coverage test
//! applies to storage ports.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

/// The six claim-store write methods and their expected production call-site
/// counts on this tree.
const EXPECTED_STORE_WRITES: [(&str, usize); 5] = [
    ("try_claim", 1),
    ("mark_sentinel", 1),
    ("heartbeat", 1),
    ("release", 2),
    ("reclaim_stuck", 1),
];

/// Method names that are write calls on `RefreshClaimStore` / `RefreshClaimAdjudicator`.
const WRITE_METHODS: [&str; 6] = [
    "try_claim",
    "mark_sentinel",
    "heartbeat",
    "release",
    "reclaim_stuck",
    "adjudicate",
];

/// The claim-store port traits a path-form write call may name (assertion (a)):
/// `RefreshClaimStore::try_claim(..)`, `RefreshClaimAdjudicator::adjudicate(..)`,
/// or `Self::<method>` inside either trait's impl. The provider contract's
/// `Dynamic::release` is a path-form `release` and must not count.
const CLAIM_SURFACE_TYPES: [&str; 2] = ["RefreshClaimStore", "RefreshClaimAdjudicator"];

/// The three claim-repo adapters whose inherent (non-trait) public surface
/// must stay constructors-only.
const ADAPTER_TYPES: [&str; 3] = [
    "InMemoryRefreshClaimRepo",
    "SqliteRefreshClaimRepo",
    "PgRefreshClaimRepo",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ParseSetKind {
    #[default]
    Credential,
    StorageAdapters,
    Server,
    Engine,
}

#[derive(Debug)]
struct ImplContext {
    self_ty: String,
    trait_: Option<String>,
}

#[derive(Debug)]
struct WriteCall {
    method: String,
    enclosing_impl: Option<String>,
    enclosing_trait: Option<String>,
    enclosing_fn: String,
    file: PathBuf,
}

impl WriteCall {
    fn describe(&self) -> String {
        format!(
            "{}::{}.{}(..) at {} (impl {:?}, trait {:?})",
            self.enclosing_fn,
            self.enclosing_impl.as_deref().unwrap_or("<free fn>"),
            self.method,
            self.file.display(),
            self.enclosing_impl,
            self.enclosing_trait,
        )
    }
}

/// A site identified by file + enclosing fn (spans are not available from
/// `proc_macro2` without the span-locations feature, so sites are keyed by
/// context rather than line numbers).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: PathBuf,
    enclosing_fn: String,
}

#[derive(Default)]
struct InventoryVisitor {
    current_file: PathBuf,
    current_set: ParseSetKind,
    /// Set by the parse-set loop: this file is reachable only through a
    /// `#[cfg(test)]`-gated `mod`, so it is test code and holds no production
    /// surface (see [`cfg_test_declared_files`]).
    current_file_is_test_only: bool,
    impl_stack: Vec<ImplContext>,
    fn_stack: Vec<String>,
    write_calls: Vec<WriteCall>,
    clone_outs: Vec<Site>,
    controller_news: Vec<Site>,
    controller_new_clone_keys: Vec<Site>,
    /// Identifiers known to bind a `CredentialRuntime` value: the composition
    /// binding (seeded) plus every `let` alias and typed parameter of it
    /// discovered while visiting (see `visit_local` / `register_runtime_params`).
    runtime_idents: BTreeSet<String>,
    adapter_pub_methods: BTreeMap<String, BTreeSet<String>>,
    gateway_field_idents: Vec<Vec<String>>,
    adapter_module_macros: Vec<(String, PathBuf)>,
    engine_pub_items: Vec<(String, PathBuf)>,
}

fn last_type_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

fn last_path_ident(path: &syn::Path) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

/// Whether an attribute is `#[cfg(..)]` with an ident `test` anywhere in its
/// arguments — covers `cfg(test)`, `cfg(any(test, ..))`, `cfg(all(.., test))`,
/// and nested forms. `cfg(feature = "test-util")` does **not** match: the
/// `test` there is a string literal value, and the gate is a feature gate,
/// not the test profile.
fn is_cfg_test_attr(attr: &syn::Attribute) -> bool {
    attr.path().is_ident("cfg") && meta_mentions_test(&attr.meta)
}

fn meta_mentions_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::NameValue(name_value) => name_value.path.is_ident("test"),
        syn::Meta::List(list) => {
            if list.path.is_ident("test") {
                return true;
            }
            let Ok(metas) = list.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return false;
            };
            metas.iter().any(meta_mentions_test)
        },
    }
}

fn has_cfg_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(is_cfg_test_attr)
}

fn collect_type_idents(ty: &syn::Type, out: &mut Vec<String>) {
    match ty {
        syn::Type::Array(array) => collect_type_idents(&array.elem, out),
        syn::Type::Group(group) => collect_type_idents(&group.elem, out),
        syn::Type::Paren(paren) => collect_type_idents(&paren.elem, out),
        syn::Type::Ptr(pointer) => collect_type_idents(&pointer.elem, out),
        syn::Type::Reference(reference) => collect_type_idents(&reference.elem, out),
        syn::Type::Slice(slice) => collect_type_idents(&slice.elem, out),
        syn::Type::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_type_idents(element, out);
            }
        },
        syn::Type::Path(type_path) => {
            for segment in &type_path.path.segments {
                out.push(segment.ident.to_string());
                if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                    for argument in &arguments.args {
                        match argument {
                            syn::GenericArgument::Type(ty) => collect_type_idents(ty, out),
                            syn::GenericArgument::AssocType(assoc) => {
                                collect_type_idents(&assoc.ty, out);
                            },
                            _ => {},
                        }
                    }
                }
            }
        },
        syn::Type::ImplTrait(impl_trait) => {
            for bound in &impl_trait.bounds {
                collect_bound_idents(bound, out);
            }
        },
        syn::Type::TraitObject(trait_object) => {
            for bound in &trait_object.bounds {
                collect_bound_idents(bound, out);
            }
        },
        _ => {},
    }
}

fn collect_bound_idents(bound: &syn::TypeParamBound, out: &mut Vec<String>) {
    if let syn::TypeParamBound::Trait(trait_bound) = bound {
        for segment in &trait_bound.path.segments {
            out.push(segment.ident.to_string());
        }
    }
}

/// Whether an expression ends in a binding of the `CredentialRuntime`: a path
/// whose last ident is a registered runtime ident (the literal composition
/// binding or a discovered alias), possibly behind parentheses / a reference.
fn ends_in_runtime_binding(expr: &syn::Expr, runtime_idents: &BTreeSet<String>) -> bool {
    match expr {
        syn::Expr::Path(path) => {
            last_path_ident(&path.path).is_some_and(|ident| runtime_idents.contains(&ident))
        },
        syn::Expr::Field(field) => matches!(&field.member, syn::Member::Named(name)
            if runtime_idents.contains(&name.to_string())),
        syn::Expr::Paren(paren) => ends_in_runtime_binding(&paren.expr, runtime_idents),
        syn::Expr::Reference(reference) => ends_in_runtime_binding(&reference.expr, runtime_idents),
        _ => false,
    }
}

/// The method-syntax clone-out of the runtime's adjudicator handle:
/// `credential_runtime.adjudicator.clone()`, possibly through a local binding
/// of the runtime (`runtime.adjudicator.clone()` for a registered alias).
fn is_runtime_adjudicator_method_clone(
    call: &syn::ExprMethodCall,
    runtime_idents: &BTreeSet<String>,
) -> bool {
    if call.method != "clone" {
        return false;
    }
    let syn::Expr::Field(field) = call.receiver.as_ref() else {
        return false;
    };
    matches!(&field.member, syn::Member::Named(name) if name == "adjudicator")
        && ends_in_runtime_binding(&field.base, runtime_idents)
}

/// The UFCS clone-out: `Arc::clone(&credential_runtime.adjudicator)` — the
/// shape through which the `RefreshClaimAdjudicator` handle may leave
/// `CredentialRuntime` in qualified-call form.
fn is_ufcs_adjudicator_clone(call: &syn::ExprCall, runtime_idents: &BTreeSet<String>) -> bool {
    let syn::Expr::Path(func_path) = call.func.as_ref() else {
        return false;
    };
    let segments: Vec<String> = func_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    if segments.as_slice() != ["Arc", "clone"] {
        return false;
    }
    let Some(first_arg) = call.args.first() else {
        return false;
    };
    let syn::Expr::Reference(reference) = first_arg else {
        return false;
    };
    let syn::Expr::Field(field) = reference.expr.as_ref() else {
        return false;
    };
    matches!(&field.member, syn::Member::Named(name) if name == "adjudicator")
        && ends_in_runtime_binding(&field.base, runtime_idents)
}

/// Whether the expression is a clone-out of the runtime's adjudicator handle
/// in either syntax. The two forms feed one count — a clone-out is a clone-out
/// however it is written.
fn is_adjudicator_clone_out(expr: &syn::Expr, runtime_idents: &BTreeSet<String>) -> bool {
    match expr {
        syn::Expr::Call(call) => is_ufcs_adjudicator_clone(call, runtime_idents),
        syn::Expr::MethodCall(call) => is_runtime_adjudicator_method_clone(call, runtime_idents),
        _ => false,
    }
}

/// Whether the call expression is `CredentialController::new(..)`.
fn is_controller_new(call: &syn::ExprCall) -> bool {
    let syn::Expr::Path(func_path) = call.func.as_ref() else {
        return false;
    };
    let segments: Vec<String> = func_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    segments.as_slice() == ["CredentialController", "new"]
}

/// The write method a path-form call names, when that call is a claim-store
/// write (assertion (a)). A path-form call counts when its callee is
/// `RefreshClaimStore::<method>`, `RefreshClaimAdjudicator::<method>`,
/// `Self::<method>`, or a bare single-segment `<method>`; the provider
/// contract's `Dynamic::release` is a path-form `release` and must not count.
fn path_form_write_method(call: &syn::ExprCall) -> Option<String> {
    let syn::Expr::Path(callee) = call.func.as_ref() else {
        return None;
    };
    let segments: Vec<String> = callee
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let method = segments.last()?.clone();
    if !WRITE_METHODS.contains(&method.as_str()) {
        return None;
    }
    let qualifies = if segments.len() == 1 {
        true
    } else {
        let qualifier = &segments[segments.len() - 2];
        qualifier == "Self" || CLAIM_SURFACE_TYPES.contains(&qualifier.as_str())
    };
    qualifies.then_some(method)
}

/// Collects every path identifier of an expression — used to register `let`
/// aliases of the runtime (`let runtime = &credential_runtime;` mentions the
/// seeded ident, so `runtime` joins the set).
struct ExprPathCollector(BTreeSet<String>);

impl<'ast> Visit<'ast> for ExprPathCollector {
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if let Some(ident) = last_path_ident(&path.path) {
            self.0.insert(ident);
        }
        syn::visit::visit_expr_path(self, path);
    }
}

/// Scan a macro invocation's token stream for write-method names immediately
/// followed by a parenthesized group (`name(`) — the shape of a call
/// expression the macro hid from `syn`'s expression parser (`tokio::select!`
/// branches, macro arguments). Name-anchored exactly like the parsed method
/// calls: a write name followed by call parens is a write call site and must
/// satisfy the same classification. Code *generated* by a macro definition is
/// not visible to any part of this inventory (documented limitation).
fn scan_macro_tokens(
    tokens: proc_macro2::TokenStream,
    enclosing_impl: Option<String>,
    enclosing_trait: Option<String>,
    enclosing_fn: String,
    file: PathBuf,
    out: &mut Vec<WriteCall>,
) {
    let mut iter = tokens.into_iter().peekable();
    while let Some(token) = iter.next() {
        match token {
            proc_macro2::TokenTree::Group(group) => scan_macro_tokens(
                group.stream(),
                enclosing_impl.clone(),
                enclosing_trait.clone(),
                enclosing_fn.clone(),
                file.clone(),
                out,
            ),
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                if WRITE_METHODS.contains(&name.as_str())
                    && let Some(proc_macro2::TokenTree::Group(next)) = iter.peek()
                    && next.delimiter() == proc_macro2::Delimiter::Parenthesis
                {
                    out.push(WriteCall {
                        method: name,
                        enclosing_impl: enclosing_impl.clone(),
                        enclosing_trait: enclosing_trait.clone(),
                        enclosing_fn: enclosing_fn.clone(),
                        file: file.clone(),
                    });
                }
            },
            _ => {},
        }
    }
}

fn item_vis(item: &syn::Item) -> Option<&syn::Visibility> {
    Some(match item {
        syn::Item::Const(item) => &item.vis,
        syn::Item::Enum(item) => &item.vis,
        syn::Item::Fn(item) => &item.vis,
        syn::Item::Mod(item) => &item.vis,
        syn::Item::Static(item) => &item.vis,
        syn::Item::Struct(item) => &item.vis,
        syn::Item::Trait(item) => &item.vis,
        syn::Item::Type(item) => &item.vis,
        syn::Item::Use(item) => &item.vis,
        _ => return None,
    })
}

fn use_tree_last_ident(tree: &syn::UseTree) -> Option<String> {
    match tree {
        syn::UseTree::Path(path) => use_tree_last_ident(&path.tree),
        syn::UseTree::Name(name) => Some(name.ident.to_string()),
        syn::UseTree::Rename(rename) => Some(rename.rename.to_string()),
        syn::UseTree::Group(_) | syn::UseTree::Glob(_) => None,
    }
}

fn item_name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Const(item) => Some(item.ident.to_string()),
        syn::Item::Enum(item) => Some(item.ident.to_string()),
        syn::Item::Fn(item) => Some(item.sig.ident.to_string()),
        syn::Item::Mod(item) => Some(item.ident.to_string()),
        syn::Item::Static(item) => Some(item.ident.to_string()),
        syn::Item::Struct(item) => Some(item.ident.to_string()),
        syn::Item::Trait(item) => Some(item.ident.to_string()),
        syn::Item::Type(item) => Some(item.ident.to_string()),
        syn::Item::Use(item) => use_tree_last_ident(&item.tree),
        _ => None,
    }
}

impl InventoryVisitor {
    /// The (impl self type, trait, enclosing fn) triple for the visitor's
    /// current position — the classification key for write call sites.
    fn write_context(&self) -> (Option<String>, Option<String>, String) {
        let (enclosing_impl, enclosing_trait) = self
            .impl_stack
            .last()
            .map(|context| (Some(context.self_ty.clone()), context.trait_.clone()))
            .unwrap_or((None, None));
        let enclosing_fn = self
            .fn_stack
            .last()
            .cloned()
            .unwrap_or_else(|| "<top-level>".to_owned());
        (enclosing_impl, enclosing_trait, enclosing_fn)
    }

    /// The (file, enclosing fn) site of the visitor's current position.
    fn current_site(&self) -> Site {
        Site {
            file: self.current_file.clone(),
            enclosing_fn: self
                .fn_stack
                .last()
                .cloned()
                .unwrap_or_else(|| "<top-level>".to_owned()),
        }
    }

    /// Whether the current parse set contributes claim-store write-surface
    /// evidence. Engine files are parsed for the pub-item check (assertion (e))
    /// only: the engine's execution plane has write-shaped method calls of its
    /// own (`.release` on the execution backend, `.reclaim_stuck` on the
    /// control queue) that are not claim-store writes. Test files contribute
    /// nothing: a write in a test is a test writing through the port, not a
    /// production call site the inventory is counting.
    fn inventories_write_surface(&self) -> bool {
        !matches!(self.current_set, ParseSetKind::Engine) && !self.current_file_is_test_only
    }

    /// Record a write call site with the classification context both syntaxes
    /// (method call and path form) share — one count per site, one
    /// classification for all of them.
    fn record_write_call(&mut self, method: String) {
        let (enclosing_impl, enclosing_trait, enclosing_fn) = self.write_context();
        self.write_calls.push(WriteCall {
            method,
            enclosing_impl,
            enclosing_trait,
            enclosing_fn,
            file: self.current_file.clone(),
        });
    }

    /// Register typed parameters whose type mentions `CredentialRuntime` so
    /// the clone detector sees the runtime through parameter bindings.
    fn register_runtime_params(
        &mut self,
        inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
    ) {
        for input in inputs {
            let syn::FnArg::Typed(typed) = input else {
                continue;
            };
            let mut idents = Vec::new();
            collect_type_idents(&typed.ty, &mut idents);
            if !idents.iter().any(|ident| ident == "CredentialRuntime") {
                continue;
            }
            if let syn::Pat::Ident(pattern) = typed.pat.as_ref() {
                self.runtime_idents.insert(pattern.ident.to_string());
            }
        }
    }
}

impl<'ast> Visit<'ast> for InventoryVisitor {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        // Inline `#[cfg(test)]` subtrees (test modules, gated fns/impls) are
        // test doubles, not production writer surface.
        if let Some(vis) = item_vis(item)
            && matches!(vis, syn::Visibility::Public(_))
        {
            // No early return: a pub item is never test-gated, but record
            // before the cfg check so `pub` + `cfg(test)` cannot silently
            // combine on engine surface. The cfg check below still skips it.
            if let Some(name) = item_name(item)
                && matches!(self.current_set, ParseSetKind::Engine)
            {
                self.engine_pub_items
                    .push((name, self.current_file.clone()));
            }
        }
        let attrs: &[syn::Attribute] = match item {
            syn::Item::Fn(item) => &item.attrs,
            syn::Item::Struct(item) => &item.attrs,
            syn::Item::Enum(item) => &item.attrs,
            syn::Item::Trait(item) => &item.attrs,
            syn::Item::Mod(item) => &item.attrs,
            syn::Item::Impl(item) => &item.attrs,
            syn::Item::Const(item) => &item.attrs,
            syn::Item::Static(item) => &item.attrs,
            syn::Item::Type(item) => &item.attrs,
            syn::Item::Use(item) => &item.attrs,
            syn::Item::Macro(item) => &item.attrs,
            _ => &[],
        };
        if has_cfg_test_attr(attrs) {
            return;
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let self_ty = last_type_ident(&item.self_ty).unwrap_or_else(|| "<unnamed>".to_owned());
        let trait_ = item
            .trait_
            .as_ref()
            .map(|(path, _)| last_path_ident(path).unwrap_or_else(|| "<unnamed>".to_owned()));
        // Assertion (c) data: the adapters' inherent (non-trait) impls may
        // only expose constructors. Trait methods belong to the port surface
        // and are not inventoried here. An impl written in a test file is the
        // test's own helper, not the adapter's public surface.
        if trait_.is_none()
            && !self.current_file_is_test_only
            && ADAPTER_TYPES.contains(&self_ty.as_str())
        {
            for impl_item in &item.items {
                if let syn::ImplItem::Fn(method) = impl_item
                    && matches!(method.vis, syn::Visibility::Public(_))
                {
                    self.adapter_pub_methods
                        .entry(self_ty.clone())
                        .or_default()
                        .insert(method.sig.ident.to_string());
                }
            }
        }
        self.impl_stack.push(ImplContext { self_ty, trait_ });
        syn::visit::visit_item_impl(self, item);
        self.impl_stack.pop();
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.register_runtime_params(&item.sig.inputs);
        self.fn_stack.push(item.sig.ident.to_string());
        syn::visit::visit_item_fn(self, item);
        self.fn_stack.pop();
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        // Register `let` bindings of the runtime — by declared type (a typed
        // binding is a `Pat::Type` wrapper in syn 3), or by an initializer
        // mentioning an already-known runtime ident — so the clone detector
        // sees aliases: `let runtime = &credential_runtime;` followed by
        // `runtime.adjudicator.clone()`.
        let init_mentions_runtime = local.init.as_ref().is_some_and(|init| {
            let mut collector = ExprPathCollector(BTreeSet::new());
            collector.visit_expr(init.expr.as_ref());
            collector
                .0
                .iter()
                .any(|ident| self.runtime_idents.contains(ident))
        });
        let type_mentions_runtime = match &local.pat {
            syn::Pat::Type(pat_type) => {
                let mut idents = Vec::new();
                collect_type_idents(&pat_type.ty, &mut idents);
                idents.iter().any(|ident| ident == "CredentialRuntime")
            },
            _ => false,
        };
        if init_mentions_runtime || type_mentions_runtime {
            let bound_ident = match &local.pat {
                syn::Pat::Ident(pattern) => Some(pattern.ident.to_string()),
                syn::Pat::Type(pat_type) => match pat_type.pat.as_ref() {
                    syn::Pat::Ident(pattern) => Some(pattern.ident.to_string()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(ident) = bound_ident {
                self.runtime_idents.insert(ident);
            }
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        // Assertion (d) data: `ServerCredentialGateway` holds the controller,
        // never a claim-store handle.
        if item.ident == "ServerCredentialGateway" {
            let mut field_idents = Vec::new();
            for field in &item.fields {
                collect_type_idents(&field.ty, &mut field_idents);
            }
            self.gateway_field_idents.push(field_idents);
        }
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        // The adapter directory (storage port implementations) follows the
        // tenancy coverage rule: module-level macros may hide surface from
        // the inventory and are rejected there. `crates/credential` sources
        // legitimately use `identity_state!`/`bitflags!` and are not held to
        // it — see the module doc's "known limitation".
        if matches!(self.current_set, ParseSetKind::StorageAdapters) {
            let name = item
                .ident
                .clone()
                .map(|ident| ident.to_string())
                .unwrap_or_else(|| "<unnamed>".to_owned());
            self.adapter_module_macros
                .push((name, self.current_file.clone()));
        }
        // Write calls written literally inside the macro body/arguments stay
        // in the inventory (a macro cannot launder them out of the count).
        // Engine files contribute the pub-item check only — their macro tokens
        // are not claim-store write surface.
        if self.inventories_write_surface() {
            let (enclosing_impl, enclosing_trait, enclosing_fn) = self.write_context();
            scan_macro_tokens(
                item.mac.tokens.clone(),
                enclosing_impl,
                enclosing_trait,
                enclosing_fn,
                self.current_file.clone(),
                &mut self.write_calls,
            );
        }
        syn::visit::visit_item_macro(self, item);
    }

    fn visit_expr_macro(&mut self, expr: &'ast syn::ExprMacro) {
        if self.inventories_write_surface() {
            let (enclosing_impl, enclosing_trait, enclosing_fn) = self.write_context();
            scan_macro_tokens(
                expr.mac.tokens.clone(),
                enclosing_impl,
                enclosing_trait,
                enclosing_fn,
                self.current_file.clone(),
                &mut self.write_calls,
            );
        }
        syn::visit::visit_expr_macro(self, expr);
    }

    fn visit_stmt_macro(&mut self, stmt: &'ast syn::StmtMacro) {
        // A bare macro invocation in statement position (e.g.
        // `tokio::select! { .. };`) parses as `Stmt::Macro`, not an
        // expression — its tokens would otherwise escape the inventory.
        if self.inventories_write_surface() {
            let (enclosing_impl, enclosing_trait, enclosing_fn) = self.write_context();
            scan_macro_tokens(
                stmt.mac.tokens.clone(),
                enclosing_impl,
                enclosing_trait,
                enclosing_fn,
                self.current_file.clone(),
                &mut self.write_calls,
            );
        }
        syn::visit::visit_stmt_macro(self, stmt);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        match item {
            syn::ImplItem::Fn(method) => {
                if has_cfg_test_attr(&method.attrs) {
                    return;
                }
                self.register_runtime_params(&method.sig.inputs);
                self.fn_stack.push(method.sig.ident.to_string());
                syn::visit::visit_impl_item_fn(self, method);
                self.fn_stack.pop();
            },
            _ => syn::visit::visit_impl_item(self, item),
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        match item {
            syn::TraitItem::Fn(method) => {
                if has_cfg_test_attr(&method.attrs) {
                    return;
                }
                self.register_runtime_params(&method.sig.inputs);
                self.fn_stack.push(method.sig.ident.to_string());
                syn::visit::visit_trait_item_fn(self, method);
                self.fn_stack.pop();
            },
            _ => syn::visit::visit_trait_item(self, item),
        }
    }

    fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
        if self.inventories_write_surface() {
            let method = expr.method.to_string();
            if WRITE_METHODS.contains(&method.as_str()) {
                self.record_write_call(method);
            } else if method == "clone"
                && is_runtime_adjudicator_method_clone(expr, &self.runtime_idents)
            {
                // The method-syntax clone-out of the adjudicator (assertion (d),
                // Fix 1): `.adjudicator.clone()` shares the count with the UFCS
                // `Arc::clone(&..)` arm.
                self.clone_outs.push(self.current_site());
            }
        }
        syn::visit::visit_expr_method_call(self, expr);
    }

    fn visit_expr_call(&mut self, expr: &'ast syn::ExprCall) {
        if self.inventories_write_surface() {
            if is_ufcs_adjudicator_clone(expr, &self.runtime_idents) {
                self.clone_outs.push(self.current_site());
            }
            // Assertion (a) path-form arm (Fix 2): a UFCS write call counts
            // exactly like a method call, through the shared classification.
            if let Some(method) = path_form_write_method(expr) {
                self.record_write_call(method);
            }
            if is_controller_new(expr) {
                let site = self.current_site();
                self.controller_news.push(site.clone());
                for argument in &expr.args {
                    if is_adjudicator_clone_out(argument, &self.runtime_idents) {
                        self.controller_new_clone_keys.push(site.clone());
                    }
                }
            }
        }
        syn::visit::visit_expr_call(self, expr);
    }
}

/// Collect every `.rs` file under `dir`, recursively (`mod.rs` included).
/// Used for the adapter directory (held to the no-module-macro rule, which
/// must see every file in it, and to rename-without-update robustness) and
/// for the engine surface walk (assertion (e) checks every file under
/// `crates/engine/src`). Test files are in the result too: whether they carry
/// production surface is decided per file by [`cfg_test_declared_files`], not
/// by leaving them out of the walk.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
            continue;
        }
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs") {
            out.push(path);
        }
    }
}

/// The file a file-backed `mod` declaration names, honoring `#[path = "..."]`.
///
/// `#[path = ".."]` is a name-value attribute, so the string is read from
/// `Meta::NameValue`: `Attribute::parse_args` parses only list-style
/// arguments and would fail here, leaving the name-based fallback to resolve a
/// `#[path]`-renamed file to whatever `<name>.rs` happens to exist.
fn mod_target_file(parent_dir: &Path, module: &syn::ItemMod) -> PathBuf {
    for attr in &module.attrs {
        if attr.path().is_ident("path")
            && let syn::Meta::NameValue(name_value) = &attr.meta
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) = &name_value.value
        {
            return parent_dir.join(path.value());
        }
    }
    let default_file = parent_dir.join(format!("{}.rs", module.ident));
    if default_file.exists() {
        return default_file;
    }
    parent_dir.join(module.ident.to_string()).join("mod.rs")
}

/// The files that a `#[cfg(test)]`-gated file-backed `mod` declaration in
/// `files` names.
///
/// A directory walk cannot tell a test file from a production one — the gate
/// sits on the declaration, in a sibling file — so the walk's results are
/// filtered through this set. It is derived from declarations rather than
/// from file names on purpose: a renamed test file that no declaration names
/// anymore drops out of the set and is inventoried as production again, which
/// fails loudly instead of quietly shrinking what the inventory sees.
fn cfg_test_declared_files<'a>(files: impl IntoIterator<Item = &'a Path>) -> BTreeSet<PathBuf> {
    let mut test_only = BTreeSet::new();
    for file in files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        let Ok(syntax) = syn::parse_file(&source) else {
            continue;
        };
        let parent_dir = file.parent().unwrap_or_else(|| Path::new("."));
        for item in &syntax.items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            if module.content.is_some() || !has_cfg_test_attr(&module.attrs) {
                continue;
            }
            test_only.insert(mod_target_file(parent_dir, module));
        }
    }
    test_only
}

/// The production file set of the credential crate, derived from the crate's
/// `mod` declarations instead of a directory walk. Starts at `lib.rs` and
/// resolves every file-backed `mod`/`pub mod` (honoring `#[path = ".."]` and
/// the `foo.rs`/`foo/mod.rs` convention) recursively; inline modules belong to
/// the declaring file, and `#[cfg(test)]`-declared modules are not production
/// surface. A stray uncompiled `.rs` file is not part of the module graph and
/// is invisible by design (a fail-loud walk in the opposite direction).
fn collect_declared_credential_files(crate_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![crate_root.join("lib.rs")];
    let mut seen = BTreeSet::new();
    while let Some(file) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        let source = std::fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!(
                "declared credential module is readable: {}: {error}",
                file.display()
            )
        });
        let syntax = syn::parse_file(&source).unwrap_or_else(|error| {
            panic!(
                "declared credential module parses as Rust: {}: {error}",
                file.display()
            )
        });
        files.push(file.clone());
        let parent_dir = file.parent().unwrap_or(crate_root);
        for item in &syntax.items {
            let syn::Item::Mod(module) = item else {
                continue;
            };
            if module.content.is_some() || has_cfg_test_attr(&module.attrs) {
                continue;
            }
            pending.push(mod_target_file(parent_dir, module));
        }
    }
    files
}

fn describe_write_calls<'a>(calls: impl IntoIterator<Item = &'a WriteCall>) -> String {
    calls
        .into_iter()
        .map(WriteCall::describe)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The source-derived inventory of the claim-store write surface.
///
/// Fails when any production write call site, adapter public method, or
/// handle flow step does not match the classification the module doc fixes.
#[test]
fn sole_management_writer_inventory() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let credential_root = manifest_dir.join("src");
    let adapter_root = manifest_dir.join("../storage/src/credential/refresh_claim");
    let server_root = manifest_dir.join("../../apps/server/src");
    let engine_root = manifest_dir.join("../engine/src");

    let mut files: Vec<(ParseSetKind, PathBuf)> = Vec::new();

    for file in collect_declared_credential_files(&credential_root) {
        files.push((ParseSetKind::Credential, file));
    }

    let mut adapter_files = Vec::new();
    collect_rs_files(&adapter_root, &mut adapter_files);
    for file in adapter_files {
        files.push((ParseSetKind::StorageAdapters, file));
    }

    for name in [
        "compose.rs",
        "credential_runtime.rs",
        "credential_composition.rs",
    ] {
        files.push((ParseSetKind::Server, server_root.join(name)));
    }

    let mut engine_files = Vec::new();
    collect_rs_files(&engine_root, &mut engine_files);
    for file in engine_files {
        files.push((ParseSetKind::Engine, file));
    }

    // Files a walked directory reaches only through a `#[cfg(test)]`-gated
    // `mod`: the walk reads them (the no-module-macro rule is directory-wide)
    // but they carry no production surface.
    let test_only = cfg_test_declared_files(files.iter().map(|(_, file)| file.as_path()));

    let mut visitor = InventoryVisitor::default();
    // The composition binding the design fixes as the adjudicator's owner;
    // `let` aliases and typed parameters of the runtime join the set as the
    // walk discovers them.
    visitor
        .runtime_idents
        .insert("credential_runtime".to_owned());
    for (set, file) in files {
        let source = std::fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!("inventory source is readable: {}: {error}", file.display())
        });
        let syntax = syn::parse_file(&source).unwrap_or_else(|error| {
            panic!(
                "inventory source parses as Rust: {}: {error}",
                file.display()
            )
        });
        visitor.current_file_is_test_only = test_only.contains(&file);
        visitor.current_file = file;
        visitor.current_set = set;
        visitor.visit_file(&syntax);
    }

    // (a) `adjudicate` is called exactly once in production, and that call is
    // `CredentialController::reconcile`'s — the operator command's only
    // adjudication path. Both syntaxes are inventoried: the method call
    // (`self.adjudicator.adjudicate(..)`) and the path form
    // (`RefreshClaimAdjudicator::adjudicate(..)`).
    let adjudicate_calls: Vec<&WriteCall> = visitor
        .write_calls
        .iter()
        .filter(|call| call.method == "adjudicate")
        .collect();
    assert_eq!(
        adjudicate_calls.len(),
        1,
        "adjudicate must have exactly one production call site, found:\n{}",
        describe_write_calls(adjudicate_calls.iter().copied())
    );
    let adjudicate = &adjudicate_calls[0];
    assert_eq!(
        adjudicate.enclosing_impl.as_deref(),
        Some("CredentialController"),
        "the sole adjudicate call must live in impl CredentialController, found {}",
        adjudicate.describe()
    );
    assert_eq!(
        adjudicate.enclosing_fn,
        "reconcile",
        "the sole adjudicate call must be CredentialController::reconcile, found {}",
        adjudicate.describe()
    );
    assert_eq!(
        adjudicate.enclosing_trait,
        None,
        "the sole adjudicate call must be an inherent method body, found {}",
        adjudicate.describe()
    );

    // (b) store-write call sites ⊆ RefreshCoordinator / the reclaim task, one
    // authority per method, in the exact counts the design fixes.
    for (method, expected) in EXPECTED_STORE_WRITES {
        let calls: Vec<&WriteCall> = visitor
            .write_calls
            .iter()
            .filter(|call| call.method == method)
            .collect();
        assert_eq!(
            calls.len(),
            expected,
            "{method} must have exactly {expected} production call site(s), found {}:\n{}",
            calls.len(),
            describe_write_calls(calls.iter().copied())
        );
        for call in &calls {
            match call.method.as_str() {
                // The lease holder's surface.
                "try_claim" | "mark_sentinel" | "heartbeat" => {
                    assert_eq!(
                        call.enclosing_impl.as_deref(),
                        Some("RefreshCoordinator"),
                        "{} must be called only from impl RefreshCoordinator, found {}",
                        call.method,
                        call.describe()
                    );
                },
                // Released from the lease guard the coordinator owns.
                "release" => {
                    assert!(
                        matches!(
                            call.enclosing_impl.as_deref(),
                            Some("RefreshCoordinator" | "RefreshLease")
                        ),
                        "release must be called only from RefreshCoordinator/RefreshLease, found {}",
                        call.describe()
                    );
                },
                // The background sweep task wired to the coordinator.
                "reclaim_stuck" => {
                    assert_eq!(
                        call.enclosing_impl,
                        None,
                        "reclaim_stuck must be called from a free function (the sweep task), found {}",
                        call.describe()
                    );
                    assert_eq!(
                        call.enclosing_fn,
                        "run_one_sweep",
                        "reclaim_stuck must be called only from run_one_sweep, found {}",
                        call.describe()
                    );
                },
                _ => unreachable!("write method inventory is exhaustive"),
            }
        }
    }

    // (c) the three adapters expose constructors only — no pub inherent write
    // method beyond the port traits.
    let expected_adapter_methods: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::from([
        (
            "InMemoryRefreshClaimRepo",
            BTreeSet::from(["new", "with_clock"]),
        ),
        ("SqliteRefreshClaimRepo", BTreeSet::from(["new"])),
        ("PgRefreshClaimRepo", BTreeSet::from(["new"])),
    ]);
    for adapter in ADAPTER_TYPES {
        let discovered: BTreeSet<String> = visitor
            .adapter_pub_methods
            .get(adapter)
            .cloned()
            .unwrap_or_default();
        let expected: BTreeSet<String> = expected_adapter_methods[adapter]
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        assert_eq!(
            discovered, expected,
            "{adapter} inherent public methods must stay constructors-only"
        );
    }

    // The adapter directory follows the tenancy coverage rule: module-level
    // macros may hide surface from a source inventory and are rejected.
    assert!(
        visitor.adapter_module_macros.is_empty(),
        "module-level macros are not allowed in the claim-repo adapter directory \
         (they can hide write surface from the inventory), found: {:#?}",
        visitor.adapter_module_macros
    );

    // (d) the runtime adjudicator clone-out feeds only
    // `CredentialController::new`; the gateway holds the controller, never a
    // claim-store handle. The clone-out is inventoried in both syntaxes —
    // `Arc::clone(&credential_runtime.adjudicator)` and
    // `credential_runtime.adjudicator.clone()` — sharing one count.
    assert_eq!(
        visitor.clone_outs.len(),
        1,
        "exactly one adjudicator clone-out (`Arc::clone(&credential_runtime.adjudicator)` \
         or `credential_runtime.adjudicator.clone()`) may exist in production, found {:#?}",
        visitor.clone_outs
    );
    assert_eq!(
        visitor.controller_news.len(),
        1,
        "exactly one production `CredentialController::new(..)` is expected, found {:#?}",
        visitor.controller_news
    );
    assert_eq!(
        visitor.controller_new_clone_keys.len(),
        1,
        "the sole adjudicator clone-out must be an argument of `CredentialController::new`, found {:#?}",
        visitor.controller_new_clone_keys
    );
    for clone_out in &visitor.clone_outs {
        assert!(
            visitor.controller_new_clone_keys.contains(clone_out),
            "adjudicator clone-out at {clone_out:?} feeds a non-controller site"
        );
    }
    assert_eq!(
        visitor.gateway_field_idents.len(),
        1,
        "ServerCredentialGateway must be defined exactly once, found {:#?}",
        visitor.gateway_field_idents
    );
    let gateway_fields = &visitor.gateway_field_idents[0];
    assert!(
        gateway_fields
            .iter()
            .any(|ident| ident == "CredentialController"),
        "ServerCredentialGateway must hold the CredentialController, found fields {gateway_fields:?}"
    );
    for handle_ident in ["RefreshClaimAdjudicator", "RefreshClaimStore"] {
        assert!(
            !gateway_fields.iter().any(|ident| ident == handle_ident),
            "ServerCredentialGateway must never hold {handle_ident}, found fields {gateway_fields:?}"
        );
    }

    // (e) `default_in_memory_coordinator` has no pub item anywhere under
    // `crates/engine/src` (the whole directory is walked, not just `lib.rs`).
    // Pinned by the removal in this change; a re-introduction fails here
    // before it can become a direct writer path again.
    let re_surfaced: Vec<&(String, PathBuf)> = visitor
        .engine_pub_items
        .iter()
        .filter(|(name, _)| name == "default_in_memory_coordinator")
        .collect();
    assert!(
        re_surfaced.is_empty(),
        "default_in_memory_coordinator must not exist as a pub item in nebula-engine, found {re_surfaced:?}"
    );
    assert!(
        visitor
            .engine_pub_items
            .iter()
            .any(|(name, _)| name == "EngineCredentialAccessor"),
        "engine surface parse must see the EngineCredentialAccessor re-export (non-vacuity anchor), found {:?}",
        visitor.engine_pub_items
    );
}
