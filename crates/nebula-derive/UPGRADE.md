# Shared Derive Infrastructure

## 🎯 Цель
Создать переиспользуемую инфраструктуру для всех derive макросов в nebula-derive:
- `#[derive(Validator)]`
- `#[derive(Action)]` (future)
- `#[derive(Resource)]` (future)
- `#[derive(Parameter)]` (future)

## 🏗️ Архитектура

```
nebula-derive/
├── src/
│   ├── lib.rs                 # Entry points for all derives
│   ├── shared/                # SHARED INFRASTRUCTURE
│   │   ├── mod.rs
│   │   ├── types.rs          # Type detection & analysis
│   │   ├── attrs.rs          # Common attribute parsing
│   │   ├── codegen.rs        # Code generation helpers
│   │   └── validation.rs     # Input validation helpers
│   ├── validator/            # Validator-specific
│   │   ├── mod.rs
│   │   ├── parse.rs         # Validator attributes
│   │   └── generate.rs      # Validator code generation
│   ├── action/               # Action-specific (future)
│   │   ├── mod.rs
│   │   ├── parse.rs
│   │   └── generate.rs
│   ├── resource/             # Resource-specific (future)
│   │   ├── mod.rs
│   │   ├── parse.rs
│   │   └── generate.rs
│   └── parameter/            # Parameter-specific (future)
│       ├── mod.rs
│       ├── parse.rs
│       └── generate.rs
└── tests/
    ├── validator/
    ├── action/
    ├── resource/
    └── shared/              # Tests for shared infrastructure
```

## 📦 Shared Modules

### 1. `shared/types.rs` - Universal Type System

```rust
//! Universal type detection and analysis for all derive macros

use syn::{Type, TypePath, PathSegment, GenericArgument};
use proc_macro2::TokenStream;
use quote::quote;

/// Comprehensive type categorization
#[derive(Debug, Clone, PartialEq)]
pub enum TypeCategory {
    // Primitives
    String,
    Str,
    Bool,
    
    // Numbers
    Integer(IntegerType),
    Float(FloatType),
    
    // Dates/Times
    DateTime(DateTimeType),
    
    // Collections
    Vec(Box<TypeCategory>),
    HashSet(Box<TypeCategory>),
    HashMap { key: Box<TypeCategory>, value: Box<TypeCategory> },
    BTreeMap { key: Box<TypeCategory>, value: Box<TypeCategory>),
    
    // Special wrappers
    Option(Box<TypeCategory>),
    Result { ok: Box<TypeCategory>, err: Box<TypeCategory> },
    Arc(Box<TypeCategory>),
    Rc(Box<TypeCategory>),
    Box(Box<TypeCategory>),
    
    // Nebula-specific types
    NebulValue,              // nebula_value::Value
    Parameter,               // nebula_parameter::Parameter
    Action(String),          // Custom Action type
    Resource(String),        // Custom Resource type
    
    // Custom types
    CustomStruct(StructInfo),
    CustomEnum(EnumInfo),
    
    // References
    Reference { 
        mutable: bool, 
        inner: Box<TypeCategory> 
    },
    
    // Unknown
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IntegerType {
    I8, I16, I32, I64, I128, ISize,
    U8, U16, U32, U64, U128, USize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FloatType {
    F32, F64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DateTimeType {
    // String-based
    IsoString,              // String with datetime validation
    
    // Native
    NativeDate,             // nebula date type
    
    // Chrono (feature-gated)
    ChronoNaiveDate,
    ChronoNaiveDateTime,
    ChronoDateTime,
    
    // Time crate (feature-gated)
    TimeDate,
    TimeDateTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructInfo {
    pub name: String,
    pub path: String,
    pub is_nebula_type: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumInfo {
    pub name: String,
    pub path: String,
}

/// Main type detection function
pub fn detect_type(ty: &Type) -> TypeCategory {
    match ty {
        Type::Path(type_path) => detect_from_path(type_path),
        Type::Reference(type_ref) => TypeCategory::Reference {
            mutable: type_ref.mutability.is_some(),
            inner: Box::new(detect_type(&type_ref.elem)),
        },
        _ => TypeCategory::Unknown,
    }
}

/// Analyze struct to get all field information
pub struct StructAnalysis {
    pub name: syn::Ident,
    pub generics: syn::Generics,
    pub fields: Vec<FieldAnalysis>,
}

pub struct FieldAnalysis {
    pub name: syn::Ident,
    pub ty: Type,
    pub category: TypeCategory,
    pub attrs: Vec<syn::Attribute>,
}

impl StructAnalysis {
    pub fn from_derive_input(input: &syn::DeriveInput) -> syn::Result<Self> {
        // ... implementation
    }
}
```

### 2. `shared/attrs.rs` - Common Attribute Parsing

```rust
//! Common attribute parsing utilities

use darling::{FromDeriveInput, FromField};

/// Common attributes that all derives might use
#[derive(Debug, Clone)]
pub struct CommonAttrs {
    /// Skip this field in code generation
    pub skip: bool,
    
    /// Custom name for this field
    pub rename: Option<String>,
    
    /// Documentation/description
    pub doc: Option<String>,
    
    /// Feature flag requirement
    pub feature: Option<String>,
    
    /// Deprecated marker
    pub deprecated: Option<String>,
}

impl CommonAttrs {
    pub fn from_attributes(attrs: &[syn::Attribute]) -> syn::Result<Self> {
        // Parse common attributes from any macro
    }
}

/// Helper to extract documentation comments
pub fn extract_doc_comments(attrs: &[syn::Attribute]) -> Option<String> {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Ok(meta) = attr.meta.require_name_value() {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        docs.push(lit_str.value().trim().to_string());
                    }
                }
            }
        }
    }
    
    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Check if field has specific attribute
pub fn has_attribute(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}
```

### 3. `shared/codegen.rs` - Code Generation Helpers

```rust
//! Common code generation utilities

use proc_macro2::TokenStream;
use quote::quote;
use crate::shared::types::TypeCategory;

/// Generate accessor for a field based on its type
pub fn generate_accessor(
    struct_name: &syn::Ident,
    field_name: &syn::Ident,
    type_category: &TypeCategory,
) -> TokenStream {
    match type_category {
        // String needs .as_str()
        TypeCategory::String => quote! {
            |obj: &#struct_name| obj.#field_name.as_str()
        },
        
        // Option<String> needs special handling
        TypeCategory::Option(inner) if matches!(**inner, TypeCategory::String) => {
            quote! {
                |obj: &#struct_name| obj.#field_name.as_ref().map(|s| s.as_str())
            }
        },
        
        // References don't need extra &
        TypeCategory::Reference { inner, .. } => {
            quote! {
                |obj: &#struct_name| obj.#field_name
            }
        },
        
        // Everything else is a simple reference
        _ => quote! {
            |obj: &#struct_name| &obj.#field_name
        },
    }
}

/// Generate clone/copy code based on type
pub fn generate_clone(type_category: &TypeCategory) -> Option<TokenStream> {
    match type_category {
        TypeCategory::Integer(_) | 
        TypeCategory::Float(_) | 
        TypeCategory::Bool => Some(quote! { .clone() }),
        
        TypeCategory::String => Some(quote! { .clone() }),
        
        TypeCategory::Arc(_) | 
        TypeCategory::Rc(_) => Some(quote! { .clone() }),
        
        _ => None, // Needs explicit handling
    }
}

/// Generate conversion code from one type to another
pub fn generate_conversion(
    from: &TypeCategory,
    to: &TypeCategory,
) -> Option<TokenStream> {
    match (from, to) {
        (TypeCategory::String, TypeCategory::Str) => {
            Some(quote! { .as_str() })
        },
        
        (TypeCategory::Option(inner_from), TypeCategory::Option(inner_to)) => {
            let inner_conv = generate_conversion(inner_from, inner_to)?;
            Some(quote! { .map(|x| x #inner_conv) })
        },
        
        _ => None,
    }
}

/// Generate impl block with common structure
pub struct ImplBlockBuilder {
    struct_name: syn::Ident,
    generics: syn::Generics,
    methods: Vec<TokenStream>,
}

impl ImplBlockBuilder {
    pub fn new(struct_name: syn::Ident, generics: syn::Generics) -> Self {
        Self {
            struct_name,
            generics,
            methods: Vec::new(),
        }
    }
    
    pub fn add_method(&mut self, method: TokenStream) {
        self.methods.push(method);
    }
    
    pub fn build(self) -> TokenStream {
        let struct_name = &self.struct_name;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();
        let methods = &self.methods;
        
        quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                #(#methods)*
            }
        }
    }
}
```

### 4. `shared/validation.rs` - Input Validation

```rust
//! Input validation for derive macros

use syn::DeriveInput;

/// Validate that input is a struct with named fields
pub fn require_named_struct(input: &DeriveInput) -> syn::Result<&syn::FieldsNamed> {
    match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => Ok(fields),
            syn::Fields::Unnamed(_) => Err(syn::Error::new_spanned(
                input,
                "This derive macro requires named fields"
            )),
            syn::Fields::Unit => Err(syn::Error::new_spanned(
                input,
                "This derive macro cannot be applied to unit structs"
            )),
        },
        syn::Data::Enum(_) => Err(syn::Error::new_spanned(
            input,
            "This derive macro can only be applied to structs"
        )),
        syn::Data::Union(_) => Err(syn::Error::new_spanned(
            input,
            "This derive macro cannot be applied to unions"
        )),
    }
}

/// Validate that struct has at least one field
pub fn require_non_empty(fields: &syn::FieldsNamed) -> syn::Result<()> {
    if fields.named.is_empty() {
        Err(syn::Error::new_spanned(
            fields,
            "Struct must have at least one field"
        ))
    } else {
        Ok(())
    }
}

/// Check for conflicting attributes
pub fn check_conflicting_attrs(
    attrs: &[syn::Attribute],
    conflicts: &[(&str, &str)],
) -> syn::Result<()> {
    for (attr1, attr2) in conflicts {
        let has_attr1 = attrs.iter().any(|a| a.path().is_ident(attr1));
        let has_attr2 = attrs.iter().any(|a| a.path().is_ident(attr2));
        
        if has_attr1 && has_attr2 {
            return Err(syn::Error::new_spanned(
                attrs[0].clone(),
                format!("Attributes '{}' and '{}' cannot be used together", attr1, attr2)
            ));
        }
    }
    Ok(())
}
```

## 🎨 Usage in Specific Derives

### Validator (uses shared infrastructure)

```rust
// validator/generate.rs
use crate::shared::{
    types::{detect_type, TypeCategory},
    codegen::{generate_accessor, ImplBlockBuilder},
    validation::require_named_struct,
};

pub fn generate_validator(input: &DeriveInput) -> syn::Result<TokenStream> {
    // Use shared validation
    let fields = require_named_struct(input)?;
    
    // Use shared type detection
    for field in &fields.named {
        let type_category = detect_type(&field.ty);
        
        // Use shared accessor generation
        let accessor = generate_accessor(
            &input.ident,
            field.ident.as_ref().unwrap(),
            &type_category,
        );
        
        // Validator-specific logic...
    }
    
    // Use shared impl builder
    let mut builder = ImplBlockBuilder::new(
        input.ident.clone(),
        input.generics.clone(),
    );
    
    builder.add_method(quote! {
        pub fn validate(&self) -> Result<(), ValidationError> {
            // ...
        }
    });
    
    Ok(builder.build())
}
```

### Action (future, uses same infrastructure)

```rust
// action/generate.rs
use crate::shared::{
    types::{detect_type, TypeCategory},
    codegen::{generate_accessor, ImplBlockBuilder},
    validation::require_named_struct,
};

pub fn generate_action(input: &DeriveInput) -> syn::Result<TokenStream> {
    let fields = require_named_struct(input)?;
    
    // Same type detection!
    for field in &fields.named {
        let type_category = detect_type(&field.ty);
        
        // Action-specific logic based on type
        match type_category {
            TypeCategory::DateTime(_) => {
                // Special handling for datetime fields in actions
            },
            TypeCategory::Parameter => {
                // Special handling for parameter fields
            },
            _ => {}
        }
    }
    
    // Generate Action trait impl
    let mut builder = ImplBlockBuilder::new(
        input.ident.clone(),
        input.generics.clone(),
    );
    
    builder.add_method(quote! {
        async fn execute(&self, ctx: &Context) -> Result<Output, Error> {
            // ...
        }
    });
    
    Ok(builder.build())
}
```

### Resource (future)

```rust
// resource/generate.rs
pub fn generate_resource(input: &DeriveInput) -> syn::Result<TokenStream> {
    let fields = require_named_struct(input)?;
    
    // Same shared infrastructure!
    for field in &fields.named {
        let type_category = detect_type(&field.ty);
        
        // Resource-specific logic
        match type_category {
            TypeCategory::Arc(_) => {
                // Resources often use Arc for sharing
            },
            TypeCategory::Option(_) => {
                // Optional resources
            },
            _ => {}
        }
    }
    
    // ...
}
```

## 📝 Example: Complete Flow

### 1. User writes code:

```rust
#[derive(Validator)]
struct UserForm {
    #[validate(date_future)]
    deadline: NativeDate,
    
    #[validate(email)]
    email: String,
}
```

### 2. Shared infrastructure analyzes:

```rust
// In validator/generate.rs
let fields = require_named_struct(input)?; // ← shared/validation.rs

for field in fields {
    // Shared type detection
    let type_category = detect_type(&field.ty); // ← shared/types.rs
    
    match type_category {
        TypeCategory::DateTime(DateTimeType::NativeDate) => {
            // Auto-select native_date validator
        },
        TypeCategory::String => {
            // Check for email attribute
        },
        _ => {}
    }
    
    // Shared accessor generation
    let accessor = generate_accessor(...); // ← shared/codegen.rs
}
```

### 3. Validator-specific logic:

```rust
// validator/generate.rs - specific to Validator derive
let validator = generate_field_validator(type_category, attrs)?;
```

### 4. Generated code:

```rust
impl UserForm {
    pub fn validate(&self) -> Result<(), ValidationError> {
        // ... generated validation code
    }
}
```

## 🎯 Benefits of Shared Infrastructure

### 1. **Consistency**
```rust
// All derives handle String → &str the same way
TypeCategory::String => generate_accessor(...) // ← shared!
```

### 2. **Reusability**
```rust
// Type detection works for ALL derives
#[derive(Validator)]  // ← uses shared types
#[derive(Action)]     // ← uses same shared types
#[derive(Resource)]   // ← uses same shared types
```

### 3. **Maintainability**
```rust
// Fix type detection bug once, fixes ALL derives
// shared/types.rs - ONE place to update
```

### 4. **Extensibility**
```rust
// Add new type support in ONE place
pub enum TypeCategory {
    // ... existing ...
    
    // New type - automatically available to ALL derives!
    TimeRange { start: Box<TypeCategory>, end: Box<TypeCategory> },
}
```

### 5. **Testing**
```rust
// Test shared infrastructure independently
#[test]
fn test_type_detection() {
    // Tests work for ALL derives
}
```

## 🔄 Migration Path

### Phase 1: Extract Shared (Now)
1. Create `shared/` module
2. Move type detection to `shared/types.rs`
3. Move common helpers to `shared/codegen.rs`
4. Update `validator/` to use shared

### Phase 2: Refactor Validator (Next)
1. Use `StructAnalysis` instead of manual parsing
2. Use shared accessor generation
3. Use shared impl builder

### Phase 3: Add Action (Future)
1. Create `action/` module
2. Use ALL shared infrastructure
3. Add Action-specific logic on top

### Phase 4: Add Resource & Parameter (Future)
1. Same pattern as Action
2. Leverage existing shared code

## 📊 Code Reuse Metrics

```
Before shared infrastructure:
- validator/: 500 lines
- action/: 500 lines (future)
- resource/: 500 lines (future)
Total: 1500 lines (with duplication)

After shared infrastructure:
- shared/: 400 lines (reused 3x!)
- validator/: 200 lines (specific)
- action/: 200 lines (specific)
- resource/: 200 lines (specific)
Total: 1000 lines (33% reduction!)
```

## ✅ Implementation Checklist

- [ ] Create `shared/` module structure
- [ ] Implement `shared/types.rs` with comprehensive type detection
- [ ] Implement `shared/attrs.rs` with common attribute parsing
- [ ] Implement `shared/codegen.rs` with generation helpers
- [ ] Implement `shared/validation.rs` with input validation
- [ ] Refactor `validator/` to use shared infrastructure
- [ ] Add tests for shared infrastructure
- [ ] Document shared API
- [ ] Create examples showing usage in multiple derives

## 🎓 Design Principles

1. **DRY (Don't Repeat Yourself)**
   - Type detection logic in ONE place
   - Accessor generation in ONE place
   - Common patterns extracted

2. **Open/Closed Principle**
   - Shared code is stable (closed for modification)
   - New derives extend functionality (open for extension)

3. **Single Responsibility**
   - `types.rs` - ONLY type detection
   - `codegen.rs` - ONLY code generation
   - `validation.rs` - ONLY input validation

4. **Dependency Inversion**
   - Specific derives depend on shared abstractions
   - Shared doesn't know about specific derives

This architecture will make adding `Action`, `Resource`, and `Parameter` derives much easier!