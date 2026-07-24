//! Downstream-safe paths for code emitted by Nebula procedural macros.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Group, Ident, TokenStream, TokenTree};
use quote::quote;

const NEBULA_CRATES: &[(&str, &str, &str)] = &[
    ("nebula_action", "nebula-action", "action"),
    ("nebula_core", "nebula-core", "core"),
    ("nebula_credential", "nebula-credential", "credential"),
    ("nebula_plugin", "nebula-plugin", "plugin"),
    ("nebula_resource", "nebula-resource", "resource"),
    ("nebula_schema", "nebula-schema", "schema"),
    ("nebula_validator", "nebula-validator", "validator"),
    ("nebula_workflow", "nebula-workflow", "workflow"),
];

/// Rewrite absolute `::nebula_*` paths in generated code to paths available to
/// the invoking crate.
///
/// A direct dependency on the leaf crate wins, including a renamed dependency.
/// Otherwise, a direct `nebula-sdk` dependency is used through its hidden macro
/// support namespace. The canonical leaf path remains the fallback for
/// workspace-internal and other contexts that `proc-macro-crate` cannot
/// identify.
#[must_use]
pub fn resolve_generated_crate_paths(tokens: TokenStream) -> TokenStream {
    rewrite_stream(tokens)
}

fn rewrite_stream(tokens: TokenStream) -> TokenStream {
    let trees = tokens.into_iter().collect::<Vec<_>>();
    let mut output = TokenStream::new();
    let mut index = 0;

    while index < trees.len() {
        if let Some(canonical) = absolute_rewritable_path_at(&trees, index) {
            output.extend(resolve_path(canonical));
            index += 3;
            continue;
        }

        match trees[index].clone() {
            TokenTree::Group(group) => {
                let mut rewritten = Group::new(group.delimiter(), rewrite_stream(group.stream()));
                rewritten.set_span(group.span());
                output.extend([TokenTree::Group(rewritten)]);
            },
            tree => output.extend([tree]),
        }
        index += 1;
    }

    output
}

fn absolute_rewritable_path_at(trees: &[TokenTree], index: usize) -> Option<&str> {
    let [first, second, TokenTree::Ident(ident)] = trees.get(index..index.checked_add(3)?)? else {
        return None;
    };
    if !is_colon(first) || !is_colon(second) {
        return None;
    }

    let candidate = ident.to_string();
    NEBULA_CRATES
        .iter()
        .find_map(|(canonical, _, _)| (*canonical == candidate).then_some(*canonical))
        .or_else(|| {
            matches!(candidate.as_str(), "semver" | "serde_json").then_some(
                if candidate == "semver" {
                    "semver"
                } else {
                    "serde_json"
                },
            )
        })
}

fn is_colon(tree: &TokenTree) -> bool {
    matches!(tree, TokenTree::Punct(punct) if punct.as_char() == ':')
}

fn resolve_path(canonical: &str) -> TokenStream {
    if matches!(canonical, "semver" | "serde_json") {
        return resolve_external_path(canonical);
    }

    let Some((_, package, sdk_module)) = NEBULA_CRATES
        .iter()
        .find(|(candidate, _, _)| *candidate == canonical)
    else {
        return absolute_ident_path(canonical);
    };

    match crate_name(package) {
        Ok(FoundCrate::Name(name)) => absolute_ident_path(&name),
        Ok(FoundCrate::Itself) => absolute_ident_path(canonical),
        Err(_) => resolve_sdk_path(sdk_module).unwrap_or_else(|| absolute_ident_path(canonical)),
    }
}

fn resolve_external_path(package: &str) -> TokenStream {
    match crate_name(package) {
        Ok(FoundCrate::Name(name)) => absolute_ident_path(&name),
        Ok(FoundCrate::Itself) => absolute_ident_path(package),
        Err(_) => {
            let sdk_name = match crate_name("nebula-sdk") {
                Ok(FoundCrate::Name(name)) => name,
                Ok(FoundCrate::Itself) => "nebula_sdk".to_owned(),
                Err(_) => return absolute_ident_path(package),
            };
            let sdk = Ident::new(&sdk_name, proc_macro2::Span::call_site());
            let dependency = Ident::new(package, proc_macro2::Span::call_site());
            if package == "semver" {
                quote!(::#sdk::__private::#dependency)
            } else {
                quote!(::#sdk::#dependency)
            }
        },
    }
}

fn resolve_sdk_path(module: &str) -> Option<TokenStream> {
    let sdk = match crate_name("nebula-sdk") {
        Ok(FoundCrate::Name(name)) => Ident::new(&name, proc_macro2::Span::call_site()),
        Ok(FoundCrate::Itself) => Ident::new("nebula_sdk", proc_macro2::Span::call_site()),
        Err(_) => return None,
    };
    let module = Ident::new(module, proc_macro2::Span::call_site());
    Some(quote!(::#sdk::__private::#module))
}

fn absolute_ident_path(name: &str) -> TokenStream {
    let ident = Ident::new(name, proc_macro2::Span::call_site());
    quote!(::#ident)
}
