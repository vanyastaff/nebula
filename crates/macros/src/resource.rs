use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::support::{attrs, diag};

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts,
        Err(e) => diag::to_compile_error(e),
    }
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let resource_attrs = attrs::parse_attrs(&input.attrs, "resource")?;
    let id = resource_attrs.require_string("id", struct_name)?;

    let config_type = resource_attrs.get_type("config")?.ok_or_else(|| {
        diag::error_spanned(struct_name, "missing required attribute `config = Type`")
    })?;

    let instance_type = resource_attrs
        .get_type("instance")?
        .unwrap_or_else(|| syn::parse_str("Self").expect("valid Self type"));

    match &input.data {
        Data::Struct(_) => {}
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Resource derive can only be used on structs",
            ));
        }
    };

    let expanded = quote! {
        impl #impl_generics ::nebula_resource::Resource for #struct_name #ty_generics #where_clause {
            type Config = #config_type;
            type Instance = #instance_type;

            fn id(&self) -> &str {
                #id
            }

            fn create(
                &self,
                _config: &Self::Config,
                _ctx: &::nebula_resource::context::Context,
            ) -> impl ::std::future::Future<Output = ::nebula_resource::error::Result<Self::Instance>> + Send {
                async move {
                    ::std::todo!(
                        "implement `create` for resource `{}`",
                        stringify!(#struct_name)
                    )
                }
            }

            fn is_valid(
                &self,
                _instance: &Self::Instance,
            ) -> impl ::std::future::Future<Output = ::nebula_resource::error::Result<bool>> + Send {
                async move { Ok(true) }
            }

            fn recycle(
                &self,
                _instance: &mut Self::Instance,
            ) -> impl ::std::future::Future<Output = ::nebula_resource::error::Result<()>> + Send {
                async move { Ok(()) }
            }

            fn cleanup(
                &self,
                _instance: Self::Instance,
            ) -> impl ::std::future::Future<Output = ::nebula_resource::error::Result<()>> + Send {
                async move {
                    drop(_instance);
                    Ok(())
                }
            }
        }
    };

    Ok(expanded.into())
}
