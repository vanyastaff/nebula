use syn::parse::Parse;
use syn::spanned::Spanned;
use syn::{Attribute, Data, DeriveInput, Fields};

/// Collect doc comments (`/// ...`) into a single string.
pub fn doc_string(attrs: &[Attribute]) -> String {
    let mut out = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc")
            && let Ok(syn::Meta::NameValue(nv)) = attr.parse_args_with(<syn::Meta as Parse>::parse)
            && let syn::Expr::Lit(expr_lit) = &nv.value
            && let syn::Lit::Str(s) = &expr_lit.lit
        {
            let line = s.value().trim().to_string();
            if !line.is_empty() {
                out.push(line);
            }
        }
    }
    out.join("\n")
}

/// Ensure input is a struct and return its fields.
pub fn require_struct_fields(input: &DeriveInput) -> syn::Result<&Fields> {
    match &input.data {
        Data::Struct(s) => Ok(&s.fields),
        _ => Err(syn::Error::new(
            input.ident.span(),
            "This derive can only be used on structs",
        )),
    }
}

/// Return named fields if struct has them; otherwise error.
pub fn require_named_fields(input: &DeriveInput) -> syn::Result<&syn::FieldsNamed> {
    let fields = require_struct_fields(input)?;
    match fields {
        Fields::Named(n) => Ok(n),
        Fields::Unnamed(_) => Err(syn::Error::new(
            fields.span(),
            "This derive requires a struct with named fields (e.g. `struct X { ... }`)",
        )),
        Fields::Unit => Err(syn::Error::new(
            fields.span(),
            "This derive requires a non-unit struct with fields",
        )),
    }
}
