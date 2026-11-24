//! Parsed `#[action(...)]` attributes for `#[derive(Action)]`.

use nebula_macro_support::{
    attrs,
    component::{COMPONENT_KEYS, ComponentAttrs},
};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Ident, Result, Type};

/// Parsed action container attributes.
#[derive(Debug, Clone)]
pub(crate) struct ActionAttrs {
    /// Key, name, description, version and config, shared with every
    /// component derive.
    pub component: ComponentAttrs,
    /// Required `Self::Output` type.
    pub output: Type,
    /// The author explicitly attests that the action performs no external
    /// business effects (`#[action(read_only)]`).
    pub read_only: bool,
}

/// The pre-0.22.0 spelling of the `read_only` flag, refused with a hint.
const RENAMED_READ_ONLY: &str = "no_external_effects";

/// The pre-0.41.0 name of the `config` attribute, refused with a hint.
const RENAMED_CONFIG: &str = "input";

impl ActionAttrs {
    /// Parse from `#[action(...)]` attribute args.
    pub(crate) fn parse(
        attr_args: &attrs::AttrArgs,
        struct_name: &Ident,
        doc_attributes: &[Attribute],
    ) -> Result<Self> {
        const ACTION_KEYS: [&str; 2] = ["output", "read_only"];
        for item in &attr_args.items {
            let key = match item {
                attrs::AttrItem::KeyValue { key, .. }
                | attrs::AttrItem::Flag(key)
                | attrs::AttrItem::List { key, .. } => key,
            };
            if key == RENAMED_READ_ONLY {
                return Err(syn::Error::new_spanned(
                    key,
                    "`no_external_effects` was renamed to `read_only`; \
                     write `#[action(read_only)]`",
                ));
            }
            if key == RENAMED_CONFIG {
                return Err(syn::Error::new_spanned(
                    key,
                    "`input` was renamed to `config`: an action's form data is its \
                     `Component::Config`; write `#[action(config = SomeType)]`",
                ));
            }
            let allowed = COMPONENT_KEYS.iter().chain(ACTION_KEYS.iter());
            if !allowed.clone().any(|allowed| key == allowed) {
                return Err(syn::Error::new_spanned(
                    key,
                    format!(
                        "unknown attribute `{key}` in #[action(...)] \
                         — allowed keys: {}",
                        allowed.copied().collect::<Vec<_>>().join(", "),
                    ),
                ));
            }
            if key == "read_only" && !matches!(item, attrs::AttrItem::Flag(_)) {
                return Err(syn::Error::new_spanned(
                    key,
                    "`read_only` is a flag; write it without `= ...`",
                ));
            }
        }

        let component = ComponentAttrs::parse(attr_args, struct_name, "action", doc_attributes)?;
        let output = attr_args.get_type("output")?.ok_or_else(|| {
            syn::Error::new_spanned(
                struct_name,
                "missing required attribute `output = SomeType` \
                 — the action's `Action::Output` must be specified",
            )
        })?;
        let read_only = attr_args.has_flag("read_only");

        Ok(Self {
            component,
            output,
            read_only,
        })
    }

    /// The `const KEY` item and a definition-site check of its literal.
    pub(crate) fn key_items(&self) -> (TokenStream2, TokenStream2) {
        let key = &self.component.key;
        (
            quote! { const KEY: &'static str = #key; },
            quote! {
                const _: () = ::core::assert!(
                    ::nebula_core::ActionKey::is_valid_key_const(#key),
                    "invalid action key literal"
                );
            },
        )
    }

    /// The `const EFFECT_CONTRACT` item for `read_only`, if set.
    pub(crate) fn effect_contract_item(&self) -> Option<TokenStream2> {
        self.read_only.then(|| {
            quote! {
                const EFFECT_CONTRACT: ::nebula_action::ActionEffectContract =
                    ::nebula_action::ActionEffectContract::ReadOnly;
            }
        })
    }
}
