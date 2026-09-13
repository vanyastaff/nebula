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
    let uses_sdk_resource_contribution = resource_factory_path_is_present(&tokens)
        && crate_name("nebula-resource").is_err()
        && crate_name("nebula-sdk").is_ok();
    rewrite_stream(tokens, uses_sdk_resource_contribution)
}

fn rewrite_stream(tokens: TokenStream, uses_sdk_resource_contribution: bool) -> TokenStream {
    let trees = tokens.into_iter().collect::<Vec<_>>();
    let mut output = TokenStream::new();
    let mut index = 0;

    while index < trees.len() {
        if uses_sdk_resource_contribution
            && let Some((replacement, consumed)) = sdk_resource_contribution_path_at(&trees, index)
        {
            output.extend(replacement);
            index += consumed;
            continue;
        }

        if let Some(canonical) = absolute_rewritable_path_at(&trees, index) {
            output.extend(resolve_path(canonical));
            index += 3;
            continue;
        }

        match trees[index].clone() {
            TokenTree::Group(group) => {
                let mut rewritten = Group::new(
                    group.delimiter(),
                    rewrite_stream(group.stream(), uses_sdk_resource_contribution),
                );
                rewritten.set_span(group.span());
                output.extend([TokenTree::Group(rewritten)]);
            },
            TokenTree::Ident(ident)
                if uses_sdk_resource_contribution && ident == "KindActivator" =>
            {
                output.extend([TokenTree::Ident(Ident::new(
                    "ResourceContributionBridge",
                    ident.span(),
                ))]);
            },
            tree => output.extend([tree]),
        }
        index += 1;
    }

    output
}

fn resource_factory_path_is_present(tokens: &TokenStream) -> bool {
    let trees = tokens.clone().into_iter().collect::<Vec<_>>();
    let mut index = 0;
    while index < trees.len() {
        if resource_contribution_path_at(&trees, index).is_some() {
            return true;
        }
        if let TokenTree::Group(group) = &trees[index]
            && resource_factory_path_is_present(&group.stream())
        {
            return true;
        }
        index += 1;
    }
    false
}

fn sdk_resource_contribution_path_at(
    trees: &[TokenTree],
    index: usize,
) -> Option<(TokenStream, usize)> {
    let (path, consumed) = resource_contribution_path_at(trees, index)?;
    let resource_path = resolve_sdk_path("resource")?;
    match path {
        ResourceContributionPath::Contribution => Some((
            quote!(#resource_path::contribution::ResourceContribution),
            consumed,
        )),
        ResourceContributionPath::Bridge => Some((
            quote!(#resource_path::contribution::ResourceContributionBridge),
            consumed,
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceContributionPath {
    Contribution,
    Bridge,
}

fn resource_contribution_path_at(
    trees: &[TokenTree],
    index: usize,
) -> Option<(ResourceContributionPath, usize)> {
    if !absolute_path_starts_with(trees, index, "nebula_resource") {
        return None;
    }
    if path_ident_at(trees, index + 3, "ResourceFactory") {
        return Some((ResourceContributionPath::Contribution, 6));
    }
    if path_ident_at(trees, index + 3, "factory")
        && path_ident_at(trees, index + 6, "KindActivator")
    {
        return Some((ResourceContributionPath::Bridge, 9));
    }
    None
}

fn absolute_path_starts_with(trees: &[TokenTree], index: usize, crate_name: &str) -> bool {
    is_colon_at(trees, index)
        && is_colon_at(trees, index + 1)
        && matches!(trees.get(index + 2), Some(TokenTree::Ident(ident)) if ident == crate_name)
}

fn path_ident_at(trees: &[TokenTree], colon_index: usize, expected: &str) -> bool {
    is_colon_at(trees, colon_index)
        && is_colon_at(trees, colon_index + 1)
        && matches!(trees.get(colon_index + 2), Some(TokenTree::Ident(ident)) if ident == expected)
}

fn is_colon_at(trees: &[TokenTree], index: usize) -> bool {
    trees.get(index).is_some_and(is_colon)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_resource_factory_paths_inside_generated_items() {
        let generated = quote! {
            pub struct ExampleFactory {
                inner: ::std::sync::Arc<dyn ::nebula_resource::ResourceFactory>,
            }
        };

        assert!(resource_factory_path_is_present(&generated));
    }

    #[test]
    fn identifies_typed_resource_contribution_bridge_path() {
        let generated = quote!(::nebula_resource::factory::KindActivator::<Example, _, _>);
        let trees = generated.into_iter().collect::<Vec<_>>();

        assert_eq!(
            resource_contribution_path_at(&trees, 0),
            Some((ResourceContributionPath::Bridge, 9))
        );
    }
}
