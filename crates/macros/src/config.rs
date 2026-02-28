use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, LitStr, Type, parse_macro_input};

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

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new(
                    struct_name.span(),
                    "Config derive requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Config derive can only be used on structs",
            ));
        }
    };

    let config_attrs = attrs::parse_attrs(&input.attrs, "config")?;
    let from_attr = config_attrs
        .get_string("source")
        .or_else(|| config_attrs.get_string("from"));
    let loaders = if let Some(list) = config_attrs
        .get_list("sources")
        .or_else(|| config_attrs.get_list("loaders"))
    {
        list
    } else if let Some(from) = from_attr {
        vec![from]
    } else {
        vec!["env".to_string()]
    };

    if loaders.is_empty() {
        return Err(syn::Error::new(
            struct_name.span(),
            "Config derive requires at least one loader in #[config(loaders = [...])]",
        ));
    }

    for loader in &loaders {
        match loader.as_str() {
            "env" | "dotenv" | "file" => {}
            other => {
                return Err(syn::Error::new(
                    struct_name.span(),
                    format!(
                        "unsupported config loader `{other}`; expected one of: env, dotenv, file"
                    ),
                ));
            }
        }
    }

    let env_prefix = config_attrs.get_string("prefix");
    let env_prefix_token = if let Some(prefix) = env_prefix.as_deref() {
        quote!(::std::option::Option::Some(#prefix))
    } else {
        quote!(::std::option::Option::None)
    };

    let separator = config_attrs
        .get_string("separator")
        .unwrap_or_else(|| "_".to_string());
    let profile_env = config_attrs
        .get_string("profile_var")
        .or_else(|| config_attrs.get_string("profile_env"))
        .unwrap_or_else(|| "APP_ENV".to_string());
    let profile_default = config_attrs.get_string("profile");
    let profile_default_token = if let Some(profile) = profile_default.as_deref() {
        quote!(::std::option::Option::Some(#profile))
    } else {
        quote!(::std::option::Option::None)
    };

    let file_default = config_attrs
        .get_string("path")
        .or_else(|| config_attrs.get_string("file"))
        .unwrap_or_else(|| "config.json".to_string());
    let dotenv_default = config_attrs
        .get_string("path")
        .or_else(|| config_attrs.get_string("file"))
        .unwrap_or_else(|| ".env".to_string());
    let file_default_lit = LitStr::new(&file_default, struct_name.span());
    let dotenv_default_lit = LitStr::new(&dotenv_default, struct_name.span());

    let loader_lits: Vec<LitStr> = loaders
        .iter()
        .map(|loader| LitStr::new(loader, struct_name.span()))
        .collect();

    let validator_attrs = attrs::parse_attrs(&input.attrs, "validator")?;
    let root_message = validator_attrs
        .get_string("message")
        .unwrap_or_else(|| "validation failed".to_string());

    let mut checks = Vec::new();
    let mut explicit_env_insertions = Vec::new();
    let mut field_default_insertions = Vec::new();

    for field in fields {
        let field_name = match &field.ident {
            Some(name) => name,
            None => continue,
        };
        let field_key = field_name.to_string();
        let is_option = is_option_type(&field.ty);
        let validate_attrs = attrs::parse_attrs(&field.attrs, "validate")?;
        let field_config_attrs = attrs::parse_attrs(&field.attrs, "config")?;

        let env_name = field_config_attrs
            .get_string("key")
            .or_else(|| field_config_attrs.get_string("name"))
            .or_else(|| field_config_attrs.get_string("env"))
            .unwrap_or_else(|| {
                let transformed = field_key.to_uppercase().replace('_', &separator);
                match &env_prefix {
                    Some(prefix) => format!("{prefix}{separator}{transformed}"),
                    None => transformed,
                }
            });

        if let Some(default_raw) = field_config_attrs.get_value("default") {
            let default_expr = value_token(default_raw);
            field_default_insertions.push(quote! {
                obj.insert(
                    #field_key.to_string(),
                    ::serde_json::to_value(#default_expr)
                        .map_err(|e| format!("failed to serialize default for field `{}`: {e}", #field_key))?,
                );
            });
        }

        explicit_env_insertions.push(quote! {
            if let Ok(raw) = ::std::env::var(#env_name) {
                map.insert(#field_key.to_string(), parse_env_value(&raw));
            }
        });

        if validate_attrs.has_flag("required") && is_option {
            checks.push(quote! {
                if input.#field_name.is_none() {
                    errors.add(
                        ::nebula_validator::foundation::ValidationError::required(#field_key)
                    );
                }
            });
        }

        if let Some(min_len) = parse_usize(&validate_attrs, "min_length")? {
            checks.push(generate_len_check(
                field_name, &field_key, is_option, min_len, true,
            ));
        }

        if let Some(max_len) = parse_usize(&validate_attrs, "max_length")? {
            checks.push(generate_len_check(
                field_name, &field_key, is_option, max_len, false,
            ));
        }

        if let Some(min_value) = parse_number_lit(&validate_attrs, "min")? {
            checks.push(generate_cmp_check(
                field_name, &field_key, is_option, min_value, true,
            ));
        }

        if let Some(max_value) = parse_number_lit(&validate_attrs, "max")? {
            checks.push(generate_cmp_check(
                field_name, &field_key, is_option, max_value, false,
            ));
        }

        let str_validators: &[(&str, TokenStream2)] = &[
            ("email", quote!(::nebula_validator::validators::email())),
            ("url", quote!(::nebula_validator::validators::url())),
            ("ipv4", quote!(::nebula_validator::validators::ipv4())),
            ("ipv6", quote!(::nebula_validator::validators::ipv6())),
            ("ip_addr", quote!(::nebula_validator::validators::ip_addr())),
            (
                "hostname",
                quote!(::nebula_validator::validators::hostname()),
            ),
            ("uuid", quote!(::nebula_validator::validators::uuid())),
            ("date", quote!(::nebula_validator::validators::date())),
            (
                "date_time",
                quote!(::nebula_validator::validators::date_time()),
            ),
            ("time", quote!(::nebula_validator::validators::time())),
        ];
        for (flag, expr) in str_validators {
            if validate_attrs.has_flag(flag) {
                checks.push(generate_str_validator_check(
                    field_name,
                    &field_key,
                    is_option,
                    expr.clone(),
                ));
            }
        }

        if let Some(pattern) = validate_attrs.get_string("regex") {
            let check = if is_option {
                quote! {
                    if let Some(ref value) = input.#field_name {
                        match ::nebula_validator::validators::matches_regex(#pattern) {
                            Ok(v) => {
                                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                                    errors.add(e.with_field(#field_key));
                                }
                            }
                            Err(_) => panic!(
                                concat!("invalid regex pattern in #[validate(regex = ...)] on field `", #field_key, "`")
                            ),
                        }
                    }
                }
            } else {
                quote! {
                    match ::nebula_validator::validators::matches_regex(#pattern) {
                        Ok(v) => {
                            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, input.#field_name.as_str()) {
                                errors.add(e.with_field(#field_key));
                            }
                        }
                        Err(_) => panic!(
                            concat!("invalid regex pattern in #[validate(regex = ...)] on field `", #field_key, "`")
                        ),
                    }
                }
            };
            checks.push(check);
        }
    }

    let expanded = quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            pub fn from_env() -> ::std::result::Result<Self, ::std::string::String>
            where
                Self: ::core::default::Default,
            {
                Self::from_env_with_prefix(None)
            }

            pub fn from_env_with_prefix(
                prefix_override: ::std::option::Option<&str>,
            ) -> ::std::result::Result<Self, ::std::string::String>
            where
                Self: ::core::default::Default,
            {
                fn parse_env_value(value: &str) -> ::serde_json::Value {
                    if value.is_empty() {
                        return ::serde_json::Value::String(::std::string::String::new());
                    }
                    if value.eq_ignore_ascii_case("true") {
                        return ::serde_json::Value::Bool(true);
                    }
                    if value.eq_ignore_ascii_case("false") {
                        return ::serde_json::Value::Bool(false);
                    }
                    if let Ok(int_val) = value.parse::<i64>() {
                        return ::serde_json::Value::Number(::serde_json::Number::from(int_val));
                    }
                    if let Ok(float_val) = value.parse::<f64>()
                        && let Some(num) = ::serde_json::Number::from_f64(float_val)
                    {
                        return ::serde_json::Value::Number(num);
                    }
                    if ((value.starts_with('{') && value.ends_with('}'))
                        || (value.starts_with('[') && value.ends_with(']')))
                        && let Ok(json_val) = ::serde_json::from_str(value)
                    {
                        return json_val;
                    }
                    if value.contains(',') && !value.starts_with('"') {
                        let items: ::std::vec::Vec<::serde_json::Value> = value
                            .split(',')
                            .map(|s| parse_env_value(s.trim()))
                            .collect();
                        return ::serde_json::Value::Array(items);
                    }
                    ::serde_json::Value::String(value.to_string())
                }

                fn collect_prefixed_env(
                    prefix: &str,
                    separator: &str,
                ) -> ::serde_json::Map<::std::string::String, ::serde_json::Value> {
                    let mut map = ::serde_json::Map::new();
                    let prefix_with_sep = format!("{prefix}{separator}");
                    for (key, value) in ::std::env::vars() {
                        if key.starts_with(&prefix_with_sep) {
                            let stripped = key
                                .trim_start_matches(&prefix_with_sep)
                                .to_lowercase()
                                .replace(separator, "_");
                            map.insert(stripped, parse_env_value(&value));
                        }
                    }
                    map
                }

                let mut value = ::serde_json::to_value(Self::default())
                    .map_err(|e| format!("failed to serialize default config: {e}"))?;
                let obj = value
                    .as_object_mut()
                    .ok_or_else(|| "Config derive requires a struct serialized as JSON object".to_string())?;

                #(#field_default_insertions)*

                let mut map = ::serde_json::Map::new();
                #(#explicit_env_insertions)*

                let effective_prefix = prefix_override.or(#env_prefix_token);
                if let Some(prefix) = effective_prefix {
                    for (k, v) in collect_prefixed_env(prefix, #separator) {
                        map.insert(k, v);
                    }
                }

                for (k, v) in map {
                    obj.insert(k, v);
                }

                let candidate: Self = ::serde_json::from_value(value)
                    .map_err(|e| format!("failed to deserialize env config: {e}"))?;
                ::nebula_validator::foundation::Validate::validate(&candidate, &candidate)
                    .map_err(|e| format!("validation failed: {e}"))?;
                Ok(candidate)
            }

            pub fn load() -> ::std::result::Result<Self, ::std::string::String>
            where
                Self: ::core::default::Default,
            {
                Self::load_with_profile(None)
            }

            pub fn load_with_profile(
                profile_override: ::std::option::Option<&str>,
            ) -> ::std::result::Result<Self, ::std::string::String>
            where
                Self: ::core::default::Default,
            {
                fn parse_env_value(value: &str) -> ::serde_json::Value {
                    if value.is_empty() {
                        return ::serde_json::Value::String(::std::string::String::new());
                    }
                    if value.eq_ignore_ascii_case("true") {
                        return ::serde_json::Value::Bool(true);
                    }
                    if value.eq_ignore_ascii_case("false") {
                        return ::serde_json::Value::Bool(false);
                    }
                    if let Ok(int_val) = value.parse::<i64>() {
                        return ::serde_json::Value::Number(::serde_json::Number::from(int_val));
                    }
                    if let Ok(float_val) = value.parse::<f64>()
                        && let Some(num) = ::serde_json::Number::from_f64(float_val)
                    {
                        return ::serde_json::Value::Number(num);
                    }
                    if ((value.starts_with('{') && value.ends_with('}'))
                        || (value.starts_with('[') && value.ends_with(']')))
                        && let Ok(json_val) = ::serde_json::from_str(value)
                    {
                        return json_val;
                    }
                    if value.contains(',') && !value.starts_with('"') {
                        let items: ::std::vec::Vec<::serde_json::Value> = value
                            .split(',')
                            .map(|s| parse_env_value(s.trim()))
                            .collect();
                        return ::serde_json::Value::Array(items);
                    }
                    ::serde_json::Value::String(value.to_string())
                }

                fn resolve_profile(
                    profile_override: ::std::option::Option<&str>,
                    profile_env: &str,
                    profile_default: ::std::option::Option<&str>,
                ) -> ::std::option::Option<::std::string::String> {
                    profile_override
                        .map(::std::string::ToString::to_string)
                        .or_else(|| profile_default.map(::std::string::ToString::to_string))
                        .or_else(|| ::std::env::var(profile_env).ok())
                        .filter(|v| !v.is_empty())
                }

                fn profile_suffix_path(path: &str, profile: &str) -> ::std::string::String {
                    let p = ::std::path::Path::new(path);
                    let mut out = p.to_path_buf();
                    let file_name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(path);

                    let profiled_name = if file_name == ".env" {
                        format!(".env.{profile}")
                    } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                        let stem = p
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(file_name);
                        format!("{stem}.{profile}.{ext}")
                    } else {
                        format!("{file_name}.{profile}")
                    };

                    out.set_file_name(profiled_name);
                    out.to_string_lossy().into_owned()
                }

                fn read_file_if_exists(path: &str) -> ::std::result::Result<::std::option::Option<::std::string::String>, ::std::string::String> {
                    match ::std::fs::read_to_string(path) {
                        Ok(content) => Ok(Some(content)),
                        Err(err) if err.kind() == ::std::io::ErrorKind::NotFound => Ok(None),
                        Err(err) => Err(format!("failed to read `{path}`: {err}")),
                    }
                }

                fn collect_prefixed_env(
                    prefix: &str,
                    separator: &str,
                ) -> ::serde_json::Map<::std::string::String, ::serde_json::Value> {
                    let mut map = ::serde_json::Map::new();
                    let prefix_with_sep = format!("{prefix}{separator}");
                    for (key, value) in ::std::env::vars() {
                        if key.starts_with(&prefix_with_sep) {
                            let stripped = key
                                .trim_start_matches(&prefix_with_sep)
                                .to_lowercase()
                                .replace(separator, "_");
                            map.insert(stripped, parse_env_value(&value));
                        }
                    }
                    map
                }

                fn parse_dotenv_content(
                    content: &str,
                    prefix: ::std::option::Option<&str>,
                    separator: &str,
                ) -> ::serde_json::Map<::std::string::String, ::serde_json::Value> {
                    let mut map = ::serde_json::Map::new();
                    for line in content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        let line = line.strip_prefix("export ").unwrap_or(line);
                        let Some((raw_key, raw_value)) = line.split_once('=') else {
                            continue;
                        };

                        let key = raw_key.trim();
                        let mut value = raw_value.trim().to_string();
                        if (value.starts_with('"') && value.ends_with('"'))
                            || (value.starts_with('\'') && value.ends_with('\''))
                        {
                            value = value[1..value.len() - 1].to_string();
                        }

                        let normalized_key = if let Some(prefix) = prefix {
                            let expected = format!("{prefix}{separator}");
                            if !key.starts_with(&expected) {
                                continue;
                            }
                            key.trim_start_matches(&expected)
                                .to_lowercase()
                                .replace(separator, "_")
                        } else {
                            key.to_lowercase().replace(separator, "_")
                        };

                        map.insert(normalized_key, parse_env_value(&value));
                    }
                    map
                }

                fn parse_config_file_content(
                    path: &str,
                    content: &str,
                    prefix: ::std::option::Option<&str>,
                    separator: &str,
                ) -> ::std::result::Result<::serde_json::Map<::std::string::String, ::serde_json::Value>, ::std::string::String> {
                    let ext = ::std::path::Path::new(path)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();

                    if ext == "env" || path.ends_with(".env") {
                        return Ok(parse_dotenv_content(content, prefix, separator));
                    }

                    if ext == "json" {
                        let value: ::serde_json::Value = ::serde_json::from_str(content)
                            .map_err(|e| format!("failed to parse JSON file `{path}`: {e}"))?;
                        let obj = value
                            .as_object()
                            .ok_or_else(|| format!("JSON config file `{path}` must contain an object at root"))?;
                        return Ok(obj.clone());
                    }

                    Err(format!(
                        "unsupported config file extension for `{path}`; supported: .json, .env"
                    ))
                }

                let mut value = ::serde_json::to_value(Self::default())
                    .map_err(|e| format!("failed to serialize default config: {e}"))?;
                let obj = value
                    .as_object_mut()
                    .ok_or_else(|| "Config derive requires a struct serialized as JSON object".to_string())?;

                #(#field_default_insertions)*

                let profile = resolve_profile(profile_override, #profile_env, #profile_default_token);
                let effective_prefix = #env_prefix_token;
                let loaders: &[&str] = &[#(#loader_lits),*];

                for loader in loaders {
                    match *loader {
                        "env" => {
                            let mut map = ::serde_json::Map::new();
                            #(#explicit_env_insertions)*
                            if let Some(prefix) = effective_prefix {
                                for (k, v) in collect_prefixed_env(prefix, #separator) {
                                    map.insert(k, v);
                                }
                            }
                            for (k, v) in map {
                                obj.insert(k, v);
                            }
                        }
                        "dotenv" => {
                            let base_path = #dotenv_default_lit;
                            let mut paths = vec![base_path.to_string()];
                            if let Some(profile) = profile.as_deref() {
                                paths.push(profile_suffix_path(base_path, profile));
                            }

                            for path in paths {
                                if let Some(content) = read_file_if_exists(&path)? {
                                    let parsed = parse_dotenv_content(&content, effective_prefix, #separator);
                                    for (k, v) in parsed {
                                        obj.insert(k, v);
                                    }
                                }
                            }
                        }
                        "file" => {
                            let base_path = #file_default_lit;
                            let mut paths = vec![base_path.to_string()];
                            if let Some(profile) = profile.as_deref() {
                                paths.push(profile_suffix_path(base_path, profile));
                            }

                            for path in paths {
                                if let Some(content) = read_file_if_exists(&path)? {
                                    let parsed = parse_config_file_content(&path, &content, effective_prefix, #separator)?;
                                    for (k, v) in parsed {
                                        obj.insert(k, v);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                let candidate: Self = ::serde_json::from_value(value)
                    .map_err(|e| format!("failed to deserialize loaded config: {e}"))?;
                ::nebula_validator::foundation::Validate::validate(&candidate, &candidate)
                    .map_err(|e| format!("validation failed: {e}"))?;
                Ok(candidate)
            }

            pub fn validate_fields(
                &self,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationErrors> {
                let input = self;
                let mut errors = ::nebula_validator::foundation::ValidationErrors::new();
                #(#checks)*

                if errors.has_errors() {
                    Err(errors)
                } else {
                    Ok(())
                }
            }
        }

        impl #impl_generics ::nebula_validator::foundation::Validate<#struct_name #ty_generics> for #struct_name #ty_generics #where_clause {
            fn validate(
                &self,
                input: &#struct_name #ty_generics,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationError> {
                let _ = self;
                input
                    .validate_fields()
                    .map_err(|errors| errors.into_single_error(#root_message))
            }
        }
    };

    Ok(expanded.into())
}

fn parse_usize(args: &attrs::AttrArgs, key: &str) -> syn::Result<Option<usize>> {
    let value = match args.get_value(key) {
        Some(value) => value,
        None => return Ok(None),
    };

    let parsed = match value {
        attrs::AttrValue::Lit(syn::Lit::Int(int)) => int
            .base10_parse::<usize>()
            .map_err(|_| diag::error_spanned(int, format!("`{key}` must be a positive integer")))?,
        _ => {
            return Err(diag::error_spanned(
                &value_token(value),
                format!("`{key}` must be an integer literal"),
            ));
        }
    };

    Ok(Some(parsed))
}

fn parse_number_lit(args: &attrs::AttrArgs, key: &str) -> syn::Result<Option<TokenStream2>> {
    let value = match args.get_value(key) {
        Some(value) => value,
        None => return Ok(None),
    };

    match value {
        attrs::AttrValue::Lit(syn::Lit::Int(int)) => Ok(Some(quote!(#int))),
        attrs::AttrValue::Lit(syn::Lit::Float(float)) => Ok(Some(quote!(#float))),
        _ => Err(diag::error_spanned(
            &value_token(value),
            format!("`{key}` must be an integer or float literal"),
        )),
    }
}

fn value_token(value: &attrs::AttrValue) -> TokenStream2 {
    match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Lit(lit) => quote!(#lit),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
    }
}

fn generate_len_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    bound: usize,
    is_min: bool,
) -> TokenStream2 {
    let error = if is_min {
        quote! {
            ::nebula_validator::foundation::ValidationError::min_length(
                #field_key,
                #bound,
                value.len(),
            )
        }
    } else {
        quote! {
            ::nebula_validator::foundation::ValidationError::max_length(
                #field_key,
                #bound,
                value.len(),
            )
        }
    };

    if is_option {
        if is_min {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value.len() < #bound {
                        errors.add(#error);
                    }
                }
            }
        } else {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value.len() > #bound {
                        errors.add(#error);
                    }
                }
            }
        }
    } else if is_min {
        quote! {
            let value = &input.#field_name;
            if value.len() < #bound {
                errors.add(#error);
            }
        }
    } else {
        quote! {
            let value = &input.#field_name;
            if value.len() > #bound {
                errors.add(#error);
            }
        }
    }
}

fn generate_cmp_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    bound: TokenStream2,
    is_min: bool,
) -> TokenStream2 {
    let error = if is_min {
        quote! {
            ::nebula_validator::foundation::ValidationError::new(
                "min",
                format!("{} must be >= {}", #field_key, #bound),
            )
            .with_field(#field_key)
        }
    } else {
        quote! {
            ::nebula_validator::foundation::ValidationError::new(
                "max",
                format!("{} must be <= {}", #field_key, #bound),
            )
            .with_field(#field_key)
        }
    };

    if is_option {
        if is_min {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value < &#bound {
                        errors.add(#error);
                    }
                }
            }
        } else {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value > &#bound {
                        errors.add(#error);
                    }
                }
            }
        }
    } else if is_min {
        quote! {
            let value = &input.#field_name;
            if value < &#bound {
                errors.add(#error);
            }
        }
    } else {
        quote! {
            let value = &input.#field_name;
            if value > &#bound {
                errors.add(#error);
            }
        }
    }
}

fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident == "Option";
    }
    false
}

fn generate_str_validator_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    validator_expr: TokenStream2,
) -> TokenStream2 {
    if is_option {
        quote! {
            if let Some(ref value) = input.#field_name {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                    errors.add(e.with_field(#field_key));
                }
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, input.#field_name.as_str()) {
                errors.add(e.with_field(#field_key));
            }
        }
    }
}
