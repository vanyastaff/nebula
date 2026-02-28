# Archived From "docs/archive/crates-architecture.md"

## 2. nebula-derive

**Purpose**: Procedural macros for generating boilerplate code.

```rust
// nebula-derive/src/lib.rs
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(Action, attributes(action))]
pub fn derive_action(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    // Implementation for Action derive
}

#[proc_macro_derive(Parameters, attributes(param, validate, display))]
pub fn derive_parameters(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    
    // Generate parameter collection
    let gen = quote! {
        impl Parameters for #name {
            fn parameter_collection() -> ParameterCollection {
                // Generated code
            }
            
            fn from_values(values: HashMap<Key, ParameterValue>) -> Result<Self, Error> {
                // Generated code
            }
        }
    };
    
    gen.into()
}

// Example attributes handling
#[proc_macro_attribute]
pub fn node(args: TokenStream, input: TokenStream) -> TokenStream {
    // Parse node metadata from attributes
}
```

