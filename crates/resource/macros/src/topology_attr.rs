//! Parser for the `#[topology(Kind)]` container attribute on `#[derive(Resource)]`.
//!
//! When `#[topology(Resident)]` or `#[topology(Pooled)]` is present on the
//! struct, `#[derive(Resource)]` emits a `<Name>Factory` newtype that
//! implements `nebula_resource::ResourceFactory` via an erased
//! `KindActivator`. Absence of the attribute is legal — no factory is emitted
//! (backward-compatible for resource types that do not yet need factory
//! registration).
//!
//! ## Accepted forms
//!
//! - `#[topology(Resident)]` — one shared cloned instance.
//! - `#[topology(Pooled)]` — N interchangeable instances with checkout/recycle.
//!
//! Only one `#[topology]` attribute per struct is accepted; a second one is a
//! compile error.

use syn::{Attribute, Ident};

/// The topology kind chosen via `#[topology(...)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopologyKind {
    Resident,
    Pooled,
}

impl TopologyKind {
    /// Human-readable name for error messages.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Resident => "Resident",
            Self::Pooled => "Pooled",
        }
    }
}

/// Scan the struct's container attributes for an optional `#[topology(...)]`.
///
/// Returns `None` when the attribute is absent (no factory emitted).
/// Returns `Err` on any parse / duplicate / unknown-kind error.
pub(crate) fn parse_topology_attr(attrs: &[Attribute]) -> syn::Result<Option<TopologyKind>> {
    let mut found: Option<(TopologyKind, &Attribute)> = None;

    for attr in attrs {
        if !attr.path().is_ident("topology") {
            continue;
        }

        if let Some((_, prev)) = found {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate #[topology(...)] attribute — only one is accepted per struct",
            )
            .combine_with(syn::Error::new_spanned(prev, "first #[topology] here")));
        }

        let kind = parse_topology_meta(attr)?;
        found = Some((kind, attr));
    }

    Ok(found.map(|(kind, _)| kind))
}

fn parse_topology_meta(attr: &Attribute) -> syn::Result<TopologyKind> {
    let mut kind_ident: Option<Ident> = None;

    attr.parse_nested_meta(|meta| {
        let ident = meta.path.get_ident().ok_or_else(|| {
            syn::Error::new_spanned(&meta.path, "expected a topology kind identifier")
        })?;

        if kind_ident.is_some() {
            return Err(syn::Error::new_spanned(
                ident,
                "only one topology kind is accepted per #[topology(...)] attribute",
            ));
        }

        match ident.to_string().as_str() {
            "Resident" | "Pooled" => {
                kind_ident = Some(ident.clone());
                Ok(())
            },
            other => Err(syn::Error::new_spanned(
                ident,
                format!("unknown topology kind `{other}` — accepted values: Resident, Pooled"),
            )),
        }
    })?;

    let ident = kind_ident.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            "empty #[topology(...)] — specify a kind: #[topology(Resident)] or #[topology(Pooled)]",
        )
    })?;

    match ident.to_string().as_str() {
        "Resident" => Ok(TopologyKind::Resident),
        "Pooled" => Ok(TopologyKind::Pooled),
        _ => unreachable!("ident was validated in the nested-meta loop"),
    }
}

/// Helper: combine two `syn::Error`s without allocating a separate call chain.
trait CombineWith: Sized {
    fn combine_with(self, other: Self) -> Self;
}

impl CombineWith for syn::Error {
    fn combine_with(mut self, other: Self) -> Self {
        self.combine(other);
        self
    }
}
