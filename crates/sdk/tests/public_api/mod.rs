//! Offline source walker behind `public_api_snapshot.rs`.
//!
//! It parses crate sources with `syn` (no rustdoc, no nightly), follows the
//! module graph from each `lib.rs` (`mod` declarations, inline modules and
//! `#[path]`), and resolves `use` chains across the loaded crates. Everything
//! it prints is a module path or re-rendered tokens: no file paths, spans or
//! line numbers, so the output is identical on every platform.
//!
//! Known limits (documented, not enforced):
//! - items produced by a macro invocation are not expanded; a name found only
//!   in a module-level macro call renders as `macro-generated` with the call;
//! - impls are attached by resolving their self type in the impl's module
//!   (falling back to a crate-unique last path segment); blanket impls
//!   (`impl<T: X> Y for T`), impls on references or other wrappers, impls in
//!   other crates and impls inside function bodies are not listed;
//! - `#[cfg(test)]`-gated modules and items (any `cfg` mentioning `test`) are
//!   skipped; feature gates are kept and printed textually.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::ToTokens;

/// Doc marker that tags a public item or method as interim surface.
const INTERIM_MARKER: &str = "**Interim surface";

/// Attributes kept when an item is re-rendered; everything else (docs,
/// lint expectations, `must_use`, `inline`) is not signature surface.
const KEPT_ATTRS: &[&str] = &[
    "async_trait",
    "cfg",
    "default",
    "deprecated",
    "derive",
    "macro_export",
    "non_exhaustive",
    "proc_macro",
    "proc_macro_attribute",
    "proc_macro_derive",
    "repr",
];

// ── Crate model ──────────────────────────────────────────────────────────────

/// Position of an item: crate, module path and index in the module.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ItemId {
    pub(crate) krate: String,
    pub(crate) module: Vec<String>,
    pub(crate) index: usize,
}

impl ItemId {
    fn module_path(&self) -> String {
        join_path(&self.krate, &self.module)
    }
}

/// One item of a module plus its own `cfg` gates.
pub(crate) struct Entry {
    pub(crate) item: syn::Item,
    pub(crate) gates: Vec<String>,
}

/// A module with its items and inherited visibility facts.
pub(crate) struct Module {
    pub(crate) entries: Vec<Entry>,
    /// `cfg` gates of this module and its ancestors.
    pub(crate) gates: Vec<String>,
    /// Reachable from the crate root through `pub` modules only.
    pub(crate) public: bool,
    /// This module or an ancestor is `#[doc(hidden)]`.
    pub(crate) hidden: bool,
}

/// All non-test modules of one crate.
pub(crate) struct CrateModel {
    pub(crate) modules: BTreeMap<Vec<String>, Module>,
    macro_exports: BTreeMap<String, ItemId>,
}

/// The set of crates the walker can resolve paths into.
pub(crate) struct Workspace {
    crates: BTreeMap<String, CrateModel>,
    /// Impls attached to the item their self type resolves to.
    impls: BTreeMap<ItemId, Vec<ItemId>>,
}

/// What a path resolves to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target {
    Item(ItemId),
    Module {
        krate: String,
        path: Vec<String>,
    },
    /// The name only appears inside a module-level macro invocation.
    MacroGenerated {
        invocation: ItemId,
        name: String,
    },
    /// A crate the walker does not load.
    External(String),
    Unresolved(String),
}

impl Workspace {
    /// Loads `(crate_name, src_dir)` pairs relative to `root`.
    pub(crate) fn load(root: &Path, crates: &[(&str, &str)]) -> Self {
        let mut models = BTreeMap::new();
        for (name, src) in crates {
            models.insert((*name).to_owned(), load_crate(name, &root.join(src)));
        }
        let mut workspace = Self {
            crates: models,
            impls: BTreeMap::new(),
        };
        workspace.index_impls();
        workspace
    }

    pub(crate) fn krate(&self, name: &str) -> &CrateModel {
        self.crates
            .get(name)
            .unwrap_or_else(|| panic!("crate {name} is not loaded"))
    }

    pub(crate) fn entry(&self, id: &ItemId) -> &Entry {
        &self.krate(&id.krate).modules[&id.module].entries[id.index]
    }

    fn index_impls(&mut self) {
        let mut by_name: BTreeMap<(String, String), Vec<ItemId>> = BTreeMap::new();
        let mut impls = Vec::new();
        for (krate, model) in &self.crates {
            for (module, contents) in &model.modules {
                for (index, entry) in contents.entries.iter().enumerate() {
                    let id = ItemId {
                        krate: krate.clone(),
                        module: module.clone(),
                        index,
                    };
                    if let syn::Item::Impl(item) = &entry.item {
                        impls.push((id, (*item.self_ty).clone()));
                    } else if let Some(name) = defined_type_name(&entry.item) {
                        by_name.entry((krate.clone(), name)).or_default().push(id);
                    }
                }
            }
        }
        let mut attached: BTreeMap<ItemId, Vec<ItemId>> = BTreeMap::new();
        for (impl_id, self_ty) in impls {
            let syn::Type::Path(type_path) = &self_ty else {
                continue;
            };
            if type_path.qself.is_some() {
                continue;
            }
            let segments: Vec<String> = type_path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            let target = match self.resolve_path(&impl_id.krate, &impl_id.module, &segments, 0) {
                Target::Item(id) => Some(id),
                _ => segments.last().and_then(|last| {
                    match by_name.get(&(impl_id.krate.clone(), last.clone())) {
                        Some(ids) if ids.len() == 1 => Some(ids[0].clone()),
                        _ => None,
                    }
                }),
            };
            if let Some(target) = target {
                attached.entry(target).or_default().push(impl_id);
            }
        }
        self.impls = attached;
    }

    /// Resolves `segments` as written in module `scope` of `krate`.
    pub(crate) fn resolve_path(
        &self,
        krate: &str,
        scope: &[String],
        segments: &[String],
        depth: usize,
    ) -> Target {
        let written = segments.join("::");
        if depth > 32 || segments.is_empty() {
            return Target::Unresolved(written);
        }
        let (mut current_krate, mut current, rest) = match segments[0].as_str() {
            "crate" => (krate.to_owned(), Vec::new(), &segments[1..]),
            "self" => (krate.to_owned(), scope.to_vec(), &segments[1..]),
            "super" => {
                let mut module = scope.to_vec();
                let mut skip = 0;
                while segments.get(skip).is_some_and(|segment| segment == "super") {
                    module.pop();
                    skip += 1;
                }
                (krate.to_owned(), module, &segments[skip..])
            },
            first => {
                if segments.len() == 1 {
                    return self.lookup(krate, scope, first, depth + 1);
                }
                match self.lookup(krate, scope, first, depth + 1) {
                    Target::Module { krate, path } => (krate, path, &segments[1..]),
                    _ if self.crates.contains_key(first) => {
                        (first.to_owned(), Vec::new(), &segments[1..])
                    },
                    _ => return Target::External(written),
                }
            },
        };
        if !self.crates.contains_key(&current_krate) {
            return Target::External(written);
        }
        let Some((last, middle)) = rest.split_last() else {
            return Target::Module {
                krate: current_krate,
                path: current,
            };
        };
        for segment in middle {
            match self.lookup(&current_krate, &current, segment, depth + 1) {
                Target::Module { krate, path } => {
                    current_krate = krate;
                    current = path;
                },
                other @ Target::External(_) => return other,
                _ => return Target::Unresolved(written),
            }
        }
        self.lookup(&current_krate, &current, last, depth + 1)
    }

    /// Resolves the single name `name` in module `module` of `krate`.
    fn lookup(&self, krate: &str, module: &[String], name: &str, depth: usize) -> Target {
        let unresolved = || Target::Unresolved(format!("{}::{name}", join_path(krate, module)));
        if depth > 32 {
            return unresolved();
        }
        let Some(model) = self.crates.get(krate) else {
            return Target::External(format!("{krate}::{name}"));
        };
        let Some(contents) = model.modules.get(module) else {
            return unresolved();
        };
        let id = |index| ItemId {
            krate: krate.to_owned(),
            module: module.to_vec(),
            index,
        };
        // 1. Definitions and child modules.
        for (index, entry) in contents.entries.iter().enumerate() {
            if let syn::Item::Mod(item) = &entry.item
                && item.ident == name
            {
                let mut path = module.to_vec();
                path.push(name.to_owned());
                return Target::Module {
                    krate: krate.to_owned(),
                    path,
                };
            }
            if defined_name(&entry.item).as_deref() == Some(name) {
                return Target::Item(id(index));
            }
        }
        // 2. Explicit imports.
        for entry in &contents.entries {
            let syn::Item::Use(item) = &entry.item else {
                continue;
            };
            for leaf in use_leaves(&item.tree) {
                if leaf.binding.as_deref() == Some(name) {
                    return self.resolve_path(krate, module, &leaf.path, depth + 1);
                }
            }
        }
        // 3. Names that only appear inside a module-level macro invocation.
        for (index, entry) in contents.entries.iter().enumerate() {
            if let syn::Item::Macro(item) = &entry.item
                && item.ident.is_none()
                && mentions_ident(&item.mac.tokens, name)
            {
                return Target::MacroGenerated {
                    invocation: id(index),
                    name: name.to_owned(),
                };
            }
        }
        // 4. Glob imports.
        for entry in &contents.entries {
            let syn::Item::Use(item) = &entry.item else {
                continue;
            };
            for leaf in use_leaves(&item.tree) {
                if leaf.binding.is_some() {
                    continue;
                }
                if let Target::Module { krate, path } =
                    self.resolve_path(krate, module, &leaf.path, depth + 1)
                {
                    let found = self.lookup(&krate, &path, name, depth + 1);
                    if !matches!(found, Target::Unresolved(_)) {
                        return found;
                    }
                }
            }
        }
        // 5. `#[macro_export]` macros live at the crate root.
        if module.is_empty()
            && let Some(id) = model.macro_exports.get(name)
        {
            return Target::Item(id.clone());
        }
        unresolved()
    }
}

fn load_crate(name: &str, src_dir: &Path) -> CrateModel {
    let mut model = CrateModel {
        modules: BTreeMap::new(),
        macro_exports: BTreeMap::new(),
    };
    let lib = src_dir.join("lib.rs");
    let file = parse_file(&lib);
    model.add_module(
        name,
        Vec::new(),
        file.items,
        &ModuleDirs {
            file_dir: src_dir.to_path_buf(),
            child_dir: src_dir.to_path_buf(),
        },
        ModuleFacts {
            gates: Vec::new(),
            public: true,
            hidden: false,
        },
    );
    model
}

fn parse_file(path: &Path) -> syn::File {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

struct ModuleDirs {
    /// Directory of the file the module's items came from (`#[path]` base).
    file_dir: PathBuf,
    /// Directory where the module's `mod child;` files live.
    child_dir: PathBuf,
}

struct ModuleFacts {
    gates: Vec<String>,
    public: bool,
    hidden: bool,
}

impl CrateModel {
    fn add_module(
        &mut self,
        krate: &str,
        path: Vec<String>,
        items: Vec<syn::Item>,
        dirs: &ModuleDirs,
        facts: ModuleFacts,
    ) {
        let mut entries = Vec::new();
        for item in items {
            let attrs = item_attrs(&item);
            if attrs.iter().any(is_test_cfg) {
                continue;
            }
            let gates = cfg_gates(attrs);
            if let syn::Item::Mod(module) = &item {
                let ident = module.ident.to_string();
                let mut child_path = path.clone();
                child_path.push(ident.clone());
                let child_facts = ModuleFacts {
                    gates: facts.gates.iter().chain(&gates).cloned().collect(),
                    public: facts.public && is_pub(&module.vis),
                    hidden: facts.hidden || is_doc_hidden(&module.attrs),
                };
                if let Some((_, inner)) = &module.content {
                    let child_dir = dirs.child_dir.join(&ident);
                    let child_dirs = ModuleDirs {
                        file_dir: dirs.file_dir.clone(),
                        child_dir,
                    };
                    self.add_module(krate, child_path, inner.clone(), &child_dirs, child_facts);
                } else {
                    let file = path_attr(&module.attrs).map_or_else(
                        || {
                            let flat = dirs.child_dir.join(format!("{ident}.rs"));
                            if flat.is_file() {
                                flat
                            } else {
                                dirs.child_dir.join(&ident).join("mod.rs")
                            }
                        },
                        |relative| dirs.file_dir.join(relative),
                    );
                    let file_dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
                    let child_dir = if file.file_name().is_some_and(|name| name == "mod.rs") {
                        file_dir.clone()
                    } else {
                        file_dir.join(file.file_stem().unwrap_or_default())
                    };
                    let parsed = parse_file(&file);
                    self.add_module(
                        krate,
                        child_path,
                        parsed.items,
                        &ModuleDirs {
                            file_dir,
                            child_dir,
                        },
                        child_facts,
                    );
                }
            }
            if let syn::Item::Macro(mac) = &item
                && let Some(ident) = &mac.ident
                && has_attr(&mac.attrs, "macro_export")
            {
                self.macro_exports.insert(
                    ident.to_string(),
                    ItemId {
                        krate: krate.to_owned(),
                        module: path.clone(),
                        index: entries.len(),
                    },
                );
            }
            entries.push(Entry { item, gates });
        }
        self.modules.insert(
            path,
            Module {
                entries,
                gates: facts.gates,
                public: facts.public,
                hidden: facts.hidden,
            },
        );
    }
}

// ── Syntax helpers ───────────────────────────────────────────────────────────

pub(crate) fn join_path(krate: &str, module: &[String]) -> String {
    std::iter::once(krate)
        .chain(module.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("::")
}

pub(crate) fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

pub(crate) fn item_vis(item: &syn::Item) -> Option<&syn::Visibility> {
    Some(match item {
        syn::Item::Const(item) => &item.vis,
        syn::Item::Enum(item) => &item.vis,
        syn::Item::Fn(item) => &item.vis,
        syn::Item::Mod(item) => &item.vis,
        syn::Item::Static(item) => &item.vis,
        syn::Item::Struct(item) => &item.vis,
        syn::Item::Trait(item) => &item.vis,
        syn::Item::TraitAlias(item) => &item.vis,
        syn::Item::Type(item) => &item.vis,
        syn::Item::Union(item) => &item.vis,
        syn::Item::Use(item) => &item.vis,
        _ => return None,
    })
}

/// The name an item defines in its module (proc-macro derives by their
/// derive name, `macro_rules!` by the macro name).
pub(crate) fn defined_name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Fn(item) => {
            Some(proc_macro_name(&item.attrs).unwrap_or_else(|| item.sig.ident.to_string()))
        },
        syn::Item::Macro(item) => item.ident.as_ref().map(ToString::to_string),
        other => defined_type_name(other),
    }
}

fn defined_type_name(item: &syn::Item) -> Option<String> {
    Some(match item {
        syn::Item::Const(item) => item.ident.to_string(),
        syn::Item::Enum(item) => item.ident.to_string(),
        syn::Item::Static(item) => item.ident.to_string(),
        syn::Item::Struct(item) => item.ident.to_string(),
        syn::Item::Trait(item) => item.ident.to_string(),
        syn::Item::TraitAlias(item) => item.ident.to_string(),
        syn::Item::Type(item) => item.ident.to_string(),
        syn::Item::Union(item) => item.ident.to_string(),
        _ => return None,
    })
}

pub(crate) fn item_kind(item: &syn::Item) -> &'static str {
    match item {
        syn::Item::Const(_) => "const",
        syn::Item::Enum(_) => "enum",
        syn::Item::Fn(item) if proc_macro_name(&item.attrs).is_some() => "proc_macro",
        syn::Item::Fn(_) => "fn",
        syn::Item::Macro(_) => "macro_rules",
        syn::Item::Mod(_) => "mod",
        syn::Item::Static(_) => "static",
        syn::Item::Struct(_) => "struct",
        syn::Item::Trait(_) => "trait",
        syn::Item::TraitAlias(_) => "trait_alias",
        syn::Item::Type(_) => "type",
        syn::Item::Union(_) => "union",
        _ => "item",
    }
}

fn proc_macro_name(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("proc_macro_derive") {
            return None;
        }
        let syn::Meta::List(list) = &attr.meta else {
            return None;
        };
        list.tokens.clone().into_iter().find_map(|tree| match tree {
            TokenTree::Ident(ident) => Some(ident.to_string()),
            _ => None,
        })
    })
}

pub(crate) fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn attr_name(attr: &syn::Attribute) -> String {
    attr.path()
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default()
}

fn is_test_cfg(attr: &syn::Attribute) -> bool {
    attr.path().is_ident("cfg") && mentions_ident(&attr.meta.to_token_stream(), "test")
}

fn mentions_ident(tokens: &TokenStream, name: &str) -> bool {
    tokens.clone().into_iter().any(|tree| match tree {
        TokenTree::Ident(ident) => ident == name,
        TokenTree::Group(group) => mentions_ident(&group.stream(), name),
        _ => false,
    })
}

pub(crate) fn cfg_gates(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .map(|attr| render(attr.meta.to_token_stream()))
        .collect()
}

pub(crate) fn is_doc_hidden(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("doc")
            && matches!(&attr.meta, syn::Meta::List(list) if mentions_ident(&list.tokens, "hidden"))
    })
}

fn doc_text(attrs: &[syn::Attribute]) -> String {
    let mut text = String::new();
    for attr in attrs {
        if let syn::Meta::NameValue(name_value) = &attr.meta
            && name_value.path.is_ident("doc")
            && let syn::Expr::Lit(expr) = &name_value.value
            && let syn::Lit::Str(value) = &expr.lit
        {
            text.push_str(&value.value());
            text.push('\n');
        }
    }
    text
}

fn is_interim(attrs: &[syn::Attribute]) -> bool {
    doc_text(attrs).contains(INTERIM_MARKER)
}

fn path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.meta {
        syn::Meta::NameValue(name_value) if name_value.path.is_ident("path") => {
            match &name_value.value {
                syn::Expr::Lit(expr) => match &expr.lit {
                    syn::Lit::Str(value) => Some(value.value()),
                    _ => None,
                },
                _ => None,
            }
        },
        _ => None,
    })
}

fn kept_attrs(attrs: &[syn::Attribute]) -> Vec<syn::Attribute> {
    attrs
        .iter()
        .filter(|attr| KEPT_ATTRS.contains(&attr_name(attr).as_str()))
        .cloned()
        .collect()
}

/// Markers appended to a rendered line.
fn markers(attrs: &[syn::Attribute]) -> String {
    let mut out = String::new();
    if is_interim(attrs) {
        out.push_str(" [interim]");
    }
    if is_doc_hidden(attrs) {
        out.push_str(" [hidden]");
    }
    out
}

/// One imported binding of a `use` tree.
pub(crate) struct UseLeaf {
    /// Path as written, including the original name.
    pub(crate) path: Vec<String>,
    /// Bound name (after `as`); `None` for a glob.
    pub(crate) binding: Option<String>,
}

pub(crate) fn use_leaves(tree: &syn::UseTree) -> Vec<UseLeaf> {
    let mut out = Vec::new();
    collect_use_leaves(tree, &mut Vec::new(), &mut out);
    out
}

fn collect_use_leaves(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<UseLeaf>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_leaves(&path.tree, prefix, out);
            prefix.pop();
        },
        syn::UseTree::Name(name) => {
            let ident = name.ident.to_string();
            if ident == "self" {
                out.push(UseLeaf {
                    path: prefix.clone(),
                    binding: prefix.last().cloned(),
                });
            } else {
                let mut path = prefix.clone();
                path.push(ident.clone());
                out.push(UseLeaf {
                    path,
                    binding: Some(ident),
                });
            }
        },
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            out.push(UseLeaf {
                path,
                binding: Some(rename.rename.to_string()),
            });
        },
        syn::UseTree::Glob(_) => out.push(UseLeaf {
            path: prefix.clone(),
            binding: None,
        }),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_leaves(item, prefix, out);
            }
        },
    }
}

// ── Export map ───────────────────────────────────────────────────────────────

/// One public path of a crate.
pub(crate) struct Export {
    /// Module the export is declared in.
    pub(crate) module: Vec<String>,
    /// Public path, e.g. `nebula_sdk::prelude::Action`.
    pub(crate) public_path: String,
    /// Origin path as written in the `use` (`None` for local items).
    pub(crate) origin: Option<Vec<String>>,
    pub(crate) line: String,
}

/// Every public path of `krate`, skipping modules named `skip_module`.
pub(crate) fn exports(workspace: &Workspace, krate: &str, skip_module: &str) -> Vec<Export> {
    let model = workspace.krate(krate);
    let mut out = Vec::new();
    for (module, contents) in &model.modules {
        if !contents.public || module.iter().any(|segment| segment == skip_module) {
            continue;
        }
        for entry in &contents.entries {
            let item = &entry.item;
            let attrs = item_attrs(item);
            let mut tags: Vec<String> = contents
                .gates
                .iter()
                .chain(&entry.gates)
                .map(|gate| format!("[{gate}]"))
                .collect();
            if contents.hidden || is_doc_hidden(attrs) {
                tags.push("[hidden]".to_owned());
            }
            let exported_macro = matches!(item, syn::Item::Macro(mac)
                if mac.ident.is_some() && has_attr(&mac.attrs, "macro_export"));
            if exported_macro {
                tags.push("[macro_export]".to_owned());
            } else if !item_vis(item).is_some_and(is_pub) {
                continue;
            }
            // `#[macro_export]` macros are exported at the crate root.
            let at = if exported_macro { &[][..] } else { &module[..] };
            let base = join_path(krate, at);
            let suffix = if tags.is_empty() {
                String::new()
            } else {
                format!(" {}", tags.join(" "))
            };
            if let syn::Item::Use(item) = item {
                for leaf in use_leaves(&item.tree) {
                    let written = leaf.path.join("::");
                    let (public_path, origin, kind) = match &leaf.binding {
                        Some(binding) => (format!("{base}::{binding}"), written, "[use]"),
                        None => (format!("{base}::*"), format!("{written}::*"), "[glob]"),
                    };
                    if public_path.rsplit("::").next() == Some(skip_module) {
                        continue;
                    }
                    out.push(Export {
                        module: module.clone(),
                        line: format!("{public_path} => {origin} {kind}{suffix}"),
                        public_path,
                        origin: Some(leaf.path),
                    });
                }
            } else if let Some(name) = defined_name(item).or_else(|| match item {
                syn::Item::Mod(item) => Some(item.ident.to_string()),
                _ => None,
            }) {
                if name == skip_module {
                    continue;
                }
                let public_path = format!("{base}::{name}");
                out.push(Export {
                    module: module.clone(),
                    line: format!("{public_path} => {} [local]{suffix}", item_kind(item)),
                    public_path,
                    origin: None,
                });
            }
        }
    }
    out.sort_by(|left, right| left.line.cmp(&right.line));
    out
}

// ── Signatures ───────────────────────────────────────────────────────────────

/// Renders the definition behind `target` plus its impls in the defining crate.
pub(crate) fn render_target(workspace: &Workspace, target: &Target) -> Vec<String> {
    match target {
        Target::Item(id) => {
            let mut lines = render_item(workspace, id);
            lines.extend(render_impls(workspace, id));
            lines
        },
        Target::MacroGenerated { invocation, .. } => {
            let entry = workspace.entry(invocation);
            let mut item = entry.item.clone();
            if let syn::Item::Macro(mac) = &mut item {
                mac.attrs = kept_attrs(&mac.attrs);
            }
            vec![format!(
                "macro-generated: {}",
                render(item.to_token_stream())
            )]
        },
        Target::Module { krate, path } => vec![format!("mod {}", join_path(krate, path))],
        Target::External(path) => vec![format!("external: {path}")],
        Target::Unresolved(path) => vec![format!("unresolved: {path}")],
    }
}

/// Section heading for a resolved target.
pub(crate) fn target_heading(workspace: &Workspace, target: &Target) -> String {
    match target {
        Target::Item(id) => {
            let entry = workspace.entry(id);
            let module = &workspace.krate(&id.krate).modules[&id.module];
            let name = defined_name(&entry.item).unwrap_or_default();
            let mut heading = format!("{} {}::{name}", item_kind(&entry.item), id.module_path());
            for gate in module.gates.iter().chain(&entry.gates) {
                heading.push_str(&format!(" [{gate}]"));
            }
            heading.push_str(&markers(item_attrs(&entry.item)));
            heading
        },
        Target::MacroGenerated { invocation, name } => {
            format!("macro-generated {}::{name}", invocation.module_path())
        },
        Target::Module { krate, path } => format!("mod {}", join_path(krate, path)),
        Target::External(path) => format!("external {path}"),
        Target::Unresolved(path) => format!("unresolved {path}"),
    }
}

fn attr_lines(attrs: &[syn::Attribute], indent: &str) -> Vec<String> {
    kept_attrs(attrs)
        .iter()
        .filter(|attr| !attr.path().is_ident("cfg"))
        .map(|attr| format!("{indent}{}", render(attr.to_token_stream())))
        .collect()
}

/// `header {}` → `header {`.
fn open_block(rendered: &str) -> String {
    rendered
        .strip_suffix("{}")
        .map_or_else(|| rendered.to_owned(), |head| format!("{head}{{"))
}

fn render_item(workspace: &Workspace, id: &ItemId) -> Vec<String> {
    let entry = workspace.entry(id);
    let attrs = item_attrs(&entry.item);
    let mut lines = attr_lines(attrs, "");
    match &entry.item {
        syn::Item::Struct(item) => {
            let mut item = item.clone();
            item.attrs.clear();
            match &mut item.fields {
                syn::Fields::Named(named) => {
                    let fields: Vec<syn::Field> = named.named.iter().cloned().collect();
                    named.named = syn::punctuated::Punctuated::new();
                    lines.push(open_block(&render(item.to_token_stream())));
                    let mut private = false;
                    for mut field in fields {
                        if !is_pub(&field.vis) {
                            private = true;
                            continue;
                        }
                        let tags = markers(&field.attrs);
                        field.attrs = kept_attrs(&field.attrs);
                        lines.push(format!("    {},{tags}", render(field.to_token_stream())));
                    }
                    if private {
                        lines.push("    /* private fields */".to_owned());
                    }
                    lines.push("}".to_owned());
                },
                syn::Fields::Unnamed(unnamed) => {
                    for field in &mut unnamed.unnamed {
                        field.attrs = kept_attrs(&field.attrs);
                        if !is_pub(&field.vis) {
                            field.ty = syn::parse_quote!(_);
                        }
                    }
                    lines.push(render(item.to_token_stream()));
                },
                syn::Fields::Unit => lines.push(render(item.to_token_stream())),
            }
        },
        syn::Item::Enum(item) => {
            let mut item = item.clone();
            item.attrs.clear();
            let variants: Vec<syn::Variant> = item.variants.iter().cloned().collect();
            item.variants = syn::punctuated::Punctuated::new();
            lines.push(open_block(&render(item.to_token_stream())));
            for mut variant in variants {
                let tags = markers(&variant.attrs);
                variant.attrs = kept_attrs(&variant.attrs);
                if let syn::Fields::Named(named) = &mut variant.fields {
                    for field in &mut named.named {
                        field.attrs = kept_attrs(&field.attrs);
                    }
                }
                if let syn::Fields::Unnamed(unnamed) = &mut variant.fields {
                    for field in &mut unnamed.unnamed {
                        field.attrs = kept_attrs(&field.attrs);
                    }
                }
                lines.push(format!("    {},{tags}", render(variant.to_token_stream())));
            }
            lines.push("}".to_owned());
        },
        syn::Item::Trait(item) => {
            let mut header = item.clone();
            header.attrs.clear();
            header.items.clear();
            lines.push(open_block(&render(header.to_token_stream())));
            for trait_item in &item.items {
                lines.extend(render_trait_item(trait_item));
            }
            lines.push("}".to_owned());
        },
        syn::Item::Fn(item) => {
            let mut tokens = TokenStream::new();
            item.vis.to_tokens(&mut tokens);
            item.sig.to_tokens(&mut tokens);
            lines.push(format!("{};", render(tokens)));
        },
        syn::Item::Macro(item) => {
            let name = item
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            lines.push(format!("macro_rules! {name} {{"));
            for matcher in macro_matchers(&item.mac.tokens) {
                lines.push(format!("    {matcher} => {{ .. }};"));
            }
            lines.push("}".to_owned());
        },
        other => {
            let mut item = other.clone();
            strip_item_attrs(&mut item);
            lines.push(render(item.to_token_stream()));
        },
    }
    lines
}

fn strip_item_attrs(item: &mut syn::Item) {
    match item {
        syn::Item::Const(item) => item.attrs.clear(),
        syn::Item::Static(item) => item.attrs.clear(),
        syn::Item::Type(item) => item.attrs.clear(),
        syn::Item::Union(item) => item.attrs.clear(),
        syn::Item::TraitAlias(item) => item.attrs.clear(),
        _ => {},
    }
}

fn render_trait_item(item: &syn::TraitItem) -> Vec<String> {
    let mut item = item.clone();
    let (attrs, rendered) = match &mut item {
        syn::TraitItem::Fn(function) => {
            let attrs = std::mem::take(&mut function.attrs);
            function.attrs = kept_attrs(&attrs);
            let has_default = function.default.take().is_some();
            if has_default {
                function.semi_token = Some(Default::default());
            }
            let rendered = render(function.to_token_stream());
            let rendered = if has_default {
                rendered
                    .strip_suffix(';')
                    .map_or_else(|| rendered.clone(), |head| format!("{head} {{ .. }}"))
            } else {
                rendered
            };
            (attrs, rendered)
        },
        syn::TraitItem::Type(assoc) => {
            let attrs = std::mem::take(&mut assoc.attrs);
            assoc.attrs = kept_attrs(&attrs);
            (attrs, render(assoc.to_token_stream()))
        },
        syn::TraitItem::Const(assoc) => {
            let attrs = std::mem::take(&mut assoc.attrs);
            assoc.attrs = kept_attrs(&attrs);
            (attrs, render(assoc.to_token_stream()))
        },
        other => (Vec::new(), render(other.to_token_stream())),
    };
    vec![format!("    {rendered}{}", markers(&attrs))]
}

fn render_impls(workspace: &Workspace, id: &ItemId) -> Vec<String> {
    let Some(impl_ids) = workspace.impls.get(id) else {
        return Vec::new();
    };
    let mut inherent = Vec::new();
    let mut trait_impls = BTreeSet::new();
    for impl_id in impl_ids {
        let syn::Item::Impl(item) = &workspace.entry(impl_id).item else {
            continue;
        };
        let mut header = item.clone();
        header.attrs.retain(|attr| attr.path().is_ident("cfg"));
        header.items.clear();
        let head = render(header.to_token_stream());
        let head = head.strip_suffix(" {}").unwrap_or(&head).to_owned();
        if item.trait_.is_some() {
            trait_impls.insert(head);
            continue;
        }
        let members: Vec<String> = item.items.iter().filter_map(render_impl_member).collect();
        if members.is_empty() {
            continue;
        }
        inherent.push(format!("{head} {{"));
        inherent.extend(members);
        inherent.push("}".to_owned());
    }
    inherent.extend(trait_impls);
    inherent
}

fn render_impl_member(member: &syn::ImplItem) -> Option<String> {
    let (attrs, rendered) = match member {
        syn::ImplItem::Fn(function) if is_pub(&function.vis) => {
            let mut tokens = TokenStream::new();
            for attr in kept_attrs(&function.attrs) {
                attr.to_tokens(&mut tokens);
            }
            function.vis.to_tokens(&mut tokens);
            function.sig.to_tokens(&mut tokens);
            (&function.attrs, format!("{};", render(tokens)))
        },
        syn::ImplItem::Const(constant) if is_pub(&constant.vis) => {
            let mut constant = constant.clone();
            let attrs = std::mem::take(&mut constant.attrs);
            constant.attrs = kept_attrs(&attrs);
            let rendered = render(constant.to_token_stream());
            return Some(format!("    {rendered}{}", markers(&attrs)));
        },
        _ => return None,
    };
    Some(format!("    {rendered}{}", markers(attrs)))
}

/// Matchers of a `macro_rules!` body, rendered one per arm.
fn macro_matchers(tokens: &TokenStream) -> Vec<String> {
    let mut out = Vec::new();
    let mut expect_matcher = true;
    for tree in tokens.clone() {
        match tree {
            TokenTree::Group(group) if expect_matcher => {
                out.push(render_with(
                    TokenStream::from(TokenTree::Group(group)),
                    Mode::MacroMatcher,
                ));
                expect_matcher = false;
            },
            TokenTree::Punct(punct) if punct.as_char() == ';' => expect_matcher = true,
            _ => {},
        }
    }
    out
}

// ── Token printer ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Code,
    MacroMatcher,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Op(String),
    Open(Delimiter),
    Close(Delimiter),
}

/// Operators printed as one token when the source joins them.
const JOINED_OPS: &[&str] = &[
    "::", "->", "=>", "..=", "..", "==", "!=", ">=", "<=", "+=", "-=",
];

/// Renders tokens with rustfmt-like spacing on one line.
pub(crate) fn render(tokens: TokenStream) -> String {
    render_with(tokens, Mode::Code)
}

fn render_with(tokens: TokenStream, mode: Mode) -> String {
    let mut flat = Vec::new();
    flatten(tokens, &mut flat);
    let toks = if mode == Mode::Code {
        drop_trailing_commas(flat)
    } else {
        flat
    };
    let mut out = String::new();
    let mut previous: Option<&Tok> = None;
    for tok in &toks {
        if let Some(previous) = previous
            && space_between(previous, tok, mode)
        {
            out.push(' ');
        }
        match tok {
            Tok::Word(word) | Tok::Op(word) => out.push_str(word),
            Tok::Open(delimiter) => out.push_str(open_char(*delimiter)),
            Tok::Close(delimiter) => out.push_str(close_char(*delimiter)),
        }
        previous = Some(tok);
    }
    out.replace('\r', "")
}

const fn open_char(delimiter: Delimiter) -> &'static str {
    match delimiter {
        Delimiter::Parenthesis => "(",
        Delimiter::Brace => "{",
        Delimiter::Bracket => "[",
        Delimiter::None => "",
    }
}

const fn close_char(delimiter: Delimiter) -> &'static str {
    match delimiter {
        Delimiter::Parenthesis => ")",
        Delimiter::Brace => "}",
        Delimiter::Bracket => "]",
        Delimiter::None => "",
    }
}

fn flatten(tokens: TokenStream, out: &mut Vec<Tok>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut index = 0;
    while index < trees.len() {
        match &trees[index] {
            TokenTree::Ident(ident) => out.push(Tok::Word(ident.to_string())),
            TokenTree::Literal(literal) => out.push(Tok::Word(literal.to_string())),
            TokenTree::Group(group) => {
                if group.delimiter() == Delimiter::None {
                    flatten(group.stream(), out);
                } else {
                    out.push(Tok::Open(group.delimiter()));
                    flatten(group.stream(), out);
                    out.push(Tok::Close(group.delimiter()));
                }
            },
            TokenTree::Punct(punct) => {
                // Lifetimes: `'` joined to the following ident.
                if punct.as_char() == '\''
                    && let Some(TokenTree::Ident(ident)) = trees.get(index + 1)
                {
                    out.push(Tok::Word(format!("'{ident}")));
                    index += 2;
                    continue;
                }
                // Longest joined operator starting here.
                let mut joined = String::from(punct.as_char());
                let mut end = index;
                let mut best = (joined.clone(), index);
                while let TokenTree::Punct(current) = &trees[end]
                    && current.spacing() == Spacing::Joint
                    && let Some(TokenTree::Punct(next)) = trees.get(end + 1)
                {
                    joined.push(next.as_char());
                    end += 1;
                    if JOINED_OPS.contains(&joined.as_str()) {
                        best = (joined.clone(), end);
                    }
                }
                out.push(Tok::Op(best.0));
                index = best.1;
            },
        }
        index += 1;
    }
}

/// Drops the trailing commas rustfmt leaves in multi-line lists (parameters,
/// where clauses, braced fields). A parenthesised list keeps its comma when it
/// is the only one, so a one-element tuple `(T,)` stays a tuple.
fn drop_trailing_commas(toks: Vec<Tok>) -> Vec<Tok> {
    let mut drop = BTreeSet::new();
    // (delimiter, top-level comma count) per open group.
    let mut groups: Vec<(Delimiter, usize)> = Vec::new();
    let mut top_level_commas = 0;
    for (index, tok) in toks.iter().enumerate() {
        let previous_is_comma = index > 0 && toks[index - 1] == Tok::Op(",".to_owned());
        match tok {
            Tok::Open(delimiter) => {
                if previous_is_comma && *delimiter == Delimiter::Brace {
                    drop.insert(index - 1);
                }
                groups.push((*delimiter, 0));
            },
            Tok::Close(delimiter) => {
                let commas = groups.pop().map_or(0, |(_, commas)| commas);
                let keep = *delimiter == Delimiter::Parenthesis && commas < 2;
                if previous_is_comma && !keep {
                    drop.insert(index - 1);
                }
            },
            Tok::Op(op) if op == "," => match groups.last_mut() {
                Some((_, commas)) => *commas += 1,
                None => top_level_commas += 1,
            },
            Tok::Op(op) if op == ";" && previous_is_comma => {
                drop.insert(index - 1);
            },
            _ => {},
        }
    }
    if top_level_commas > 0 && toks.last() == Some(&Tok::Op(",".to_owned())) {
        drop.insert(toks.len() - 1);
    }
    toks.into_iter()
        .enumerate()
        .filter(|(index, _)| !drop.contains(index))
        .map(|(_, tok)| tok)
        .collect()
}

fn is_op(tok: &Tok, ops: &[&str]) -> bool {
    matches!(tok, Tok::Op(op) if ops.contains(&op.as_str()))
}

fn space_between(previous: &Tok, next: &Tok, mode: Mode) -> bool {
    use Delimiter::{Brace, Bracket, Parenthesis};
    match (previous, next) {
        (_, Tok::Close(Brace)) => !matches!(previous, Tok::Open(Brace)),
        (_, Tok::Close(_)) => false,
        (Tok::Open(Brace), _) => true,
        (Tok::Open(_), _) => false,
        (_, Tok::Open(Brace)) => !is_op(previous, &["$", "#"]),
        (_, Tok::Open(Parenthesis)) => {
            !(matches!(previous, Tok::Word(_) | Tok::Close(Parenthesis | Bracket))
                || is_op(previous, &[">", "&", "::", "!", "#", "*", "<", "$"]))
        },
        (_, Tok::Open(_)) => !is_op(previous, &["#", "&", "!", "<", "::", "*", "$"]),
        _ => {
            if let Tok::Op(op) = previous {
                match op.as_str() {
                    "*" | "+" | "?" if mode == Mode::MacroMatcher => return true,
                    "::" | "<" | "&" | "#" | "." | ".." | "$" | "?" | "*" => return false,
                    "!" => return true,
                    ">" => return !is_op(next, &["::", ",", ";", ">", ":"]),
                    ":" if mode == Mode::MacroMatcher => return false,
                    "," if mode == Mode::MacroMatcher && is_op(next, &["*", "+", "?"]) => {
                        return false;
                    },
                    "," | ";" | ":" => return true,
                    _ => {},
                }
            }
            if let Tok::Op(op) = next {
                match op.as_str() {
                    "," | ";" | ":" | "." | ".." | "..=" | ">" => return false,
                    "::" => {
                        return !matches!(previous, Tok::Word(_) | Tok::Close(_));
                    },
                    "<" => return !matches!(previous, Tok::Word(_) | Tok::Close(_)),
                    "!" => return !matches!(previous, Tok::Word(_)),
                    "*" | "+" | "?" if mode == Mode::MacroMatcher => {
                        return !matches!(previous, Tok::Close(_));
                    },
                    _ => {},
                }
            }
            if let Tok::Op(op) = previous
                && op == "$"
            {
                return false;
            }
            true
        },
    }
}
