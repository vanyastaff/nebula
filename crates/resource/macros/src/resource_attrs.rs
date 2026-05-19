//! Parsed `#[resource(...)]` attributes for the Phase 4 / slot model
//! `#[derive(Resource)]`.

use nebula_macro_support::{attrs, diag};
use syn::{Ident, Result, Type};

/// Validated resource topology.
///
/// Parsed once in [`ResourceAttrs::parse`] from the accepted
/// `pool` / `resident` / `bounded` strings (the former
/// `service` / `transport` / `exclusive` topologies are folded into
/// `bounded`, selected by the resource's `Bounded::Cap` typestate).
/// Holding the validated enum instead of the raw string lets
/// [`ResourceAttrs::topology_ident`] be an exhaustive match with no
/// panic path — a panicking proc macro is a poor failure mode.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Topology {
    Pool,
    Resident,
    Bounded,
}

/// Parsed resource container attributes.
#[derive(Debug, Clone)]
pub(crate) struct ResourceAttrs {
    /// Unique resource key (e.g. `"postgres"`).
    pub key: String,
    /// Validated topology, parsed from the `topology = "..."` attribute.
    pub topology: Topology,
    /// Required `Self::Config` type.
    pub config: Type,
    /// Optional `Self::Runtime` type — defaults to `()`.
    pub runtime: Type,
    /// Optional `Self::Lease` type — defaults to `Self::Runtime`.
    pub lease: Type,
    /// Optional `Self::Error` type — defaults to `nebula_resource::Error`.
    pub error: Type,
}

impl ResourceAttrs {
    /// Parse from `#[resource(...)]` attribute args.
    pub(crate) fn parse(attr_args: &attrs::AttrArgs, struct_name: &Ident) -> Result<Self> {
        const ALLOWED: &[&str] = &["key", "topology", "config", "runtime", "lease", "error"];
        for item in &attr_args.items {
            let key = match item {
                attrs::AttrItem::KeyValue { key, .. }
                | attrs::AttrItem::Flag(key)
                | attrs::AttrItem::List { key, .. } => key,
            };
            if !ALLOWED.iter().any(|allowed| key == allowed) {
                return Err(syn::Error::new_spanned(
                    key,
                    format!(
                        "unknown attribute `{key}` in #[resource(...)] \
                         — allowed keys: {}",
                        ALLOWED.join(", "),
                    ),
                ));
            }
        }

        let key = attr_args.require_string("key", struct_name)?;
        let topology = attr_args.get_string("topology").ok_or_else(|| {
            diag::error_spanned(
                struct_name,
                "missing required attribute `topology = \"pool|resident|bounded\"`",
            )
        })?;
        // Validate + classify the topology value. The former
        // `service` / `transport` / `exclusive` topologies are folded
        // into `bounded`.
        let topology = match topology.as_str() {
            "pool" => Topology::Pool,
            "resident" => Topology::Resident,
            "bounded" => Topology::Bounded,
            other => {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    format!(
                        "invalid `topology = \"{other}\"` — \
                         must be one of: pool, resident, bounded"
                    ),
                ));
            },
        };

        let config = attr_args.get_type("config")?.ok_or_else(|| {
            diag::error_spanned(
                struct_name,
                "missing required attribute `config = SomeType` \
                 — Phase 4 requires Self::Config to be specified",
            )
        })?;

        let runtime = attr_args
            .get_type("runtime")?
            .unwrap_or_else(|| syn::parse_quote!(()));
        let lease = attr_args
            .get_type("lease")?
            .unwrap_or_else(|| runtime.clone());
        let error = attr_args
            .get_type("error")?
            .unwrap_or_else(|| syn::parse_quote!(::nebula_resource::Error));

        Ok(Self {
            key,
            topology,
            config,
            runtime,
            lease,
            error,
        })
    }

    /// Returns the topology variant identifier for the corresponding `TopologyTag`.
    pub(crate) fn topology_ident(&self) -> Ident {
        let variant = match self.topology {
            Topology::Pool => "Pool",
            Topology::Resident => "Resident",
            Topology::Bounded => "Bounded",
        };
        Ident::new(variant, proc_macro2::Span::call_site())
    }
}
